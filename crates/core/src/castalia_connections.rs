// SPDX-License-Identifier: MIT OR Apache-2.0
//! Public Castalia connection metadata. Validation does not establish trust or grant authority.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum NativeAuthorityProfileSchema {
    #[serde(rename = "castalia.native-authority-profile.v1")]
    V1,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum ProviderConnectionSchema {
    #[serde(rename = "castalia.provider-connection.v1")]
    V1,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeAuthorityProfile {
    pub schema: NativeAuthorityProfileSchema,
    #[schemars(extend("minLength" = 1, "maxLength" = 160, "pattern" = r"^[^\u0000-\u001f\u007f-\u009f\u061c\u200e\u200f\u202a-\u202e\u2066-\u2069]+$"))]
    pub display_name: String,
    #[schemars(extend("minLength" = 1, "maxLength" = 200, "pattern" = r"^[!-~](?:[ -~]*[!-~])?$"))]
    pub issuer: String,
    #[schemars(extend("minLength" = 1, "maxLength" = 200, "pattern" = r"^[!-~](?:[ -~]*[!-~])?$"))]
    pub key_id: String,
    #[schemars(extend("pattern" = r"^[a-f0-9]{64}$"))]
    pub issuer_public_key: String,
    #[schemars(extend("minLength" = 1, "maxLength" = 200, "pattern" = r"^[!-~](?:[ -~]*[!-~])?$"))]
    pub audience: String,
    pub caller: NativeCaller,
    #[schemars(extend("pattern" = r"^[a-f0-9]{64}$"))]
    pub policy_digest_sha256: String,
    /// Canonical HTTPS or exact-loopback HTTP. Full URL and UTF-8 byte limits are checked by the public runtime validator.
    #[schemars(extend("minLength" = 1, "maxLength" = 2048, "format" = "uri", "pattern" = r"^https?://[^?#]+$"))]
    pub credential_endpoint: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeCaller {
    pub kind: NativeCallerKind,
    #[schemars(extend("minLength" = 1, "maxLength" = 200, "pattern" = r"^[!-~](?:[ -~]*[!-~])?$"))]
    pub id: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NativeCallerKind {
    Terminal,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["schema", "connection_id", "provider_id", "display_name", "capabilities", "status", "credential"]))]
pub struct ProviderConnection {
    pub schema: ProviderConnectionSchema,
    #[schemars(extend("pattern" = r"^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$"))]
    pub connection_id: String,
    #[schemars(extend("pattern" = r"^[a-z0-9][a-z0-9._:-]{0,127}$"))]
    pub provider_id: String,
    #[schemars(extend("minLength" = 1, "maxLength" = 160, "pattern" = r"^[^\u0000-\u001f\u007f-\u009f\u061c\u200e\u200f\u202a-\u202e\u2066-\u2069]+$"))]
    pub display_name: String,
    #[schemars(extend("maxItems" = 64, "uniqueItems" = true, "items" = {"type": "string", "pattern": "^[a-z0-9][a-z0-9._:-]{0,127}$"}))]
    pub capabilities: Vec<String>,
    pub status: ConnectionStatus,
    #[serde(deserialize_with = "required_option")]
    pub credential: Option<CredentialReference>,
}
fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> std::result::Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionStatus {
    Available,
    ReauthorizationRequired,
    Unavailable,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CredentialReference {
    #[schemars(extend("pattern" = r"^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$"))]
    pub reference: String,
    pub storage: CredentialStorage,
    pub portability: CredentialPortability,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CredentialStorage {
    SecureLocal,
    WalletManaged,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CredentialPortability {
    DeviceBound,
    ReauthorizationRequired,
    ProviderDefined,
}

pub fn valid_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn valid_reference(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
}
fn namespace(value: &str) -> bool {
    valid_reference(value) && !value.bytes().any(|b| b.is_ascii_uppercase())
}
fn label(value: &str) -> bool {
    !value.is_empty() && value.len() <= 160 && !value.chars().any(|c| c.is_control() || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value.trim() == value
        && value.bytes().all(|b| (0x20..=0x7e).contains(&b))
}
fn invalid(message: &str) -> Error {
    Error::Invalid(message.into())
}
impl NativeAuthorityProfile {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        if raw.len() > 8192 {
            return Err(invalid("Castalia authority profile exceeds 8192 bytes"));
        }
        let value: Self = crate::strict_parse(raw)?;
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        if !label(&self.display_name)
            || ![&self.issuer, &self.key_id, &self.audience, &self.caller.id]
                .into_iter()
                .all(|s| identifier(s))
            || !valid_hex(&self.issuer_public_key)
            || !valid_hex(&self.policy_digest_sha256)
        {
            return Err(invalid(
                "Invalid public Castalia authority profile identity, label, or digest",
            ));
        }
        let endpoint = &self.credential_endpoint;
        if endpoint.len() > 2048 || !endpoint.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
            return Err(invalid("Invalid Castalia authority endpoint"));
        }
        crate::settings::validate_service_endpoint(endpoint)?;
        let url = url::Url::parse(endpoint)
            .map_err(|_| invalid("Invalid Castalia authority endpoint"))?;
        if url.as_str() != endpoint
            || url
                .host_str()
                .is_some_and(|h| h.ends_with('.') || h.contains('*'))
            || url.port() == Some(0)
        {
            return Err(invalid(
                "Castalia authority endpoint must be canonical without wildcard/trailing-dot host or port zero",
            ));
        }
        if crate::canonical(self)?.len() > 8192 {
            return Err(invalid("Castalia authority profile exceeds 8192 bytes"));
        }
        Ok(())
    }
}
impl ProviderConnection {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        if raw.len() > 16384 {
            return Err(invalid("Castalia provider connection exceeds 16384 bytes"));
        }
        let value: Self = crate::strict_parse(raw)?;
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        if !valid_reference(&self.connection_id)
            || !namespace(&self.provider_id)
            || !label(&self.display_name)
            || self.capabilities.len() > 64
            || self.capabilities.iter().any(|s| !namespace(s))
            || self.capabilities.iter().collect::<BTreeSet<_>>().len() != self.capabilities.len()
            || self
                .credential
                .as_ref()
                .is_some_and(|c| !valid_reference(&c.reference))
        {
            return Err(invalid("Invalid public Castalia provider connection"));
        }
        if crate::canonical(self)?.len() > 16384 {
            return Err(invalid("Castalia provider connection exceeds 16384 bytes"));
        }
        Ok(())
    }
}
pub fn validate_connections(connections: &[ProviderConnection]) -> Result<()> {
    if connections.len() > 64
        || connections
            .iter()
            .map(|c| &c.connection_id)
            .collect::<BTreeSet<_>>()
            .len()
            != connections.len()
    {
        return Err(invalid(
            "Use at most 64 uniquely identified provider connections",
        ));
    }
    for connection in connections {
        connection.validate()?;
    }
    Ok(())
}

pub fn parse_connections(raw: &[u8]) -> Result<Vec<ProviderConnection>> {
    if raw.len() > 1024 * 1024 {
        return Err(invalid("Provider registry exceeds 1 MiB"));
    }
    let connections: Vec<ProviderConnection> = crate::strict_parse(raw)?;
    validate_connections(&connections)?;
    Ok(connections)
}
