//! Optional native adapters. These are never required for the offline desk.
pub mod amazon;
pub mod nostr_transport;
use booknook_core::{
    Error, Result,
    authority::{self, AuthorityPolicy},
    model::{Activity, State},
};
use nostr_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub use booknook_core::portable_archive::KeyBinding;
pub struct VerifiedBinding {
    binding: KeyBinding,
}
impl VerifiedBinding {
    pub fn verify(
        binding: KeyBinding,
        request: &[u8],
        presentation: &[u8],
        policy: &AuthorityPolicy,
        now: u64,
    ) -> Result<Self> {
        if binding.schema != "booknook.nostr-binding.v1"
            || binding.castalia_holder != policy.actor
            || policy.action != "nostr.bind"
            || binding.valid_from > now
            || binding.valid_until <= now
        {
            return Err(Error::Denied("Invalid Nostr binding".into()));
        }
        PublicKey::from_hex(&binding.nostr_public_key)
            .map_err(|_| Error::Invalid("Invalid Nostr key".into()))?;
        authority::verify_receipt(
            request,
            presentation,
            policy,
            &booknook_core::canonical(&binding)?,
            binding.version,
            now,
        )?;
        Ok(Self { binding })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SocialEnvelope {
    pub schema: String,
    pub activity: Activity,
    pub request: serde_json::Value,
    pub presentation: serde_json::Value,
}

/// Relay signatures prove transport authorship; Castalia independently authorizes the action.
pub fn accept_event(
    state: &mut State,
    event: &Event,
    binding: &VerifiedBinding,
    policy: &AuthorityPolicy,
    now: u64,
) -> Result<bool> {
    event
        .verify()
        .map_err(|_| Error::Denied("Forged Nostr event".into()))?;
    if event.kind != Kind::Custom(30078)
        || event.pubkey.to_hex() != binding.binding.nostr_public_key
        || event.created_at.as_secs() < binding.binding.valid_from
        || event.created_at.as_secs() >= binding.binding.valid_until
    {
        return Err(Error::Denied(
            "Transport key binding mismatch/expiry".into(),
        ));
    }
    if event.content.len() > 262144 {
        return Err(Error::Invalid("Oversized social event".into()));
    }
    let envelope: SocialEnvelope = booknook_core::strict_parse(event.content.as_bytes())?;
    if envelope.schema != "booknook.social-envelope.v1"
        || envelope.activity.actor != binding.binding.castalia_holder
        || event.created_at.as_secs() != envelope.activity.published_at
        || event.tags.identifier()
            != Some(format!("booknook:activity:{}", envelope.activity.id).as_str())
    {
        return Err(Error::Denied("Activity envelope binding mismatch".into()));
    }
    // Historical publications carry evidence of approval at publication time; current
    // operator pins and group membership are still rechecked on every receive.
    let auth = authority::verify_receipt(
        &booknook_core::canonical(&envelope.request)?,
        &booknook_core::canonical(&envelope.presentation)?,
        policy,
        &booknook_core::canonical(&envelope.activity)?,
        envelope.activity.group_epoch.unwrap_or(0),
        event.created_at.as_secs(),
    )?;
    booknook_core::social::receive(state, envelope.activity, &auth, now)
}

pub struct NostrRelayTransport {
    client: Client,
}
impl NostrRelayTransport {
    pub async fn connect(relays: &[String]) -> Result<Self> {
        if relays.is_empty() {
            return Err(Error::Unavailable("No Nostr relays configured".into()));
        }
        let client = Client::default();
        for relay in relays {
            let url =
                RelayUrl::parse(relay).map_err(|_| Error::Invalid("Invalid relay URL".into()))?;
            if !relay.starts_with("wss://") {
                return Err(Error::Invalid("Remote relays require wss".into()));
            }
            client
                .add_relay(url)
                .await
                .map_err(|e| Error::Unavailable(e.to_string()))?;
        }
        client.connect().await;
        Ok(Self { client })
    }
    /// Already-signed, authorized public events only. No key is created by this adapter.
    pub async fn publish(
        &self,
        event: &Event,
        binding: &VerifiedBinding,
        policy: &AuthorityPolicy,
        state: &State,
        now: u64,
    ) -> Result<String> {
        let mut check = state.clone();
        accept_event(&mut check, event, binding, policy, now)?;
        let result = self
            .client
            .send_event(event)
            .await
            .map_err(|e| Error::Unavailable(e.to_string()))?;
        if result.success.is_empty() {
            return Err(Error::Unavailable(
                "No relay acknowledged publication; retain the outbox item".into(),
            ));
        }
        Ok(event.id.to_hex())
    }
    pub async fn fetch(&self, authors: Vec<PublicKey>, since: u64) -> Result<Vec<Event>> {
        if authors.is_empty() {
            return Ok(vec![]);
        }
        let filter = Filter::new()
            .kind(Kind::Custom(30078))
            .authors(authors)
            .since(Timestamp::from(since))
            .limit(500);
        let events = self
            .client
            .fetch_events(filter, Duration::from_secs(10))
            .await
            .map_err(|e| Error::Unavailable(e.to_string()))?;
        Ok(events.into_iter().collect())
    }
}

/// Account authentication is supplied by a separately qualified local credential provider.
/// No password flow, remote credential proxy, or claim of supported public Audible API.
pub struct AudiblePositionConnector {
    client: reqwest::Client,
    origin: String,
}
impl AudiblePositionConnector {
    pub fn new(marketplace: &str) -> Result<Self> {
        let host = match marketplace {
            "us" => "api.audible.com",
            "uk" => "api.audible.co.uk",
            "de" => "api.audible.de",
            "ca" => "api.audible.ca",
            "au" => "api.audible.com.au",
            _ => return Err(Error::Invalid("Unqualified Audible marketplace".into())),
        };
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|e| Error::Unavailable(e.to_string()))?;
        Ok(Self {
            client,
            origin: format!("https://{host}"),
        })
    }
    /// Returns source evidence, not fabricated normalized observations. The server response
    /// shape and timestamp semantics must be qualified before producing ProgressObservation.
    pub async fn lookup(
        &self,
        asins: &[String],
        headers: reqwest::header::HeaderMap,
    ) -> Result<serde_json::Value> {
        if asins.is_empty()
            || asins.len() > 100
            || asins
                .iter()
                .any(|id| id.len() != 10 || !id.bytes().all(|b| b.is_ascii_alphanumeric()))
        {
            return Err(Error::Invalid("Expected 1–100 ASINs".into()));
        }
        let response = self
            .client
            .get(format!("{}/1.0/annotations/lastpositions", self.origin))
            .query(&[("asins", asins.join(","))])
            .headers(headers)
            .send()
            .await
            .map_err(|_| Error::Unavailable("Audible position request failed".into()))?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED
            || response.status() == reqwest::StatusCode::FORBIDDEN
        {
            return Err(Error::Denied(
                "Audible connector access expired or denied".into(),
            ));
        }
        if !response.status().is_success() {
            return Err(Error::Unavailable(format!(
                "Audible returned HTTP {}",
                response.status().as_u16()
            )));
        }
        if response
            .content_length()
            .is_some_and(|size| size > 4 * 1024 * 1024)
        {
            return Err(Error::Invalid("Audible response too large".into()));
        }
        let mut response = response;
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| Error::Unavailable("Audible response interrupted".into()))?
        {
            if bytes.len() + chunk.len() > 4 * 1024 * 1024 {
                return Err(Error::Invalid("Audible response too large".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        booknook_core::strict_parse(&bytes)
    }
}
