// SPDX-License-Identifier: MIT OR Apache-2.0
use wasm_bindgen::prelude::*;
/// Pure normalization of original bounded bytes; all output strings are text.
#[wasm_bindgen]
pub fn normalize_feed_xml_v1(context_bytes: &[u8], xml_bytes: &[u8]) -> Result<String, JsValue> {
    let feed = booknook_core::feed_xml::normalize(context_bytes, xml_bytes)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    serde_json::to_string(&feed).map_err(|e| JsValue::from_str(&e.to_string()))
}
