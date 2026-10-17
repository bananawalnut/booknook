// SPDX-License-Identifier: MIT OR Apache-2.0
use booknook_connectors::{KeyBinding, SocialEnvelope, VerifiedBinding, accept_event};
use booknook_core::{
    authority::AuthorityPolicy,
    canonical,
    model::{Activity, State},
};
use ed25519_dalek::SigningKey;
use nostr_sdk::prelude::*;
#[path = "fixtures/receipts.rs"]
mod fixtures;

fn setup() -> (Event, VerifiedBinding, AuthorityPolicy) {
    let transport = Keys::parse(&"03".repeat(32)).unwrap();
    let holder = hex::encode(SigningKey::from_bytes(&[2; 32]).verifying_key().as_bytes());
    let binding = KeyBinding {
        schema: "booknook.nostr-binding.v1".into(),
        castalia_holder: holder.clone(),
        nostr_public_key: transport.public_key().to_hex(),
        valid_from: 100,
        valid_until: 500,
        version: 1,
    };
    let (r, p, policy) = fixtures::receipt(&canonical(&binding).unwrap(), "nostr.bind", 1);
    let binding = VerifiedBinding::verify(binding, &r, &p, &policy, 110).unwrap();
    let activity = Activity {
        id: "fixture-activity".into(),
        actor: holder,
        published_at: 110,
        kind: "review".into(),
        book_id: None,
        body: "Only the published selection.".into(),
        spoiler: false,
        group_id: None,
        group_epoch: None,
    };
    let (r, p, policy) = fixtures::receipt(&canonical(&activity).unwrap(), "social.publish", 0);
    let content = serde_json::to_string(&SocialEnvelope {
        schema: "booknook.social-envelope.v1".into(),
        activity,
        request: serde_json::from_slice(&r).unwrap(),
        presentation: serde_json::from_slice(&p).unwrap(),
    })
    .unwrap();
    let event = EventBuilder::new(Kind::Custom(30078), content)
        .tags([Tag::identifier("booknook:activity:fixture-activity")])
        .custom_created_at(Timestamp::from(110))
        .sign_with_keys(&transport)
        .unwrap();
    (event, binding, policy)
}

#[test]
fn verified_relay_event_deduplicates_and_rechecks_current_trust() {
    let (event, binding, policy) = setup();
    let mut state = State::default();
    assert!(accept_event(&mut state, &event, &binding, &policy, 115).unwrap());
    assert!(!accept_event(&mut state, &event, &binding, &policy, 115).unwrap());
    let mut revoked = policy;
    revoked.pins.clear();
    assert!(accept_event(&mut state, &event, &binding, &revoked, 115).is_err());
    assert_eq!(state.activity.len(), 1);
}

#[test]
fn transport_signature_does_not_replace_castalia_identity_binding() {
    let (event, binding, policy) = setup();
    let impostor = Keys::parse(&"04".repeat(32)).unwrap();
    let forged = EventBuilder::new(event.kind, event.content.clone())
        .tags(event.tags.clone())
        .custom_created_at(event.created_at)
        .sign_with_keys(&impostor)
        .unwrap();
    let mut state = State::default();
    assert!(forged.verify().is_ok());
    assert!(accept_event(&mut state, &forged, &binding, &policy, 115).is_err());
    let mut tampered = event.clone();
    tampered.content.push(' ');
    assert!(accept_event(&mut state, &tampered, &binding, &policy, 115).is_err());
    assert!(state.activity.is_empty());
}

#[test]
fn unsigned_or_changed_binding_is_rejected() {
    let transport = Keys::parse(&"03".repeat(32)).unwrap();
    let mut binding = KeyBinding {
        schema: "booknook.nostr-binding.v1".into(),
        castalia_holder: hex::encode(SigningKey::from_bytes(&[2; 32]).verifying_key().as_bytes()),
        nostr_public_key: transport.public_key().to_hex(),
        valid_from: 100,
        valid_until: 500,
        version: 1,
    };
    let (r, p, policy) = fixtures::receipt(&canonical(&binding).unwrap(), "nostr.bind", 1);
    binding.nostr_public_key = Keys::parse(&"05".repeat(32)).unwrap().public_key().to_hex();
    assert!(VerifiedBinding::verify(binding, &r, &p, &policy, 110).is_err());
}
