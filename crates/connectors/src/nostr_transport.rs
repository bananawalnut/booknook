// SPDX-License-Identifier: MIT OR Apache-2.0
//! Approved social exchange over Nostr. NIP-44 encrypts each private delivery separately.
//! Custody supplies transient keys; this adapter neither creates nor persists signing keys.
use crate::{KeyBinding, VerifiedBinding};
use booknook_core::{
    Error, Result,
    authority::AuthorityPolicy,
    canonical,
    social_protocol::{self as social, ApprovedPublication, Delivery, SocialState},
};
use nostr_sdk::pool::transport::websocket::{DefaultWebsocketTransport, IntoWebSocketTransport};
use nostr_sdk::{nips::nip44, prelude::*};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

const KIND: Kind = Kind::Custom(30078);
const MAX_WIRE_BYTES: usize = 100_000;
fn denied(message: &str) -> Error {
    Error::Denied(message.into())
}

pub use booknook_core::portable_archive::BindingDocument;
pub fn verify_binding(
    document: &BindingDocument,
    policy: &AuthorityPolicy,
    minimum_version: u64,
    now: u64,
) -> Result<VerifiedBinding> {
    let b = &document.binding;
    if document.schema != "booknook.transport-identity.v1"
        || b.version < minimum_version
        || document.approved_at > now
        || document.approved_at < b.valid_from
        || document.approved_at >= b.valid_until
        || b.valid_from > now
        || b.valid_until <= now
        || b.valid_until - b.valid_from > 366 * 86400
        || document.request_json.len() > 262144
        || document.presentation_json.len() > 16384
    {
        return Err(denied(
            "Transport identity is revoked, expired or malformed",
        ));
    }
    // Validate historical consent against current, independently pinned issuer policy.
    let copy: KeyBinding = booknook_core::strict_parse(&canonical(b)?)?;
    VerifiedBinding::verify(
        copy,
        document.request_json.as_bytes(),
        document.presentation_json.as_bytes(),
        policy,
        document.approved_at,
    )
}
fn binding_at(binding: &VerifiedBinding, at: u64) -> Result<()> {
    if at < binding.binding.valid_from || at >= binding.binding.valid_until {
        return Err(denied("Transport binding expired"));
    }
    Ok(())
}
fn key(binding: &VerifiedBinding) -> Result<PublicKey> {
    PublicKey::from_hex(&binding.binding.nostr_public_key)
        .map_err(|_| denied("Invalid bound Nostr key"))
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrivatePayload {
    schema: String,
    recipient: String,
    sender: String,
    publication: ApprovedPublication,
}

/// Constructed only by encode_publication after authority and current recipients are checked.
#[derive(Debug)]
pub struct PreparedDelivery {
    recipient: String,
    event: Event,
}
impl PreparedDelivery {
    pub fn recipient(&self) -> &str {
        &self.recipient
    }
    pub fn event(&self) -> &Event {
        &self.event
    }
}
/// One public event or one encrypted event per recipient, including the author's own devices.
/// No application content or group identifier is emitted in a private event's relay tags.
pub fn encode_publication(
    state: &SocialState,
    publication: &ApprovedPublication,
    policy: &AuthorityPolicy,
    sender: &VerifiedBinding,
    recipients: &BTreeMap<String, VerifiedBinding>,
    keys: &Keys,
    now: u64,
) -> Result<Vec<PreparedDelivery>> {
    social::validate_outgoing(state, publication, policy, now)?;
    binding_at(sender, now)?;
    if keys.public_key() != key(sender)?
        || sender.binding.castalia_holder != publication.event.actor
    {
        return Err(denied(
            "Custody key does not match the approved Castalia binding",
        ));
    }
    let mut result = Vec::new();
    match &publication.event.delivery {
        Delivery::Public => {
            let event = EventBuilder::new(
                KIND,
                String::from_utf8(canonical(publication)?)
                    .map_err(|_| denied("Invalid publication encoding"))?,
            )
            .tags([
                Tag::identifier(format!("booknook:social:{}:public", publication.event.id)),
                Tag::hashtag("booknook"),
            ])
            .custom_created_at(Timestamp::from(now))
            .sign_with_keys(keys)
            .map_err(|e| Error::Unavailable(e.to_string()))?;
            result.push(PreparedDelivery {
                recipient: "public".into(),
                event,
            });
        }
        Delivery::Direct { recipients: actors } => {
            // Resolve every identity before encryption. Partial recipient sets cannot leak a
            // publication before the complete intended audience has a verified binding.
            for actor in actors {
                let b = recipients.get(actor).ok_or_else(|| {
                    Error::NotFound(format!("Verified transport binding for {actor}"))
                })?;
                binding_at(b, now)?;
                if b.binding.castalia_holder != *actor {
                    return Err(denied("Recipient identity mismatch"));
                }
            }
            for actor in actors {
                let bound = &recipients[actor];
                let public = key(bound)?;
                let plaintext = canonical(&PrivatePayload {
                    schema: "booknook.private-social.v1".into(),
                    recipient: actor.clone(),
                    sender: publication.event.actor.clone(),
                    publication: publication.clone(),
                })?;
                if plaintext.len() > 65_535 {
                    return Err(Error::Invalid(
                        "Private publication exceeds NIP-44 size limit".into(),
                    ));
                }
                let encrypted =
                    nip44::encrypt(keys.secret_key(), &public, plaintext, nip44::Version::V2)
                        .map_err(|_| denied("Private message encryption failed"))?;
                // The opaque digest is stable for relay deduplication and does not contain a
                // literary/group identifier or a Castalia directory identity.
                let address =
                    booknook_core::digest(format!("{}:{}", publication.event.id, actor).as_bytes());
                let event = EventBuilder::new(KIND, encrypted)
                    .allow_self_tagging()
                    .tags([
                        Tag::identifier(format!("booknook:private:{address}")),
                        Tag::public_key(public),
                        Tag::hashtag("booknook"),
                    ])
                    .custom_created_at(Timestamp::from(now))
                    .sign_with_keys(keys)
                    .map_err(|e| Error::Unavailable(e.to_string()))?;
                result.push(PreparedDelivery {
                    recipient: actor.clone(),
                    event,
                });
            }
        }
    }
    Ok(result)
}
/// Decode before applying. Plaintext delivery of an action with a private audience is rejected.
pub fn decode_event(
    event: &Event,
    sender: &VerifiedBinding,
    recipient: Option<(&VerifiedBinding, &Keys)>,
    now: u64,
) -> Result<ApprovedPublication> {
    event.verify().map_err(|_| denied("Forged Nostr event"))?;
    binding_at(sender, event.created_at.as_secs())?;
    if event.kind != KIND
        || event.pubkey != key(sender)?
        || event.created_at.as_secs() > now.saturating_add(60)
        || event.content.len() > MAX_WIRE_BYTES
    {
        return Err(denied("Nostr envelope mismatch"));
    }
    let identifier = event
        .tags
        .identifier()
        .ok_or_else(|| denied("Missing application event identifier"))?;
    let publication = if identifier.starts_with("booknook:social:") {
        let publication: ApprovedPublication =
            booknook_core::strict_parse(event.content.as_bytes())?;
        if publication.event.delivery != Delivery::Public
            || identifier != format!("booknook:social:{}:public", publication.event.id)
            || event.tags.public_keys().next().is_some()
        {
            return Err(denied(
                "Public event contains private content or mismatched tags",
            ));
        }
        publication
    } else if identifier.starts_with("booknook:private:") {
        let (recipient, keys) = recipient
            .ok_or_else(|| denied("Private event requires the recipient's custody key"))?;
        binding_at(recipient, now)?;
        let recipient_key = key(recipient)?;
        if recipient_key != keys.public_key()
            || event.tags.public_keys().collect::<Vec<_>>() != vec![&recipient_key]
        {
            return Err(denied("Wrong encrypted event recipient"));
        }
        let plaintext = nip44::decrypt_to_bytes(keys.secret_key(), &event.pubkey, &event.content)
            .map_err(|_| denied("Private event authentication failed"))?;
        let payload: PrivatePayload = booknook_core::strict_parse(&plaintext)?;
        let expected = format!(
            "booknook:private:{}",
            booknook_core::digest(
                format!("{}:{}", payload.publication.event.id, payload.recipient).as_bytes()
            )
        );
        if payload.schema != "booknook.private-social.v1"
            || payload.recipient != recipient.binding.castalia_holder
            || payload.sender != sender.binding.castalia_holder
            || expected != identifier
            || !matches!(&payload.publication.event.delivery,Delivery::Direct {recipients} if recipients.contains(&payload.recipient))
        {
            return Err(denied("Encrypted audience or identity mismatch"));
        }
        payload.publication
    } else {
        return Err(denied("Not a Book Nook social event"));
    };
    if publication.event.actor != sender.binding.castalia_holder
        || publication.approved_at > event.created_at.as_secs()
    {
        return Err(denied("Castalia author or timestamp mismatch"));
    }
    Ok(publication)
}
pub fn receive_event(
    state: &mut SocialState,
    event: &Event,
    sender: &VerifiedBinding,
    recipient: Option<(&VerifiedBinding, &Keys)>,
    policy: &AuthorityPolicy,
    now: u64,
) -> Result<bool> {
    let publication = decode_event(event, sender, recipient, now)?;
    social::receive(state, publication, policy, now)
}

pub struct SocialRelayTransport {
    client: Client,
}
impl SocialRelayTransport {
    pub async fn connect(relays: &[String]) -> Result<Self> {
        Self::connect_with_transport(relays, DefaultWebsocketTransport).await
    }
    /// Hosts may supply a WebSocket implementation while retaining all relay validation,
    /// event verification and acknowledgment handling. The reference app uses the SDK's
    /// default TLS transport; isolated tests inject a real loopback socket transport.
    pub async fn connect_with_transport<T: IntoWebSocketTransport>(
        relays: &[String],
        transport: T,
    ) -> Result<Self> {
        if relays.is_empty() || relays.len() > 20 {
            return Err(Error::Invalid("Configure 1–20 Nostr relays".into()));
        }
        let client = ClientBuilder::new().websocket_transport(transport).build();
        for relay in relays {
            let url =
                RelayUrl::parse(relay).map_err(|_| Error::Invalid("Invalid relay URL".into()))?;
            if !relay.starts_with("wss://") || url.as_str().contains('@') {
                return Err(Error::Invalid(
                    "Relays require wss without embedded credentials".into(),
                ));
            }
            client
                .add_relay(url)
                .await
                .map_err(|e| Error::Unavailable(e.to_string()))?;
        }
        client.connect().await;
        Ok(Self { client })
    }
    /// Return the actual relays that acknowledged this delivery; the caller commits them to
    /// the durable outbox. An outage retains the item rather than claiming it was published.
    pub async fn publish(&self, delivery: &PreparedDelivery) -> Result<BTreeSet<String>> {
        let result = self
            .client
            .send_event(&delivery.event)
            .await
            .map_err(|e| Error::Unavailable(e.to_string()))?;
        if result.success.is_empty() {
            return Err(Error::Unavailable(
                "No relay acknowledged the publication".into(),
            ));
        }
        Ok(result.success.into_iter().map(|u| u.to_string()).collect())
    }
    /// Data-only transport port. Signing/permission checks belong to the calling protocol.
    pub async fn publish_signed_event(&self, event: &Event) -> Result<BTreeSet<String>> {
        event
            .verify()
            .map_err(|_| Error::Denied("Invalid signed transport event".into()))?;
        if event.as_json().len() > 100_000 {
            return Err(Error::Invalid("Transport event exceeds size limit".into()));
        }
        let result = self
            .client
            .send_event(event)
            .await
            .map_err(|e| Error::Unavailable(e.to_string()))?;
        if result.success.is_empty() {
            return Err(Error::Unavailable("No relay acknowledged the event".into()));
        }
        Ok(result.success.into_iter().map(|u| u.to_string()).collect())
    }
    /// Retrieval is bounded. Every returned event must independently pass receive_event;
    /// relay search/filtering is never an authorization decision.
    pub async fn fetch(
        &self,
        authors: Vec<PublicKey>,
        recipient: Option<PublicKey>,
        since: u64,
    ) -> Result<Vec<Event>> {
        self.fetch_page(authors, recipient, since, None).await
    }
    pub async fn fetch_page(
        &self,
        authors: Vec<PublicKey>,
        recipient: Option<PublicKey>,
        since: u64,
        until: Option<u64>,
    ) -> Result<Vec<Event>> {
        if authors.len() > 200 {
            return Err(Error::Invalid(
                "Fetch at most 200 bound authors at once".into(),
            ));
        }
        let mut filter = Filter::new()
            .kind(KIND)
            .hashtag("booknook")
            .since(Timestamp::from(since))
            .limit(500);
        if let Some(until) = until {
            filter = filter.until(Timestamp::from(until));
        }
        if !authors.is_empty() {
            filter = filter.authors(authors);
        }
        if let Some(recipient) = recipient {
            filter = filter.pubkey(recipient);
        }
        let events = self
            .client
            .fetch_events(filter, Duration::from_secs(10))
            .await
            .map_err(|e| Error::Unavailable(e.to_string()))?;
        Ok(events.into_iter().collect())
    }
    pub async fn disconnect(&self) {
        self.client.disconnect().await;
    }
}

/// Publishing the approved binding makes public-group discovery verifiable against the
/// reader's independently configured Castalia issuer pins. Unknown issuers are never trusted.
pub fn encode_binding(
    document: &BindingDocument,
    policy: &AuthorityPolicy,
    keys: &Keys,
    now: u64,
) -> Result<Event> {
    let verified = verify_binding(document, policy, document.binding.version, now)?;
    if key(&verified)? != keys.public_key() {
        return Err(denied("Binding custody key mismatch"));
    }
    EventBuilder::new(
        KIND,
        String::from_utf8(canonical(document)?).map_err(|_| denied("Invalid identity encoding"))?,
    )
    .tags([
        Tag::identifier(format!(
            "booknook:identity:{}:{}",
            document.binding.castalia_holder, document.binding.version
        )),
        Tag::hashtag("booknook"),
    ])
    .custom_created_at(Timestamp::from(document.approved_at))
    .sign_with_keys(keys)
    .map_err(|e| Error::Unavailable(e.to_string()))
}
pub fn decode_binding(
    event: &Event,
    policy: &AuthorityPolicy,
    minimum_version: u64,
    now: u64,
) -> Result<BindingDocument> {
    event
        .verify()
        .map_err(|_| denied("Forged transport identity event"))?;
    if event.kind != KIND || event.content.len() > 300_000 {
        return Err(denied("Invalid identity event"));
    }
    let doc: BindingDocument = booknook_core::strict_parse(event.content.as_bytes())?;
    let verified = verify_binding(&doc, policy, minimum_version, now)?;
    if event.pubkey != key(&verified)?
        || event.created_at.as_secs() != doc.approved_at
        || event.tags.identifier()
            != Some(
                format!(
                    "booknook:identity:{}:{}",
                    doc.binding.castalia_holder, doc.binding.version
                )
                .as_str(),
            )
    {
        return Err(denied("Transport identity envelope mismatch"));
    }
    Ok(doc)
}
impl SocialRelayTransport {
    pub async fn publish_binding(
        &self,
        document: &BindingDocument,
        policy: &AuthorityPolicy,
        keys: &Keys,
        now: u64,
    ) -> Result<BTreeSet<String>> {
        let event = encode_binding(document, policy, keys, now)?;
        let result = self
            .client
            .send_event(&event)
            .await
            .map_err(|e| Error::Unavailable(e.to_string()))?;
        if result.success.is_empty() {
            return Err(Error::Unavailable(
                "No relay acknowledged the identity binding".into(),
            ));
        }
        Ok(result.success.into_iter().map(|u| u.to_string()).collect())
    }
}
