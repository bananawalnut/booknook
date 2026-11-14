// SPDX-License-Identifier: MIT OR Apache-2.0
//! Independent, data-only feed SDK. These objects own real Rust review lifetimes;
//! previews cannot recreate reviews or grant persistence/network authority.
use crate::Result;
use booknook_core::feeds::{
    ConflictResolution, DisclosureSelection, FeedConfiguration, FeedSection, ImportReview,
    ProjectionReview,
};
use std::sync::Arc;
fn encode(value: &impl serde::Serialize) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}
#[uniffi::export]
pub fn validate_feed_section_v1(raw: Vec<u8>) -> Result<String> {
    encode(&FeedSection::parse(&raw)?)
}
#[uniffi::export]
pub fn validate_feed_configuration_v1(raw: Vec<u8>) -> Result<String> {
    encode(&FeedConfiguration::parse(&raw)?)
}
#[uniffi::export]
pub fn empty_feed_configuration_v1() -> Result<String> {
    encode(&FeedConfiguration::empty())
}

#[derive(uniffi::Object)]
pub struct FeedImportReviewV1 {
    inner: ImportReview,
}
#[uniffi::export]
impl FeedImportReviewV1 {
    #[uniffi::constructor]
    pub fn new(configuration_bytes: Vec<u8>, incoming_bytes: Option<Vec<u8>>) -> Result<Self> {
        let current = FeedConfiguration::parse(&configuration_bytes)?;
        Ok(Self {
            inner: ImportReview::prepare(&current, incoming_bytes.as_deref())?,
        })
    }
    pub fn preview(&self) -> Result<String> {
        encode(
            &serde_json::json!({"schema":"booknook.feed-import-review.v1","conflicts":self.inner.conflicts(),"candidate":self.inner.candidate()}),
        )
    }
    pub fn resolve(
        &self,
        configuration_bytes: Vec<u8>,
        incoming_bytes: Option<Vec<u8>>,
        resolution_bytes: Vec<u8>,
    ) -> Result<Arc<Self>> {
        let current = FeedConfiguration::parse(&configuration_bytes)?;
        let choices = ConflictResolution::parse_all(&resolution_bytes)?;
        Ok(Arc::new(Self {
            inner: self
                .inner
                .resolve(&current, incoming_bytes.as_deref(), &choices)?,
        }))
    }
    /// Return a candidate only. The host owns consent and atomic persistence.
    pub fn confirm(
        &self,
        configuration_bytes: Vec<u8>,
        incoming_bytes: Option<Vec<u8>>,
    ) -> Result<String> {
        let current = FeedConfiguration::parse(&configuration_bytes)?;
        encode(&self.inner.confirm(&current, incoming_bytes.as_deref())?)
    }
}
#[derive(uniffi::Object)]
pub struct FeedProjectionReviewV1 {
    inner: ProjectionReview,
}
#[uniffi::export]
impl FeedProjectionReviewV1 {
    #[uniffi::constructor]
    pub fn new(configuration_bytes: Vec<u8>, selection_bytes: Vec<u8>) -> Result<Self> {
        let current = FeedConfiguration::parse(&configuration_bytes)?;
        let selection = DisclosureSelection::parse(&selection_bytes)?;
        Ok(Self {
            inner: ProjectionReview::prepare(&current, &selection)?,
        })
    }
    pub fn preview(&self) -> Result<String> {
        encode(self.inner.candidate())
    }
    /// Returns the minimal origin-bound projection; this does not send it.
    pub fn confirm(&self, configuration_bytes: Vec<u8>, origin: String) -> Result<String> {
        let current = FeedConfiguration::parse(&configuration_bytes)?;
        encode(&self.inner.confirm(&current, &origin)?)
    }
}
