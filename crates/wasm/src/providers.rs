// SPDX-License-Identifier: MIT OR Apache-2.0
//! Shared provider validation/reconciliation. Hosts own asynchronous I/O and storage CAS.
use booknook_core::{provider_runtime as runtime, strict_parse};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn provider_registry(registry_json: &str) -> Result<String, JsValue> {
    super::bridge(runtime::parse_registry(registry_json.as_bytes()))
}

#[wasm_bindgen]
pub fn provider_prepare_operation(
    registry_json: &str,
    generation: u64,
    provider_id: &str,
    operation_json: &str,
    request_id: &str,
) -> Result<String, JsValue> {
    super::bridge((|| {
        runtime::prepare_operation(
            &runtime::parse_registry(registry_json.as_bytes())?,
            generation,
            provider_id,
            operation_json.as_bytes(),
            request_id,
        )
    })())
}

#[wasm_bindgen]
pub fn provider_validate_reply(
    registry_json: &str,
    generation: u64,
    request_json: &str,
    response_json: &str,
    content_bytes: &[u8],
    now: u64,
) -> Result<String, JsValue> {
    super::bridge((|| {
        runtime::validate_reply(
            &runtime::parse_registry(registry_json.as_bytes())?,
            generation,
            &strict_parse(request_json.as_bytes())?,
            response_json.as_bytes(),
            content_bytes,
            now,
        )
    })())
}

#[wasm_bindgen]
pub fn provider_validate_execution(
    registry_json: &str,
    generation: u64,
    execution_json: &str,
    content_bytes: &[u8],
    now: u64,
) -> Result<String, JsValue> {
    super::bridge((|| {
        runtime::validate_execution(
            &runtime::parse_registry(registry_json.as_bytes())?,
            generation,
            execution_json.as_bytes(),
            content_bytes,
            now,
        )
    })())
}

/// Shared import rules run against freshly loaded state. The host commits the returned
/// state only with an atomic CAS against that starting revision; failure is not success.
#[wasm_bindgen]
pub fn provider_apply_catalog(
    state_json: &str,
    registry_json: &str,
    generation: u64,
    execution_json: &str,
    now: u64,
) -> Result<String, JsValue> {
    super::bridge((|| {
        let mut state = strict_parse(state_json.as_bytes())?;
        booknook_core::validate(&state)?;
        let result = runtime::apply_catalog(
            &mut state,
            &runtime::parse_registry(registry_json.as_bytes())?,
            generation,
            execution_json.as_bytes(),
            now,
        )?;
        booknook_core::validate(&state)?;
        Ok(serde_json::json!({"state":state,"result":result}))
    })())
}

/// Preserve an explicitly reviewed provider location without inventing an edition alignment.
#[wasm_bindgen]
pub fn provider_apply_source_position(
    state_json: &str,
    registry_json: &str,
    generation: u64,
    execution_json: &str,
    observation_json: &str,
    pass_id: &str,
    now: u64,
) -> Result<String, JsValue> {
    super::bridge((|| {
        let mut state = strict_parse(state_json.as_bytes())?;
        booknook_core::validate(&state)?;
        let result = runtime::apply_source_position(
            &mut state,
            &runtime::parse_registry(registry_json.as_bytes())?,
            generation,
            execution_json.as_bytes(),
            observation_json.as_bytes(),
            pass_id,
            now,
        )?;
        booknook_core::validate(&state)?;
        Ok(serde_json::json!({"state":state,"result":result}))
    })())
}
