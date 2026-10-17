// SPDX-License-Identifier: MIT OR Apache-2.0
//! A deliberately projected text snapshot, never serialization of private card/history records.
use crate::{
    Error, Result,
    model::{Location, State},
};

// Match the social publication body bound. Creating a draft must not silently truncate it
// or defer a predictable oversize failure until after the reader starts approval.
const MAX_BODY_BYTES: usize = 12_000;
const TOO_LARGE: &str = "This selection exceeds the 12,000-byte sharing limit. Choose a shorter interpretation or copy a smaller excerpt into a note draft. No draft was created.";
pub(crate) fn validate_share_body(body: &str) -> Result<()> {
    if body.trim().is_empty() {
        return Err(Error::Invalid("Select content to share".into()));
    }
    if body.len() > MAX_BODY_BYTES {
        return Err(Error::Invalid(TOO_LARGE.into()));
    }
    if body.contains('\0') {
        return Err(Error::Invalid(
            "Remove null characters from the selected text before sharing. No draft was created."
                .into(),
        ));
    }
    Ok(())
}
struct Text(String);
impl Text {
    fn push(&mut self, value: &str) -> Result<()> {
        if value.len() > MAX_BODY_BYTES.saturating_sub(self.0.len()) {
            return Err(Error::Invalid(TOO_LARGE.into()));
        }
        self.0.push_str(value);
        Ok(())
    }
    fn section(&mut self, title: &str, values: &[String]) -> Result<()> {
        if values.is_empty() {
            return Ok(());
        }
        self.push("\n\n")?;
        self.push(title)?;
        for value in values {
            self.push("\n• ")?;
            self.push(value)?;
        }
        Ok(())
    }
}
pub(crate) fn selected_version(state: &State, card_id: &str, version_id: &str) -> Result<String> {
    let card = state.cards.get(card_id).ok_or_else(|| {
        Error::NotFound("Character card was not found; no sharing draft was created".into())
    })?;
    let version = card
        .versions
        .iter()
        .find(|v| v.id == version_id)
        .ok_or_else(|| {
            Error::NotFound(
                "Choose a saved version of this character card; no sharing draft was created"
                    .into(),
            )
        })?;
    let character = state
        .entities
        .get(&card.character_id)
        .ok_or_else(|| Error::NotFound("Character name was not found".into()))?;
    let book = state
        .books
        .get(&character.book_id)
        .ok_or_else(|| Error::NotFound("Book context was not found".into()))?;
    let interpretation = &version.interpretation;
    let mut text = Text(String::new());
    text.push("Book: ")?;
    text.push(&book.title)?;
    if !book.authors.is_empty() {
        text.push("\nAuthors: ")?;
        for (index, author) in book.authors.iter().enumerate() {
            if index > 0 {
                text.push(", ")?;
            }
            text.push(author)?;
        }
    }
    text.push("\n")?;
    text.push("Character: ")?;
    text.push(&character.name)?;
    text.push("\nSelected interpretation")?;
    if !interpretation.description.is_empty() {
        text.push("\n\nDescription\n")?;
        text.push(&interpretation.description)?;
    }
    text.section("Traits", &interpretation.traits)?;
    if !interpretation.relationships.is_empty() {
        text.push("\n\nRelationships")?;
        for relationship in &interpretation.relationships {
            let target = state
                .entities
                .get(&relationship.target)
                .ok_or_else(|| Error::NotFound("Related literary name was not found".into()))?;
            text.push("\n• ")?;
            text.push(&target.name)?;
            text.push(": ")?;
            text.push(&relationship.description)?;
        }
    }
    text.section("Observations", &interpretation.observations)?;
    text.section("Questions", &interpretation.questions)?;
    if !interpretation.passages.is_empty() {
        text.push("\n\nSupporting passages")?;
        let mut omitted = 0;
        for passage in &interpretation.passages {
            if let Location::Text { quote, .. } = passage {
                text.push("\n\n“")?;
                text.push(quote)?;
                text.push("”")?;
            } else {
                omitted += 1;
            }
        }
        if omitted > 0 {
            text.push(&format!("\n\n{omitted} supporting reference(s) have no stored passage text and are omitted. Reading locations stay private."))?;
        }
    }
    validate_share_body(&text.0)?;
    Ok(text.0)
}
