// SPDX-License-Identifier: MIT OR Apache-2.0
//! Optional paired library companion client. Amazon protocols are not assumed to be public.
//! Providers report observed source times; transport time is never substituted for progress time.
use booknook_core::{
    Error, Result,
    castalia_connections::{ProviderConnection, validate_connections},
    imports::KindleCatalog,
    strict_parse,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompanionManifest {
    pub schema: String,
    pub name: String,
    pub capabilities: Vec<String>,
    pub qualified: bool,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairingRequest {
    pub schema: String,
    pub code: String,
    pub device_name: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairingResponse {
    pub schema: String,
    pub token: String,
    pub manifest: CompanionManifest,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PositionBatch {
    pub schema: String,
    pub observations: Vec<AudioObservation>,
}
pub use booknook_core::audio::AudioObservation;
fn valid_asin(s: &str) -> bool {
    s.len() == 10 && s.bytes().all(|b| b.is_ascii_alphanumeric())
}
fn valid_token(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn validate_marketplace(marketplace: &str) -> Result<()> {
    if !matches!(marketplace, "us" | "uk" | "de" | "ca" | "au") {
        return Err(Error::Invalid("Unsupported Audible marketplace".into()));
    }
    Ok(())
}

fn source_error(status: u16, body: &[u8]) -> Error {
    let value: Option<serde_json::Value> = if body.len() <= 16_384 {
        strict_parse(body).ok()
    } else {
        None
    };
    let code = value
        .as_ref()
        .and_then(|v| v.get("code"))
        .and_then(|v| v.as_str());
    let message = match code {
        Some("source_not_configured") => "No live source adapter is installed on the companion. Local Kindle catalog import is available.".into(),
        Some("capability_unavailable") => "Source adapter does not support this operation; refresh capabilities.".into(),
        Some("source_timeout") => "Source adapter timed out; check it and retry.".into(),
        Some("source_failure" | "renewal_failed") => "Source account access failed; renew access or reconnect the source account.".into(),
        Some("invalid_source_data") => "Invalid source data; check the selected export or adapter.".into(),
        _ if status == 401 || status == 403 => "Companion access expired or denied; pair again.".into(),
        _ => format!("Companion HTTP {status}; check the configured source adapter."),
    };
    if status == 401 || status == 403 {
        Error::Denied(message)
    } else {
        Error::Unavailable(message)
    }
}

pub struct CompanionClient {
    client: reqwest::Client,
    endpoint: String,
}
impl CompanionClient {
    pub fn new(endpoint: &str) -> Result<Self> {
        booknook_core::settings::validate_service_endpoint(endpoint)?;
        Ok(Self {
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(25))
                .build()
                .map_err(|_| Error::Unavailable("HTTP client unavailable".into()))?,
            endpoint: endpoint.trim_end_matches('/').into(),
        })
    }
    async fn receive<T: serde::de::DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
        limit: usize,
    ) -> Result<T> {
        let mut response = request
            .send()
            .await
            .map_err(|_| Error::Unavailable("Companion is unavailable".into()))?;
        let status = response.status();
        let limit = if status.is_success() {
            limit
        } else {
            limit.min(16_384)
        };
        if response.content_length().is_some_and(|n| n > limit as u64) {
            return Err(if status.is_success() {
                Error::Invalid("Companion response too large".into())
            } else {
                source_error(status.as_u16(), &[])
            });
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| Error::Unavailable("Companion response interrupted".into()))?
        {
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err(if status.is_success() {
                    Error::Invalid("Companion response too large".into())
                } else {
                    source_error(status.as_u16(), &[])
                });
            }
            bytes.extend(chunk);
        }
        if !status.is_success() {
            return Err(source_error(status.as_u16(), &bytes));
        }
        strict_parse(&bytes)
    }
    fn authorized(
        &self,
        method: reqwest::Method,
        path: &str,
        token: &str,
    ) -> Result<reqwest::RequestBuilder> {
        if !valid_token(token) {
            return Err(Error::Denied("Invalid companion device token".into()));
        }
        Ok(self
            .client
            .request(method, format!("{}/v1/{path}", self.endpoint))
            .bearer_auth(token))
    }
    pub async fn pair(&self, code: &str, device_name: &str) -> Result<PairingResponse> {
        if !(12..=128).contains(&code.len()) || device_name.is_empty() || device_name.len() > 160 {
            return Err(Error::Invalid("Invalid pairing input".into()));
        }
        let result: PairingResponse = self
            .receive(
                self.client
                    .post(format!("{}/v1/pair", self.endpoint))
                    .json(&PairingRequest {
                        schema: "booknook.companion-pair.v1".into(),
                        code: code.into(),
                        device_name: device_name.into(),
                    }),
                16384,
            )
            .await?;
        if result.schema != "booknook.companion-paired.v1"
            || result.manifest.schema != "booknook.companion.v1"
            || !valid_token(&result.token)
        {
            return Err(Error::Invalid(
                "Unsupported companion pairing response".into(),
            ));
        }
        Ok(result)
    }
    pub async fn capabilities(&self, token: &str) -> Result<CompanionManifest> {
        let manifest: CompanionManifest = self
            .receive(
                self.authorized(reqwest::Method::GET, "capabilities", token)?,
                16384,
            )
            .await?;
        if manifest.schema != "booknook.companion.v1" {
            return Err(Error::Invalid("Unsupported companion contract".into()));
        }
        Ok(manifest)
    }
    /// Public metadata for this paired device and marketplace. References are
    /// not credentials, grants, or evidence that a live account was qualified.
    pub async fn provider_connections(
        &self,
        token: &str,
        marketplace: &str,
    ) -> Result<Vec<ProviderConnection>> {
        validate_marketplace(marketplace)?;
        let connections: Vec<ProviderConnection> = self
            .receive(
                self.authorized(
                    reqwest::Method::GET,
                    &format!("provider-connections/{marketplace}"),
                    token,
                )?,
                1024 * 1024,
            )
            .await
            .map_err(|error| match error {
                Error::Json(_) => Error::Invalid("Invalid provider connection contract".into()),
                other => other,
            })?;
        validate_connections(&connections)?;
        Ok(connections)
    }
    pub async fn kindle_catalog(&self, token: &str) -> Result<KindleCatalog> {
        let catalog: KindleCatalog = self
            .receive(
                self.authorized(reqwest::Method::GET, "kindle/catalog", token)?,
                32 * 1024 * 1024,
            )
            .await?;
        if catalog.schema != "booknook.kindle-catalog.v1"
            || catalog
                .items
                .iter()
                .any(|i| !valid_asin(&i.asin) || i.title.trim().is_empty())
        {
            return Err(Error::Invalid("Invalid Kindle catalog".into()));
        }
        Ok(catalog)
    }
    /// Compatibility method: preserves the original `{asins}` request. The
    /// companion selects its legacy US default; use the explicit method elsewhere.
    pub async fn audible_positions(
        &self,
        token: &str,
        asins: &[String],
        now: u64,
    ) -> Result<Vec<AudioObservation>> {
        self.request_positions(token, asins, None, now).await
    }
    /// Read source observations using the paired account in the chosen marketplace.
    /// The built-in account-aware adapter selects this marketplace. Legacy adapters
    /// retain their existing account-selection contract at the companion boundary.
    pub async fn audible_positions_in_marketplace(
        &self,
        token: &str,
        marketplace: &str,
        asins: &[String],
        now: u64,
    ) -> Result<Vec<AudioObservation>> {
        validate_marketplace(marketplace)?;
        self.request_positions(token, asins, Some(marketplace), now)
            .await
    }
    async fn request_positions(
        &self,
        token: &str,
        asins: &[String],
        marketplace: Option<&str>,
        now: u64,
    ) -> Result<Vec<AudioObservation>> {
        if asins.is_empty() || asins.len() > 100 || asins.iter().any(|s| !valid_asin(s)) {
            return Err(Error::Invalid("Expected 1–100 ASINs".into()));
        }
        let body = match marketplace {
            Some(marketplace) => serde_json::json!({"asins":asins,"marketplace":marketplace}),
            None => serde_json::json!({"asins":asins}),
        };
        let batch: PositionBatch = self
            .receive(
                self.authorized(reqwest::Method::POST, "audible/positions", token)?
                    .json(&body),
                4 * 1024 * 1024,
            )
            .await?;
        if batch.schema != "booknook.audible-positions.v1" {
            return Err(Error::Invalid("Unsupported position contract".into()));
        }
        for observation in &batch.observations {
            observation.validate(asins, now)?;
        }
        Ok(batch.observations)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use booknook_core::model::Alignment;
    #[test]
    fn stale_audio_never_implies_ebook_progress() {
        let p = AudioObservation {
            asin: "B012345678".into(),
            position_milliseconds: 123_000,
            source_updated_at: 100,
            fetched_at: 2000,
            duration_milliseconds: Some(300_000),
            edition_identifier: None,
            alignment: "unmatched".into(),
            metadata: None,
        };
        p.validate(std::slice::from_ref(&p.asin), 2000).unwrap();
        assert!(p.stale(2000));
        let observation = p.attach("pass");
        assert_eq!(observation.observed_at, 100);
        assert_eq!(observation.received_at, 2000);
        assert_eq!(observation.alignment, Alignment::Unmatched);
        let mut bad = p.clone();
        bad.source_updated_at = 4000;
        assert!(bad.validate(std::slice::from_ref(&bad.asin), 2000).is_err());
        let mut bad = p;
        bad.alignment = "confirmed".into();
        assert!(bad.validate(std::slice::from_ref(&bad.asin), 2000).is_err());
    }
    #[test]
    fn source_errors_use_bounded_codes_without_echoing_account_material() {
        let missing = source_error(503, br#"{"code":"source_not_configured","error":"private account token must not be echoed"}"#).to_string();
        assert!(missing.contains("No live source adapter"));
        assert!(!missing.contains("private account"));
        assert!(
            !source_error(500, br#"{"error":"private account token"}"#)
                .to_string()
                .contains("private account")
        );
        assert!(source_error(401, &[]).to_string().contains("pair again"));
        assert!(
            source_error(503, &vec![b'x'; 20000])
                .to_string()
                .contains("HTTP 503")
        );
    }
    #[test]
    fn remote_cleartext_or_embedded_credentials_are_rejected() {
        for url in [
            "http://example.org",
            "https://user@example.org",
            "https://example.org?token=secret",
        ] {
            assert!(CompanionClient::new(url).is_err());
        }
        assert!(CompanionClient::new("http://127.0.0.1:8765").is_ok());
    }
}
