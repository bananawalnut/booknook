// SPDX-License-Identifier: MIT OR Apache-2.0
/// Pure bounded RSS2/Atom normalization. Output is inert produced-only JSON;
/// this neither fetches nor validates reopened durable normalized caches.
#[uniffi::export]
pub fn normalize_feed_xml_v1(context_bytes: Vec<u8>, xml_bytes: Vec<u8>) -> crate::Result<String> {
    Ok(serde_json::to_string(&booknook_core::feed_xml::normalize(
        &context_bytes,
        &xml_bytes,
    )?)?)
}
