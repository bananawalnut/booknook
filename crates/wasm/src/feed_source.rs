// SPDX-License-Identifier: MIT OR Apache-2.0
//! Opaque source attempts for a host Promise provider. Rust never auto-fetches.
use booknook_core::{Error, feed_source as f, feeds::FeedConfiguration};
use wasm_bindgen::prelude::*;
fn failure(error: Error) -> JsValue {
    JsValue::from_str(&error.to_string())
}
fn encode(value: &impl serde::Serialize) -> Result<String, JsValue> {
    serde_json::to_string(value).map_err(|e| JsValue::from_str(&e.to_string()))
}
#[wasm_bindgen]
pub fn validate_feed_source_descriptor_v1(raw: &[u8]) -> Result<String, JsValue> {
    encode(&f::FeedSourceDescriptor::parse(raw).map_err(failure)?)
}
#[wasm_bindgen(typescript_custom_section)]
const CACHE_CONTRACT: &str = r#"
/** Synchronous final durable CAS only. Async final transactions are unsupported. */
export interface FeedCacheCommitterV1 {
  compare_and_store(conditionJson: string, untrustedBody: Uint8Array): undefined;
}
"#;
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "FeedCacheCommitterV1")]
    pub type FeedCacheCommitterV1;
    #[wasm_bindgen(method, catch)]
    fn compare_and_store(
        this: &FeedCacheCommitterV1,
        condition_json: &str,
        body: &[u8],
    ) -> Result<JsValue, JsValue>;
}
#[wasm_bindgen]
pub struct FeedSourceSessionV1 {
    inner: f::FeedSourceSession,
    provider_id: String,
}
#[wasm_bindgen]
impl FeedSourceSessionV1 {
    #[wasm_bindgen(constructor)]
    pub fn new(
        configuration_bytes: &[u8],
        descriptor_bytes: &[u8],
    ) -> Result<FeedSourceSessionV1, JsValue> {
        let configuration = FeedConfiguration::parse(configuration_bytes).map_err(failure)?;
        let descriptor = f::FeedSourceDescriptor::parse(descriptor_bytes).map_err(failure)?;
        Ok(Self {
            provider_id: descriptor.id.clone(),
            inner: f::FeedSourceSession::new(configuration, vec![descriptor]).map_err(failure)?,
        })
    }
    pub fn close(&self) -> Result<(), JsValue> {
        self.inner.close().map_err(failure)
    }
    pub fn replace_configuration(&self, configuration_bytes: &[u8]) -> Result<(), JsValue> {
        self.inner
            .replace_configuration(FeedConfiguration::parse(configuration_bytes).map_err(failure)?)
            .map_err(failure)
    }
    pub fn replace_provider(&mut self, descriptor_bytes: &[u8]) -> Result<(), JsValue> {
        let descriptor = f::FeedSourceDescriptor::parse(descriptor_bytes).map_err(failure)?;
        self.inner
            .replace_providers(vec![descriptor.clone()])
            .map_err(failure)?;
        self.provider_id = descriptor.id;
        Ok(())
    }
    pub fn activate(&self, source_id: &str) -> Result<(), JsValue> {
        self.inner
            .activate(source_id, &self.provider_id)
            .map_err(failure)
    }
    pub fn disable(&self, source_id: &str) -> Result<(), JsValue> {
        self.inner.disable(source_id).map_err(failure)
    }
    pub fn begin(
        &self,
        source_id: &str,
        conditional_bytes: Option<Vec<u8>>,
    ) -> Result<FeedSourceAttemptV1, JsValue> {
        let hint = conditional_bytes
            .as_deref()
            .map(f::FeedConditionalHint::parse)
            .transpose()
            .map_err(failure)?;
        Ok(FeedSourceAttemptV1 {
            inner: self
                .inner
                .begin_with_conditional(source_id, hint)
                .map_err(failure)?,
        })
    }
}
#[wasm_bindgen]
pub struct FeedSourceAttemptV1 {
    inner: f::FeedSourceAttempt,
}
#[wasm_bindgen]
impl FeedSourceAttemptV1 {
    pub fn dispatch(&self) -> Result<String, JsValue> {
        encode(&self.inner.dispatch().map_err(failure)?)
    }
    pub fn cancel(&self) -> bool {
        self.inner.cancel()
    }
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancellation().is_cancelled()
    }
    pub fn complete(
        &self,
        response_bytes: &[u8],
        body: Vec<u8>,
    ) -> Result<FeedSourceReadV1, JsValue> {
        Ok(FeedSourceReadV1 {
            inner: self.inner.complete(response_bytes, body).map_err(failure)?,
        })
    }
}
#[wasm_bindgen]
pub struct FeedSourceReadV1 {
    inner: f::FeedSourceRead,
}
#[wasm_bindgen]
impl FeedSourceReadV1 {
    pub fn request(&self) -> Result<String, JsValue> {
        encode(self.inner.request())
    }
    pub fn response(&self) -> Result<String, JsValue> {
        encode(self.inner.response())
    }
    pub fn body(&self) -> Vec<u8> {
        self.inner.body().to_vec()
    }
    pub fn cache_condition(&self) -> Result<String, JsValue> {
        encode(&self.inner.cache_condition().map_err(failure)?)
    }
    pub fn cancel(&self) -> bool {
        self.inner.cancellation().cancel()
    }
    pub fn commit_cache(
        &self,
        committer: &FeedCacheCommitterV1,
        existing_body: Option<Vec<u8>>,
    ) -> Result<(), JsValue> {
        self.inner.with_current_cache_commit(existing_body.as_deref(),|condition|{
        let value=committer.compare_and_store(&serde_json::to_string(condition)?,self.inner.body()).map_err(|_|Error::Unavailable("Host synchronous cache CAS failed".into()))?;
        if !value.is_undefined(){return Err(Error::Invalid("Final cache CAS must be synchronous and return undefined; asynchronous final commits are unsupported".into()));}Ok(())
    }).map_err(failure)
    }
}
