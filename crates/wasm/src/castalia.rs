// SPDX-License-Identifier: MIT OR Apache-2.0
//! Portable verification only. Private custody and consent remain in the native host.
use wasm_bindgen::prelude::*;

/// Preserve original JSON until the shared duplicate/unknown-field checks have run.
/// Expected request bytes must already be canonical; the host supplies independent policy.
#[wasm_bindgen]
pub fn validate_castalia_request(
    request_json: &str,
    expected_request_json: &str,
    expected_disclosure_json: &str,
    policy_json: &str,
    now: u64,
) -> Result<String, JsValue> {
    super::bridge((|| {
        booknook_core::authority::validate_castalia_request(
            request_json.as_bytes(),
            &booknook_core::strict_parse(policy_json.as_bytes())?,
            expected_request_json.as_bytes(),
            &booknook_core::strict_parse(expected_disclosure_json.as_bytes())?,
            now,
        )
    })())
}

/// A verified presentation does not execute an application action or consume its replay ID.
/// Hosts validate their own action and atomically consume replay_key with durable changes.
#[wasm_bindgen]
pub fn verify_castalia_receipt(
    request_json: &str,
    presentation_json: &str,
    expected_request_json: &str,
    expected_disclosure_json: &str,
    policy_json: &str,
    now: u64,
) -> Result<String, JsValue> {
    super::bridge((|| {
        let verified = booknook_core::authority::verify_castalia_receipt(
            request_json.as_bytes(),
            presentation_json.as_bytes(),
            &booknook_core::strict_parse(policy_json.as_bytes())?,
            expected_request_json.as_bytes(),
            &booknook_core::strict_parse(expected_disclosure_json.as_bytes())?,
            now,
        )?;
        Ok(serde_json::json!({
            "holder_public_key": verified.holder_public_key,
            "request_digest_sha256": verified.request_digest_sha256,
            "expires_at": verified.expires_at,
            "replay_key": verified.replay_key()
        }))
    })())
}
