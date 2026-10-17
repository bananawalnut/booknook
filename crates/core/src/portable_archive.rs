// SPDX-License-Identifier: MIT OR Apache-2.0
//! The complete, existing native export envelope. Validation conveys no identity trust.
//! Assets retain their original base64 strings; signed requests and relay JSON are opaque
//! strings, never rewritten. Native restore additionally validates EPUB/PDF content.
use crate::{
    Error, Result,
    model::State,
    social_protocol::{self as social, SocialState},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use nostr::{Event, JsonUtil, PublicKey};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA: &str = "booknook.portable-export.v1";
pub const SOCIAL_SCHEMA: &str = "booknook.social-archive.v1";

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PortableArchive {
    pub schema: String,
    pub state: State,
    pub assets: BTreeMap<String, String>,
    // Preserve the precise original JSON representation, including legacy omitted defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<SocialArchiveBundle>")]
    pub social: Option<serde_json::Value>,
}
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyBinding {
    pub schema: String,
    pub castalia_holder: String,
    pub nostr_public_key: String,
    pub valid_from: u64,
    pub valid_until: u64,
    pub version: u64,
}
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BindingDocument {
    pub schema: String,
    pub binding: KeyBinding,
    pub approved_at: u64,
    pub request_json: String,
    pub presentation_json: String,
}
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BindingHistory {
    pub document: BindingDocument,
    pub retired_at: Option<u64>,
}
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArchiveBinding {
    pub document: BindingDocument,
    pub minimum_version: u64,
}
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PendingEnvelope {
    pub id: String,
    pub event_json: String,
    pub reason: String,
}
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SocialArchiveBundle {
    pub schema: String,
    pub state: SocialState,
    pub bindings: Vec<ArchiveBinding>,
    pub pending: Vec<PendingEnvelope>,
    #[serde(default)]
    pub binding_history: Vec<BindingHistory>,
}

/// Parse original bytes so duplicate keys cannot be hidden by a host JSON decoder.
pub fn parse(bytes: &[u8]) -> Result<PortableArchive> {
    let archive = crate::strict_parse(bytes)?;
    validate(&archive)?;
    Ok(archive)
}
/// Produce a validated envelope without changing embedded signed strings or asset bytes.
/// JSON whitespace/key ordering may change; this is not canonicalization for signing.
pub fn roundtrip(bytes: &[u8]) -> Result<String> {
    let json = serde_json::to_string(&parse(bytes)?)?;
    if json.len() > crate::MAX_JSON_BYTES {
        return Err(Error::Invalid("Portable export exceeds 128 MiB".into()));
    }
    Ok(json)
}
pub fn validate(archive: &PortableArchive) -> Result<()> {
    if archive.schema != SCHEMA {
        return Err(Error::Invalid("Unsupported export schema".into()));
    }
    crate::validate(&archive.state)?;
    if let Some(social) = &archive.social {
        validate_social_bundle(social)?;
    }
    // Keep v1 compatibility: unreferenced asset entries are retained, not interpreted.
    // Every referenced asset must be present and match its original content checksum.
    for asset in archive.state.editions.values().flat_map(|e| &e.assets) {
        let encoded = archive
            .assets
            .get(&asset.id)
            .ok_or_else(|| Error::Invalid("Export is missing an asset".into()))?;
        let bytes = STANDARD
            .decode(encoded)
            .map_err(|_| Error::Invalid("Invalid export asset base64".into()))?;
        if crate::digest(&bytes) != asset.sha256 {
            return Err(Error::Invalid("Export asset checksum mismatch".into()));
        }
    }
    Ok(())
}
fn sqlite_integer(value: u64) -> Result<()> {
    i64::try_from(value)
        .map(|_| ())
        .map_err(|_| Error::Invalid("Version exceeds SQLite range".into()))
}
/// Structural/integrity checks only. Importing proofs cannot install issuer pins, accept
/// a transport binding, authorize publication, or bypass later replay/membership checks.
pub fn validate_social_bundle(value: &serde_json::Value) -> Result<()> {
    let bundle: SocialArchiveBundle = crate::strict_parse(&crate::canonical(value)?)?;
    if bundle.schema != SOCIAL_SCHEMA
        || bundle.bindings.len() > 10_000
        || bundle.pending.len() > 1000
        || bundle.binding_history.len() > 20_000
    {
        return Err(Error::Invalid("Invalid social archive bundle".into()));
    }
    social::validate(&bundle.state)?;
    let mut holders = BTreeSet::new();
    let mut ids = BTreeSet::new();
    for b in bundle.bindings {
        if !holders.insert(b.document.binding.castalia_holder.clone())
            || b.document.schema != "booknook.transport-identity.v1"
            || b.document.binding.schema != "booknook.nostr-binding.v1"
            || b.document.binding.castalia_holder.len() != 64
            || b.minimum_version == 0
            || b.document.binding.version == 0
            || b.document.request_json.len() > 262144
            || b.document.presentation_json.len() > 16384
        {
            return Err(Error::Invalid("Invalid archived transport identity".into()));
        }
        sqlite_integer(b.minimum_version)?;
        sqlite_integer(b.document.binding.version)?;
        PublicKey::from_hex(&b.document.binding.nostr_public_key)
            .map_err(|_| Error::Invalid("Invalid archived Nostr key".into()))?;
    }
    let mut versions = BTreeSet::new();
    for h in bundle.binding_history {
        if !versions.insert((
            h.document.binding.castalia_holder.clone(),
            h.document.binding.version,
        )) || h.document.schema != "booknook.transport-identity.v1"
            || h.document.binding.schema != "booknook.nostr-binding.v1"
            || h.document.binding.version == 0
            || h.document.request_json.len() > 262144
            || h.document.presentation_json.len() > 16384
        {
            return Err(Error::Invalid("Invalid binding history".into()));
        }
        sqlite_integer(h.document.binding.version)?;
        if let Some(retired) = h.retired_at {
            sqlite_integer(retired)?;
        }
    }
    for p in bundle.pending {
        if !ids.insert(p.id.clone()) || p.event_json.len() > 300_000 || p.reason.len() > 4000 {
            return Err(Error::Invalid("Invalid pending relay event".into()));
        }
        let event = Event::from_json(p.event_json)
            .map_err(|_| Error::Invalid("Invalid archived relay event".into()))?;
        if event.id.to_hex() != p.id {
            return Err(Error::Invalid("Archived relay event ID mismatch".into()));
        }
        event
            .verify()
            .map_err(|_| Error::Denied("Forged archived Nostr event".into()))?;
    }
    Ok(())
}
