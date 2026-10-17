// SPDX-License-Identifier: MIT OR Apache-2.0
//! Audio observations and conservative, reviewable ebook-edition suggestions.
//! An edition association never translates audio time into an ebook location.
use crate::{Error, Result, model::*};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudioMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    /// Explicit ebook identifiers reported by the source, with their namespaces.
    /// The audiobook ASIN is not implicitly a Kindle ASIN or an ebook ISBN.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ebook_identifiers: Vec<SourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}
impl AudioMetadata {
    pub fn validate(&self) -> Result<()> {
        fn text(value: &str, limit: usize) -> bool {
            !value.trim().is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
        }
        if self.title.as_ref().is_some_and(|s| !text(s, 2000))
            || self.authors.len() > 100
            || self.authors.iter().any(|s| !text(s, 500))
            || self.ebook_identifiers.len() > 100
            || self
                .ebook_identifiers
                .iter()
                .any(|s| !text(&s.provider, 200) || !text(&s.id, 2000))
            || self.language.as_ref().is_some_and(|s| !text(s, 100))
        {
            return Err(Error::Invalid("Invalid source audiobook metadata".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudioObservation {
    pub asin: String,
    pub position_milliseconds: u64,
    pub source_updated_at: u64,
    pub fetched_at: u64,
    pub duration_milliseconds: Option<u64>,
    /// Legacy opaque identifier retained without guessing its namespace.
    pub edition_identifier: Option<String>,
    pub alignment: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<AudioMetadata>,
}
impl AudioObservation {
    pub fn validate(&self, expected_asins: &[String], now: u64) -> Result<()> {
        if self.asin.len() != 10
            || !self.asin.bytes().all(|b| b.is_ascii_alphanumeric())
            || !expected_asins.contains(&self.asin)
            || self.source_updated_at == 0
            || self.source_updated_at > self.fetched_at.saturating_add(300)
            || self.fetched_at > now.saturating_add(300)
            || self
                .duration_milliseconds
                .is_some_and(|total| self.position_milliseconds > total)
            || self.alignment != "unmatched"
            || self
                .edition_identifier
                .as_ref()
                .is_some_and(|s| s.len() > 2000)
        {
            return Err(Error::Invalid(
                "Unqualified Audible position identity, timestamp, duration or alignment".into(),
            ));
        }
        if let Some(metadata) = &self.metadata {
            metadata.validate()?;
        }
        Ok(())
    }
    pub fn stale(&self, now: u64) -> bool {
        now.saturating_sub(self.source_updated_at) > 900
    }
    pub fn attach(&self, pass_id: &str) -> ProgressObservation {
        ProgressObservation {
            provider_connection_id: None,
            id: crate::digest(
                format!(
                    "audible:{pass_id}:{}:{}:{}",
                    self.asin, self.source_updated_at, self.position_milliseconds
                )
                .as_bytes(),
            ),
            pass_id: pass_id.into(),
            location: Location::Audio {
                provider: "audible".into(),
                source_id: self.asin.clone(),
                milliseconds: self.position_milliseconds,
                chapter: None,
            },
            observed_at: self.source_updated_at,
            received_at: self.fetched_at,
            alignment: Alignment::Unmatched,
            audio_metadata: self.metadata.clone(),
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum MatchConfidence {
    Identifier,
    TitleAndAuthor,
    TitleOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudioEditionCandidate {
    pub book_id: String,
    pub edition_id: String,
    pub edition_label: String,
    pub title: String,
    pub authors: Vec<String>,
    pub confidence: MatchConfidence,
    pub reasons: Vec<String>,
    pub warnings: Vec<String>,
}
fn normalized(value: &str) -> String {
    value
        .nfc()
        .flat_map(char::to_lowercase)
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
fn language(value: &str) -> String {
    value
        .trim()
        .to_lowercase()
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .into()
}

/// Return all supported candidates in stable evidence order; never select one automatically.
pub fn edition_candidates(
    state: &State,
    metadata: Option<&AudioMetadata>,
) -> Result<Vec<AudioEditionCandidate>> {
    let Some(metadata) = metadata else {
        return Ok(vec![]);
    };
    metadata.validate()?;
    let title = metadata
        .title
        .as_deref()
        .map(normalized)
        .filter(|s| !s.is_empty());
    let authors: Vec<_> = metadata
        .authors
        .iter()
        .map(|s| normalized(s))
        .filter(|s| !s.is_empty())
        .collect();
    let mut candidates = vec![];
    for edition in state.editions.values() {
        let Some(book) = state.books.get(&edition.book_id) else {
            continue;
        };
        let identifier = metadata
            .ebook_identifiers
            .iter()
            .find(|id| edition.identifiers.contains(id));
        let same_title = title
            .as_ref()
            .is_some_and(|title| title == &normalized(&book.title));
        let same_author = book
            .authors
            .iter()
            .any(|a| authors.contains(&normalized(a)));
        let mut reasons = vec![];
        let confidence = if let Some(id) = identifier {
            reasons.push(format!(
                "Source reports the same ebook identifier: {}:{}",
                id.provider, id.id
            ));
            MatchConfidence::Identifier
        } else if same_title && same_author {
            reasons.push(
                "Title and at least one author match after Unicode/case/punctuation normalization."
                    .into(),
            );
            MatchConfidence::TitleAndAuthor
        } else if same_title {
            reasons.push("Title matches, but no matching author was established.".into());
            MatchConfidence::TitleOnly
        } else {
            continue;
        };
        let mut warnings = vec!["Review edition, narrator language and abridgment. Audio time will remain independent of ebook text.".into()];
        if let (Some(audio_language), Some(ebook_language)) =
            (&metadata.language, &edition.language)
            && language(audio_language) != language(ebook_language)
        {
            warnings.push(format!(
                "Language differs: source audio {audio_language}; ebook {ebook_language}."
            ));
        }
        if identifier.is_some() && title.is_some() && !same_title {
            warnings.push("The reported identifier matches, but the titles differ.".into());
        }
        let mut labels: Vec<String> = edition
            .identifiers
            .iter()
            .map(|id| format!("{}: {}", id.provider, id.id))
            .collect();
        if let Some(language) = &edition.language {
            labels.push(language.clone());
        }
        if edition
            .assets
            .iter()
            .any(|a| a.media_type == "application/epub+zip")
        {
            labels.push("EPUB".into());
        }
        if edition
            .assets
            .iter()
            .any(|a| a.media_type == "application/pdf")
        {
            labels.push("PDF".into());
        }
        if labels.is_empty() {
            labels.push("Catalog edition; no accessible text".into());
        }
        let edition_label = labels.join(" · ");
        candidates.push(AudioEditionCandidate {
            edition_label,
            book_id: book.id.clone(),
            edition_id: edition.id.clone(),
            title: book.title.clone(),
            authors: book.authors.clone(),
            confidence,
            reasons,
            warnings,
        });
    }
    candidates.sort_by(|a, b| {
        (a.confidence, &a.title, &a.edition_id).cmp(&(b.confidence, &b.title, &b.edition_id))
    });
    Ok(candidates)
}

/// The UI obtains an explicit reader confirmation. The association is checked again against
/// current state; metadata-only guesses cannot create a confirmed position by themselves.
pub fn reviewed_observation(
    state: &State,
    source: &AudioObservation,
    pass_id: &str,
    confirmed_edition_id: Option<&str>,
    now: u64,
) -> Result<ProgressObservation> {
    source.validate(std::slice::from_ref(&source.asin), now)?;
    let pass = state
        .passes
        .get(pass_id)
        .ok_or_else(|| Error::NotFound("Reading pass".into()))?;
    let mut observation = source.attach(pass_id);
    if let Some(edition_id) = confirmed_edition_id {
        let candidate = edition_candidates(state, source.metadata.as_ref())?.into_iter().find(|c| c.edition_id == edition_id && c.book_id == pass.book_id).ok_or_else(|| Error::Conflict("The selected edition no longer matches the source metadata and reading pass. Review the match again.".into()))?;
        observation.alignment = Alignment::Confirmed {
            edition_id: candidate.edition_id,
            evidence: format!(
                "Reader confirmed edition association for Audible {}. {} Audio time is not mapped to ebook text.",
                source.asin,
                candidate.reasons.join(" ")
            ),
        };
    }
    Ok(observation)
}
