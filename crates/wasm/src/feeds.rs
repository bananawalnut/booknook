// SPDX-License-Identifier: MIT OR Apache-2.0
//! Byte-preserving feed SDK over the same Rust review objects as UniFFI.
//! No review is deserializable. Dropping/freeing objects ends their lifetime.
use booknook_core::feeds::{
    ConflictResolution, DisclosureSelection, ExportReview, ExportSelection, FeedConfiguration,
    FeedSection, ImportReview, ProjectionReview,
};
use wasm_bindgen::prelude::*;
fn failure(error: booknook_core::Error) -> JsValue {
    JsValue::from_str(&error.to_string())
}
fn encode(value: &impl serde::Serialize) -> Result<String, JsValue> {
    serde_json::to_string(value).map_err(|error| JsValue::from_str(&error.to_string()))
}
#[wasm_bindgen]
pub fn validate_feed_section_v1(raw: &[u8]) -> Result<String, JsValue> {
    encode(&FeedSection::parse(raw).map_err(failure)?)
}
#[wasm_bindgen]
pub fn validate_feed_configuration_v1(raw: &[u8]) -> Result<String, JsValue> {
    encode(&FeedConfiguration::parse(raw).map_err(failure)?)
}
#[wasm_bindgen]
pub fn empty_feed_configuration_v1() -> Result<String, JsValue> {
    encode(&FeedConfiguration::empty())
}

#[wasm_bindgen]
pub struct FeedImportReviewV1 {
    inner: ImportReview,
}
#[wasm_bindgen]
impl FeedImportReviewV1 {
    #[wasm_bindgen(constructor)]
    pub fn new(
        configuration_bytes: &[u8],
        incoming_bytes: Option<Vec<u8>>,
    ) -> Result<FeedImportReviewV1, JsValue> {
        let current = FeedConfiguration::parse(configuration_bytes).map_err(failure)?;
        Ok(Self {
            inner: ImportReview::prepare(&current, incoming_bytes.as_deref()).map_err(failure)?,
        })
    }
    pub fn preview(&self) -> Result<String, JsValue> {
        encode(
            &serde_json::json!({"schema":"booknook.feed-import-review.v1","conflicts":self.inner.conflicts(),"candidate":self.inner.candidate()}),
        )
    }
    pub fn resolve(
        &self,
        configuration_bytes: &[u8],
        incoming_bytes: Option<Vec<u8>>,
        resolution_bytes: &[u8],
    ) -> Result<FeedImportReviewV1, JsValue> {
        let current = FeedConfiguration::parse(configuration_bytes).map_err(failure)?;
        let choices = ConflictResolution::parse_all(resolution_bytes).map_err(failure)?;
        Ok(Self {
            inner: self
                .inner
                .resolve(&current, incoming_bytes.as_deref(), &choices)
                .map_err(failure)?,
        })
    }
    pub fn confirm(
        &self,
        configuration_bytes: &[u8],
        incoming_bytes: Option<Vec<u8>>,
    ) -> Result<String, JsValue> {
        let current = FeedConfiguration::parse(configuration_bytes).map_err(failure)?;
        encode(
            &self
                .inner
                .confirm(&current, incoming_bytes.as_deref())
                .map_err(failure)?,
        )
    }
}
#[wasm_bindgen]
pub struct FeedProjectionReviewV1 {
    inner: ProjectionReview,
}
#[wasm_bindgen]
impl FeedProjectionReviewV1 {
    #[wasm_bindgen(constructor)]
    pub fn new(
        configuration_bytes: &[u8],
        selection_bytes: &[u8],
    ) -> Result<FeedProjectionReviewV1, JsValue> {
        let current = FeedConfiguration::parse(configuration_bytes).map_err(failure)?;
        let selection = DisclosureSelection::parse(selection_bytes).map_err(failure)?;
        Ok(Self {
            inner: ProjectionReview::prepare(&current, &selection).map_err(failure)?,
        })
    }
    pub fn preview(&self) -> Result<String, JsValue> {
        encode(self.inner.candidate())
    }
    pub fn confirm(&self, configuration_bytes: &[u8], origin: &str) -> Result<String, JsValue> {
        let current = FeedConfiguration::parse(configuration_bytes).map_err(failure)?;
        encode(&self.inner.confirm(&current, origin).map_err(failure)?)
    }
}
#[wasm_bindgen]
pub struct FeedExportReviewV1 {
    inner: ExportReview,
}
#[wasm_bindgen]
impl FeedExportReviewV1 {
    #[wasm_bindgen(constructor)]
    pub fn new(
        configuration_bytes: &[u8],
        selection_bytes: &[u8],
    ) -> Result<FeedExportReviewV1, JsValue> {
        let current = FeedConfiguration::parse(configuration_bytes).map_err(failure)?;
        let selection = ExportSelection::parse(selection_bytes).map_err(failure)?;
        Ok(Self {
            inner: ExportReview::prepare(&current, &selection).map_err(failure)?,
        })
    }
    pub fn preview(&self) -> Result<String, JsValue> {
        encode(self.inner.candidate())
    }
    pub fn confirm(&self, configuration_bytes: &[u8]) -> Result<String, JsValue> {
        let current = FeedConfiguration::parse(configuration_bytes).map_err(failure)?;
        encode(&self.inner.confirm(&current).map_err(failure)?)
    }
}
