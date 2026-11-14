// SPDX-License-Identifier: MIT OR Apache-2.0
//! Portable, inert feed configuration and explicit local reviews.
//!
//! This independent SDK domain does not extend the issued provider-v1 contract.
//! It contains no custody, identity, networking, polling or activation operations.
//! `enabled` and `selected` remain preferences. Neither grants fetch, disclosure,
//! signing or other operational authority. A host must obtain consent, persist
//! atomically and check actual public addresses at every network boundary.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_SECTION_BYTES: usize = 256 * 1024;
pub const MAX_SUBSCRIPTIONS: usize = 256;
pub const MAX_COLLECTIONS: usize = 64;
pub const MAX_COLLECTION_REFERENCES: usize = 16;
pub const MAX_REVISION: u64 = 9_007_199_254_740_991;

fn invalid(message: &str) -> Error {
    Error::Invalid(message.into())
}
fn require(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(invalid(message))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeedSectionSchema {
    #[serde(rename = "castalia.feeds-section.v1")]
    V1,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeedFormat {
    Auto,
    Rss,
    Atom,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeedDisclosure {
    Private,
    Selected,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeedOrder {
    #[serde(rename = "newest-first")]
    NewestFirst,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedPreferences {
    pub order: FeedOrder,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedCollection {
    pub id: String,
    pub title: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeedSubscription {
    pub id: String,
    pub url: String,
    pub format: FeedFormat,
    pub title: String,
    pub enabled: bool,
    pub collection_ids: Vec<String>,
    pub disclosure: FeedDisclosure,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedSection {
    pub schema: FeedSectionSchema,
    pub version: u8,
    pub subscriptions: Vec<FeedSubscription>,
    pub collections: Vec<FeedCollection>,
    pub preferences: FeedPreferences,
}

fn label(value: &str) -> Result<()> {
    require(
        value.len() <= 512 && !value.chars().any(char::is_control),
        "Feed labels must be Unicode scalar text without control characters, at most 512 UTF-8 bytes",
    )
}
fn uuid(value: &str) -> Result<()> {
    let bytes = value.as_bytes();
    require(
        bytes.len() == 36
            && bytes.iter().enumerate().all(|(index, byte)| {
                if [8, 13, 18, 23].contains(&index) {
                    *byte == b'-'
                } else {
                    byte.is_ascii_digit() || (b'a'..=b'f').contains(byte)
                }
            })
            && (b'1'..=b'8').contains(&bytes[14])
            && b"89ab".contains(&bytes[19]),
        "Feed IDs must be canonical lowercase UUIDs with supported version and variant",
    )
}
/// Syntax only. A canonical hostname may still resolve to a private address.
fn endpoint(raw: &str) -> Result<()> {
    require(
        raw.len() <= 2048
            && raw.starts_with("https://")
            && !raw.chars().any(|c| c.is_control() || c.is_whitespace())
            && !raw.contains('\\')
            && !raw.as_bytes().iter().enumerate().any(|(index, byte)| {
                *byte == b'%'
                    && !raw
                        .as_bytes()
                        .get(index + 1..index + 3)
                        .is_some_and(|digits| digits.iter().all(u8::is_ascii_hexdigit))
            }),
        "Feed URL must be canonical public HTTPS syntax without malformed escapes",
    )?;
    let url = url::Url::parse(raw).map_err(|_| invalid("Malformed feed URL"))?;
    let Some(url::Host::Domain(host)) = url.host() else {
        return Err(invalid("Feed URLs require DNS hostnames, not IP literals"));
    };
    let suffixes = [
        "localhost",
        "local",
        "internal",
        "home",
        "lan",
        "corp",
        "test",
        "invalid",
        "example",
        "onion",
        "arpa",
    ];
    require(
        url.as_str() == raw
            && url.scheme() == "https"
            && url.port().is_none()
            && url.username().is_empty()
            && url.password().is_none()
            && !raw[8..].split('/').next().unwrap_or_default().contains('@')
            && url.query().is_none()
            && url.fragment().is_none()
            && host.contains('.')
            && host.len() <= 253
            && host.split('.').all(|part| {
                !part.is_empty()
                    && part.len() <= 63
                    && part.as_bytes()[0].is_ascii_alphanumeric()
                    && part.as_bytes()[part.len() - 1].is_ascii_alphanumeric()
                    && part
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            })
            && !suffixes
                .iter()
                .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}"))),
        "Feed URL is not a canonical admissible public DNS endpoint",
    )
}
fn unique_ids(values: &[String], maximum: usize) -> Result<BTreeSet<&str>> {
    require(
        values.len() <= maximum,
        "Feed ID selection exceeds its count limit",
    )?;
    let mut seen = BTreeSet::new();
    for value in values {
        uuid(value)?;
        require(seen.insert(value.as_str()), "Duplicate feed ID selection")?;
    }
    Ok(seen)
}

impl FeedSection {
    pub fn empty() -> Self {
        Self {
            schema: FeedSectionSchema::V1,
            version: 1,
            subscriptions: vec![],
            collections: vec![],
            preferences: FeedPreferences {
                order: FeedOrder::NewestFirst,
            },
        }
    }
    /// Original bounded bytes are required so duplicates, invalid UTF-8/surrogates
    /// and floating-point aliases of the integer version cannot be hidden.
    pub fn parse(raw: &[u8]) -> Result<Self> {
        require(
            !raw.is_empty() && raw.len() <= MAX_SECTION_BYTES,
            "Feed JSON exceeds 256 KiB or is empty",
        )?;
        let section: Self = crate::strict_parse(raw)?;
        section.validate()?;
        Ok(section)
    }
    pub fn validate(&self) -> Result<()> {
        require(self.version == 1, "Unsupported feed section version")?;
        require(
            self.subscriptions.len() <= MAX_SUBSCRIPTIONS
                && self.collections.len() <= MAX_COLLECTIONS,
            "Feed section exceeds its subscription or collection limit",
        )?;
        require(
            serde_json::to_vec(self)?.len() <= MAX_SECTION_BYTES,
            "Serialized feed section exceeds 256 KiB",
        )?;
        let mut collections = BTreeSet::new();
        for collection in &self.collections {
            uuid(&collection.id)?;
            label(&collection.title)?;
            require(
                collections.insert(collection.id.as_str()),
                "Duplicate feed collection ID",
            )?;
        }
        let mut subscriptions = BTreeSet::new();
        for subscription in &self.subscriptions {
            uuid(&subscription.id)?;
            require(
                subscriptions.insert(subscription.id.as_str()),
                "Duplicate feed subscription ID",
            )?;
            label(&subscription.title)?;
            endpoint(&subscription.url)?;
            let references = unique_ids(&subscription.collection_ids, MAX_COLLECTION_REFERENCES)?;
            require(
                references.iter().all(|id| collections.contains(id)),
                "Feed collection reference does not resolve",
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeedConfigurationSchema {
    #[serde(rename = "booknook.feed-configuration.v1")]
    V1,
}
/// Data-only local configuration, separate from book archives and social events.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedConfiguration {
    pub schema: FeedConfigurationSchema,
    pub revision: u64,
    pub feeds: FeedSection,
}
impl FeedConfiguration {
    pub fn empty() -> Self {
        Self {
            schema: FeedConfigurationSchema::V1,
            revision: 0,
            feeds: FeedSection::empty(),
        }
    }
    pub fn new(revision: u64, feeds: FeedSection) -> Result<Self> {
        let value = Self {
            schema: FeedConfigurationSchema::V1,
            revision,
            feeds,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn parse(raw: &[u8]) -> Result<Self> {
        require(
            raw.len() <= MAX_SECTION_BYTES + 4096,
            "Feed configuration exceeds its byte limit",
        )?;
        let value: Self = crate::strict_parse(raw)?;
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        require(
            self.revision <= MAX_REVISION,
            "Feed configuration revision exceeds the portable integer range",
        )?;
        self.feeds.validate()
    }
    fn fingerprint(&self) -> Result<String> {
        self.validate()?;
        Ok(crate::digest(&crate::canonical(self)?))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictTarget {
    Collection,
    Subscription,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionChoice {
    Current,
    Incoming,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConflictResolution {
    pub target: ConflictTarget,
    pub id: String,
    pub choice: ResolutionChoice,
}
/// Both interpretations are retained for local review; no partial candidate commits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FeedConflict {
    pub target: ConflictTarget,
    pub id: String,
    pub current: Value,
    pub incoming: Value,
}

/// A review is an in-memory value, not an authorization token or a persistence write.
/// It cannot be deserialized from untrusted input. Hosts must keep its lifetime
/// within their own consent flow and atomically persist a confirmed configuration.
#[derive(Clone, Debug)]
pub struct ImportReview {
    configuration_digest: String,
    incoming_digest: Option<String>,
    conflicts: Vec<FeedConflict>,
    candidate: Option<FeedConfiguration>,
}
fn incoming_digest(raw: Option<&[u8]>) -> Result<Option<String>> {
    raw.map(|bytes| {
        require(
            !bytes.is_empty() && bytes.len() <= MAX_SECTION_BYTES,
            "Feed import exceeds its byte limit or is empty",
        )?;
        Ok(crate::digest(bytes))
    })
    .transpose()
}
fn merge_records<T: Clone + PartialEq + Serialize>(
    current: &mut Vec<T>,
    incoming: &[T],
    target: ConflictTarget,
    id: fn(&T) -> &str,
    choices: &mut BTreeMap<(ConflictTarget, String), ResolutionChoice>,
    conflicts: &mut Vec<FeedConflict>,
) -> Result<()> {
    for record in incoming {
        if let Some(index) = current.iter().position(|saved| id(saved) == id(record)) {
            if current[index] != *record {
                match choices.remove(&(target, id(record).into())) {
                    Some(ResolutionChoice::Current) => (),
                    Some(ResolutionChoice::Incoming) => current[index] = record.clone(),
                    None => conflicts.push(FeedConflict {
                        target,
                        id: id(record).into(),
                        current: serde_json::to_value(&current[index])?,
                        incoming: serde_json::to_value(record)?,
                    }),
                }
            }
        } else {
            current.push(record.clone());
        }
    }
    Ok(())
}
impl ImportReview {
    /// Imports are additive. An absent section, an empty section and omitted
    /// records never infer deletion. Conflicting IDs require explicit choices.
    pub fn prepare(current: &FeedConfiguration, incoming: Option<&[u8]>) -> Result<Self> {
        Self::build(current, incoming, &[])
    }
    /// Resolve one complete batch of this review's conflicts. Choices cannot be
    /// carried to changed source bytes or configuration: start a fresh review.
    /// A resolved review can be confirmed, but changing its choices also requires
    /// preparing a fresh review. Hosts still own consent and persistence.
    pub fn resolve(
        &self,
        current: &FeedConfiguration,
        incoming: Option<&[u8]>,
        resolutions: &[ConflictResolution],
    ) -> Result<Self> {
        self.check_inputs(current, incoming)?;
        require(
            resolutions.len() == self.conflicts.len(),
            "Every unresolved feed conflict needs exactly one reviewed choice",
        )?;
        if self.conflicts.is_empty() {
            return Ok(self.clone());
        }
        for resolution in resolutions {
            require(
                self.conflicts.iter().any(|conflict| {
                    conflict.target == resolution.target && conflict.id == resolution.id
                }),
                "Feed resolution is not part of this review",
            )?;
        }
        Self::build(current, incoming, resolutions)
    }
    fn build(
        current: &FeedConfiguration,
        incoming: Option<&[u8]>,
        resolutions: &[ConflictResolution],
    ) -> Result<Self> {
        let configuration_digest = current.fingerprint()?;
        let incoming_digest = incoming_digest(incoming)?;
        let imported = incoming.map(FeedSection::parse).transpose()?;
        require(
            resolutions.len() <= MAX_SUBSCRIPTIONS + MAX_COLLECTIONS,
            "Too many feed conflict resolutions",
        )?;
        let mut choices = BTreeMap::new();
        for resolution in resolutions {
            uuid(&resolution.id)?;
            require(
                choices
                    .insert(
                        (resolution.target, resolution.id.clone()),
                        resolution.choice,
                    )
                    .is_none(),
                "Duplicate feed conflict resolution",
            )?;
        }
        let mut merged = current.feeds.clone();
        let mut conflicts = vec![];
        if let Some(imported) = imported {
            merge_records(
                &mut merged.collections,
                &imported.collections,
                ConflictTarget::Collection,
                |c| &c.id,
                &mut choices,
                &mut conflicts,
            )?;
            merge_records(
                &mut merged.subscriptions,
                &imported.subscriptions,
                ConflictTarget::Subscription,
                |s| &s.id,
                &mut choices,
                &mut conflicts,
            )?;
        }
        require(
            choices.is_empty(),
            "Feed resolution does not identify an actual conflict",
        )?;
        merged.validate()?;
        let candidate = if conflicts.is_empty() {
            let revision = if merged == current.feeds {
                current.revision
            } else {
                current
                    .revision
                    .checked_add(1)
                    .filter(|revision| *revision <= MAX_REVISION)
                    .ok_or_else(|| {
                        Error::Conflict("Feed configuration revision exhausted".into())
                    })?
            };
            Some(FeedConfiguration::new(revision, merged)?)
        } else {
            None
        };
        Ok(Self {
            configuration_digest,
            incoming_digest,
            conflicts,
            candidate,
        })
    }
    pub fn conflicts(&self) -> &[FeedConflict] {
        &self.conflicts
    }
    pub fn candidate(&self) -> Option<&FeedConfiguration> {
        self.candidate.as_ref()
    }
    /// Pure confirmation: this returns data, and never persists or activates it.
    pub fn confirm(
        &self,
        current: &FeedConfiguration,
        incoming: Option<&[u8]>,
    ) -> Result<FeedConfiguration> {
        self.check_inputs(current, incoming)?;
        self.candidate
            .clone()
            .ok_or_else(|| Error::Conflict("Feed import has unresolved interpretations".into()))
    }
    fn check_inputs(&self, current: &FeedConfiguration, incoming: Option<&[u8]>) -> Result<()> {
        if current.fingerprint()? != self.configuration_digest
            || incoming_digest(incoming)? != self.incoming_digest
        {
            return Err(Error::Conflict(
                "Feed configuration or exact import bytes changed after review".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DisclosureSelectionSchema {
    #[serde(rename = "booknook.feed-disclosure-selection.v1")]
    V1,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MembershipSelection {
    pub subscription_id: String,
    pub collection_ids: Vec<String>,
}
/// Labels and each subscription's collection memberships are separate choices.
/// Every selected subscription must have an explicit membership choice, even [].
/// Projection omits every collection label not referenced by those chosen
/// memberships, including labels belonging only to an unselected source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DisclosureSelection {
    pub schema: DisclosureSelectionSchema,
    pub origin: String,
    pub subscription_ids: Vec<String>,
    pub collection_ids: Vec<String>,
    pub memberships: Vec<MembershipSelection>,
}
impl DisclosureSelection {
    pub fn new(
        origin: String,
        subscription_ids: Vec<String>,
        collection_ids: Vec<String>,
        memberships: Vec<MembershipSelection>,
    ) -> Self {
        Self {
            schema: DisclosureSelectionSchema::V1,
            origin,
            subscription_ids,
            collection_ids,
            memberships,
        }
    }
}
fn exact_origin(raw: &str) -> Result<()> {
    require(
        raw.len() <= 2048,
        "Feed disclosure origin exceeds its byte limit",
    )?;
    let url = url::Url::parse(raw).map_err(|_| invalid("Malformed feed disclosure origin"))?;
    require(
        url.origin().ascii_serialization() == raw,
        "Feed disclosure requires an exact canonical HTTPS origin",
    )?;
    endpoint(&format!("{raw}/"))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum SelectedFeedProjectionSchema {
    #[serde(rename = "booknook.selected-feeds.v1")]
    V1,
}
/// Only explicitly selected public configuration is present. No source snapshot,
/// unselected IDs, private identity, cache, credential or authority accompanies it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SelectedFeedProjection {
    schema: SelectedFeedProjectionSchema,
    origin: String,
    feeds: FeedSection,
}
impl SelectedFeedProjection {
    pub fn origin(&self) -> &str {
        &self.origin
    }
    pub fn feeds(&self) -> &FeedSection {
        &self.feeds
    }
}
#[derive(Clone, Debug)]
pub struct ProjectionReview {
    configuration_digest: String,
    candidate: SelectedFeedProjection,
}
impl ProjectionReview {
    pub fn prepare(current: &FeedConfiguration, selection: &DisclosureSelection) -> Result<Self> {
        let configuration_digest = current.fingerprint()?;
        exact_origin(&selection.origin)?;
        let subscriptions = unique_ids(&selection.subscription_ids, MAX_SUBSCRIPTIONS)?;
        let collections = unique_ids(&selection.collection_ids, MAX_COLLECTIONS)?;
        require(
            selection.memberships.len() == subscriptions.len(),
            "Every selected feed needs an explicit collection membership choice",
        )?;
        let mut memberships = BTreeMap::new();
        for membership in &selection.memberships {
            uuid(&membership.subscription_id)?;
            require(
                subscriptions.contains(membership.subscription_id.as_str()),
                "Membership choice names an unselected feed",
            )?;
            let references = unique_ids(&membership.collection_ids, MAX_COLLECTION_REFERENCES)?;
            require(
                references.iter().all(|id| collections.contains(id)),
                "Feed membership requires an explicitly selected collection label",
            )?;
            require(
                memberships
                    .insert(membership.subscription_id.as_str(), references)
                    .is_none(),
                "Duplicate feed membership choice",
            )?;
        }
        for id in &subscriptions {
            let source = current
                .feeds
                .subscriptions
                .iter()
                .find(|source| source.id == **id)
                .ok_or_else(|| invalid("Selected feed does not exist"))?;
            require(
                source.disclosure == FeedDisclosure::Selected,
                "Private feed must first be marked selected through a reviewed configuration edit",
            )?;
            let references = memberships
                .get(id)
                .ok_or_else(|| invalid("Selected feed has no explicit membership choice"))?;
            require(
                references
                    .iter()
                    .all(|id| source.collection_ids.iter().any(|original| original == id)),
                "Selected membership is not part of this feed",
            )?;
        }
        require(
            collections.iter().all(|id| {
                current
                    .feeds
                    .collections
                    .iter()
                    .any(|collection| collection.id == **id)
            }),
            "Selected collection label does not exist",
        )?;
        let mut feeds = current.feeds.clone();
        feeds
            .subscriptions
            .retain(|source| subscriptions.contains(source.id.as_str()));
        for source in &mut feeds.subscriptions {
            let references = &memberships[source.id.as_str()];
            source
                .collection_ids
                .retain(|id| references.contains(id.as_str()));
        }
        let referenced: BTreeSet<&str> = feeds
            .subscriptions
            .iter()
            .flat_map(|source| source.collection_ids.iter().map(String::as_str))
            .collect();
        feeds
            .collections
            .retain(|collection| referenced.contains(collection.id.as_str()));
        feeds.validate()?;
        Ok(Self {
            configuration_digest,
            candidate: SelectedFeedProjection {
                schema: SelectedFeedProjectionSchema::V1,
                origin: selection.origin.clone(),
                feeds,
            },
        })
    }
    pub fn candidate(&self) -> &SelectedFeedProjection {
        &self.candidate
    }
    /// Exact destination and unchanged source are required again at confirmation.
    /// This is a local data projection, not a grant to send it or to fetch feeds.
    pub fn confirm(
        &self,
        current: &FeedConfiguration,
        origin: &str,
    ) -> Result<SelectedFeedProjection> {
        if origin != self.candidate.origin || current.fingerprint()? != self.configuration_digest {
            return Err(Error::Conflict(
                "Feed disclosure destination or configuration changed after review".into(),
            ));
        }
        Ok(self.candidate.clone())
    }
}
