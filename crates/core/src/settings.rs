//! Privileged, user-edited connection configuration. Secrets live in native secure storage.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CastaliaSettings {
    pub issuer: String,
    pub key_id: String,
    pub issuer_public_key: String,
    pub audience: String,
    pub terminal_caller: String,
    pub holder_public_key: String,
    #[serde(default)]
    pub policy_digest: String,
    #[serde(default)]
    pub credential_endpoint: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectionSettings {
    pub schema: String,
    pub castalia: Option<CastaliaSettings>,
    #[serde(default)]
    pub castalia_profile_name: Option<String>,
    #[serde(default)]
    pub provider_connections: Vec<crate::castalia_connections::ProviderConnection>,
    pub nostr_relays: Vec<String>,
    pub nostr_public_key: Option<String>,
    pub audible_marketplace: String,
    pub audible_test_asin: String,
}
impl Default for ConnectionSettings {
    fn default() -> Self {
        Self {
            schema: "booknook.connections.v1".into(),
            castalia: None,
            castalia_profile_name: None,
            provider_connections: vec![],
            nostr_relays: vec![],
            nostr_public_key: None,
            audible_marketplace: "us".into(),
            audible_test_asin: String::new(),
        }
    }
}
fn key(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn display(s: &str) -> bool {
    !s.is_empty() && s.len() <= 200 && s.bytes().all(|b| (0x20..=0x7e).contains(&b))
}
pub fn validate(settings: &ConnectionSettings) -> Result<()> {
    let invalid = |m: &str| Error::Invalid(m.into());
    if settings.schema != "booknook.connections.v1" {
        return Err(invalid("Unsupported connections schema"));
    }
    if let Some(c) = &settings.castalia
        && (![&c.issuer, &c.key_id, &c.audience, &c.terminal_caller]
            .iter()
            .all(|s| display(s))
            || !key(&c.issuer_public_key)
            || !key(&c.holder_public_key))
    {
        return Err(invalid(
            "Castalia requires issuer, key ID, audience, caller, and lowercase 32-byte public keys",
        ));
    }
    if let Some(c) = &settings.castalia {
        if !c.policy_digest.is_empty() && !key(&c.policy_digest) {
            return Err(invalid("Policy digest must be lowercase 32-byte hex"));
        }
        if !c.credential_endpoint.is_empty() {
            validate_service_endpoint(&c.credential_endpoint)?;
        }
    }
    if settings
        .castalia_profile_name
        .as_ref()
        .is_some_and(|s| s.is_empty() || s.len() > 160 || s.chars().any(char::is_control))
    {
        return Err(invalid("Invalid Castalia profile display name"));
    }
    crate::castalia_connections::validate_connections(&settings.provider_connections)?;
    if settings.nostr_public_key.as_ref().is_some_and(|s| !key(s)) {
        return Err(invalid("Nostr public key must be lowercase 32-byte hex"));
    }
    if settings.nostr_relays.len() > 20 {
        return Err(invalid("At most 20 Nostr relays"));
    }
    for relay in &settings.nostr_relays {
        let url = url::Url::parse(relay).map_err(|_| invalid("Invalid relay URL"))?;
        if url.scheme() != "wss"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(invalid(
                "Relays require wss URLs without embedded credentials or fragments",
            ));
        }
    }
    if !["us", "uk", "de", "ca", "au"].contains(&settings.audible_marketplace.as_str()) {
        return Err(invalid("Unsupported Audible marketplace"));
    }
    if !settings.audible_test_asin.is_empty()
        && (settings.audible_test_asin.len() != 10
            || !settings
                .audible_test_asin
                .bytes()
                .all(|b| b.is_ascii_alphanumeric()))
    {
        return Err(invalid("Audible test ASIN must contain 10 letters/digits"));
    }
    Ok(())
}

/// Endpoint trust comes from user configuration. Cleartext is restricted to exact loopback.
pub fn validate_service_endpoint(value: &str) -> Result<()> {
    let url =
        url::Url::parse(value).map_err(|_| Error::Invalid("Invalid service endpoint".into()))?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(url.scheme() == "https" || url.scheme() == "http" && local)
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.query().is_some()
    {
        return Err(Error::Invalid("Service endpoints require HTTPS (HTTP only on exact loopback), without credentials, query or fragment".into()));
    }
    Ok(())
}
