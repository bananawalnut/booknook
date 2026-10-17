// SPDX-License-Identifier: MIT OR Apache-2.0
use booknook_connectors::{KeyBinding, VerifiedBinding, nostr_transport::*};
use booknook_core::{canonical, social_protocol::*};
use nostr_sdk::prelude::*;
use std::collections::BTreeMap;
#[path = "fixtures/social_receipts.rs"]
mod receipts;
use receipts::{actor, proof, receipt};

fn binding(keys: &Keys, seed: u8) -> VerifiedBinding {
    binding_started(keys, seed, 100)
}
fn binding_started(keys: &Keys, seed: u8, at: u64) -> VerifiedBinding {
    let binding = KeyBinding {
        schema: "booknook.nostr-binding.v1".into(),
        castalia_holder: actor(seed),
        nostr_public_key: keys.public_key().to_hex(),
        valid_from: at,
        valid_until: 1000,
        version: 1,
    };
    let (request_json, presentation_json, policy) =
        receipt(&canonical(&binding).unwrap(), "nostr.bind", 1, at, seed);
    verify_binding(
        &BindingDocument {
            schema: "booknook.transport-identity.v1".into(),
            binding,
            approved_at: at,
            request_json,
            presentation_json,
        },
        &policy,
        1,
        at,
    )
    .unwrap()
}
fn approved(
    action: SocialAction,
) -> (
    SocialState,
    ApprovedPublication,
    booknook_core::authority::AuthorityPolicy,
) {
    let mut state = SocialState::default();
    let event = prepare(&state, &actor(2), action, 110).unwrap();
    let (r, p, policy) = proof(&event, 2);
    let publication = approve(&mut state, event, r, p, &policy, 110).unwrap();
    (state, publication, policy)
}
#[test]
fn private_nip44_round_trip_reveals_no_plaintext_and_binds_exact_recipient() {
    let sender = Keys::parse(&"03".repeat(32)).unwrap();
    let recipient = Keys::parse(&"04".repeat(32)).unwrap();
    let wrong = Keys::parse(&"05".repeat(32)).unwrap();
    let (state, publication, policy) = approved(SocialAction::Friendship {
        peer: actor(3),
        requested: true,
    });
    let mut bindings = BTreeMap::new();
    bindings.insert(actor(2), binding(&sender, 2));
    bindings.insert(actor(3), binding(&recipient, 3));
    let wires = encode_publication(
        &state,
        &publication,
        &policy,
        &bindings[&actor(2)],
        &bindings,
        &sender,
        111,
    )
    .unwrap();
    assert_eq!(wires.len(), 2);
    let own = wires.iter().find(|w| w.recipient() == actor(2)).unwrap();
    assert_eq!(
        own.event().tags.public_keys().collect::<Vec<_>>(),
        vec![&sender.public_key()]
    );
    assert_eq!(
        decode_event(
            own.event(),
            &bindings[&actor(2)],
            Some((&bindings[&actor(2)], &sender)),
            112
        )
        .unwrap(),
        publication
    );
    let target = wires.iter().find(|w| w.recipient() == actor(3)).unwrap();
    assert!(!target.event().content.contains("friendship"));
    assert!(!target.event().content.contains(&actor(2)));
    assert!(!target.event().as_json().contains(&actor(3)));
    assert!(decode_event(target.event(), &bindings[&actor(2)], None, 112).is_err());
    assert!(
        decode_event(
            target.event(),
            &bindings[&actor(2)],
            Some((&bindings[&actor(3)], &wrong)),
            112
        )
        .is_err()
    );
    let decoded = decode_event(
        target.event(),
        &bindings[&actor(2)],
        Some((&bindings[&actor(3)], &recipient)),
        112,
    )
    .unwrap();
    assert_eq!(decoded, publication);
    let mut receiving = SocialState::default();
    assert!(
        receive_event(
            &mut receiving,
            target.event(),
            &bindings[&actor(2)],
            Some((&bindings[&actor(3)], &recipient)),
            &policy,
            112
        )
        .unwrap()
    );
    assert!(
        !receive_event(
            &mut receiving,
            target.event(),
            &bindings[&actor(2)],
            Some((&bindings[&actor(3)], &recipient)),
            &policy,
            113
        )
        .unwrap()
    );
}
#[test]
fn public_transport_cannot_smuggle_a_private_publication_or_claim_another_holder() {
    let sender = Keys::parse(&"03".repeat(32)).unwrap();
    let bound = binding(&sender, 2);
    let (_, private, _) = approved(SocialAction::Friendship {
        peer: actor(3),
        requested: true,
    });
    let wire = EventBuilder::new(
        Kind::Custom(30078),
        String::from_utf8(canonical(&private).unwrap()).unwrap(),
    )
    .tags([Tag::identifier(format!(
        "booknook:social:{}:public",
        private.event.id
    ))])
    .custom_created_at(Timestamp::from(110))
    .sign_with_keys(&sender)
    .unwrap();
    assert!(decode_event(&wire, &bound, None, 111).is_err());
    let (state, public, policy) = approved(SocialAction::Publish {
        group_id: None,
        post: Post {
            kind: PostKind::Note,
            book: None,
            body: "Selected public text".into(),
            stars: None,
            reading_status: None,
            spoiler: false,
            reply_to: None,
        },
    });
    let wires = encode_publication(
        &state,
        &public,
        &policy,
        &bound,
        &BTreeMap::new(),
        &sender,
        111,
    )
    .unwrap();
    assert_eq!(wires.len(), 1);
    assert_eq!(
        decode_event(wires[0].event(), &bound, None, 112).unwrap(),
        public
    );
    let other = binding(&sender, 3);
    assert!(decode_event(wires[0].event(), &other, None, 112).is_err());
}
#[test]
fn missing_recipient_binding_and_revoked_binding_version_fail_before_send() {
    let sender = Keys::parse(&"03".repeat(32)).unwrap();
    let bound = binding(&sender, 2);
    let (state, p, policy) = approved(SocialAction::Friendship {
        peer: actor(3),
        requested: true,
    });
    assert!(
        encode_publication(&state, &p, &policy, &bound, &BTreeMap::new(), &sender, 111).is_err()
    );
    let raw = KeyBinding {
        schema: "booknook.nostr-binding.v1".into(),
        castalia_holder: actor(2),
        nostr_public_key: sender.public_key().to_hex(),
        valid_from: 90,
        valid_until: 1000,
        version: 1,
    };
    let (r, p, policy) = receipt(&canonical(&raw).unwrap(), "nostr.bind", 1, 100, 2);
    let doc = BindingDocument {
        schema: "booknook.transport-identity.v1".into(),
        binding: raw,
        approved_at: 100,
        request_json: r,
        presentation_json: p,
    };
    assert!(verify_binding(&doc, &policy, 2, 110).is_err());
    assert!(verify_binding(&doc, &policy, 1, 1000).is_err());
    let mut revoked = policy;
    revoked.pins.clear();
    assert!(verify_binding(&doc, &revoked, 1, 110).is_err());
}

#[test]
fn offline_approved_record_can_be_sent_after_binding_and_retry_is_logically_deduplicated() {
    let keys = Keys::parse(&"03".repeat(32)).unwrap();
    let (state, publication, policy) = approved(SocialAction::Publish {
        group_id: None,
        post: Post {
            kind: PostKind::Note,
            book: None,
            body: "Reviewed before connecting transport".into(),
            stars: None,
            reading_status: None,
            spoiler: false,
            reply_to: None,
        },
    });
    let sender = binding_started(&keys, 2, 120);
    let mut received = SocialState::default();
    for at in [130, 140] {
        let wires = encode_publication(
            &state,
            &publication,
            &policy,
            &sender,
            &BTreeMap::new(),
            &keys,
            at,
        )
        .unwrap();
        assert_eq!(wires[0].event().created_at.as_secs(), at);
        assert_eq!(
            decode_event(wires[0].event(), &sender, None, at)
                .unwrap()
                .approved_at,
            110
        );
        assert_eq!(
            receive_event(&mut received, wires[0].event(), &sender, None, &policy, at).unwrap(),
            at == 130
        );
    }
    assert_eq!(received.publications.len(), 1);
}
