// SPDX-License-Identifier: MIT OR Apache-2.0
//! Three independent readers receive only their addressed, authenticated NIP-44 deliveries.
use booknook_connectors::{KeyBinding, VerifiedBinding, nostr_transport::*};
use booknook_core::{canonical, social_protocol::*};
use nostr_sdk::prelude::*;
use std::collections::BTreeMap;
#[path = "fixtures/social_receipts.rs"]
mod receipts;
use receipts::{actor, proof, receipt};

struct Reader {
    state: SocialState,
    keys: Keys,
}
fn readers() -> (BTreeMap<u8, Reader>, BTreeMap<String, VerifiedBinding>) {
    let mut readers = BTreeMap::new();
    let mut bindings = BTreeMap::new();
    for seed in [2, 3, 4] {
        let keys = Keys::parse(&format!("{seed:02x}").repeat(32)).unwrap();
        let binding = KeyBinding {
            schema: "booknook.nostr-binding.v1".into(),
            castalia_holder: actor(seed),
            nostr_public_key: keys.public_key().to_hex(),
            valid_from: 90,
            valid_until: 2000,
            version: 1,
        };
        let (request_json, presentation_json, policy) =
            receipt(&canonical(&binding).unwrap(), "nostr.bind", 1, 90, seed);
        let bound = verify_binding(
            &BindingDocument {
                schema: "booknook.transport-identity.v1".into(),
                binding,
                approved_at: 90,
                request_json,
                presentation_json,
            },
            &policy,
            1,
            90,
        )
        .unwrap();
        bindings.insert(actor(seed), bound);
        readers.insert(
            seed,
            Reader {
                state: SocialState::default(),
                keys,
            },
        );
    }
    (readers, bindings)
}
fn send(
    readers: &mut BTreeMap<u8, Reader>,
    bindings: &BTreeMap<String, VerifiedBinding>,
    seed: u8,
    action: SocialAction,
    at: u64,
) -> ApprovedPublication {
    let sender = readers.get_mut(&seed).unwrap();
    let event = prepare(&sender.state, &actor(seed), action, at).unwrap();
    let (request, presentation, policy) = proof(&event, seed);
    let publication =
        approve(&mut sender.state, event, request, presentation, &policy, at).unwrap();
    let deliveries = encode_publication(
        &sender.state,
        &publication,
        &policy,
        &bindings[&actor(seed)],
        bindings,
        &sender.keys,
        at,
    )
    .unwrap();
    for (recipient_seed, reader) in readers.iter_mut() {
        for delivery in &deliveries {
            if delivery.recipient() == actor(*recipient_seed) {
                receive_event(
                    &mut reader.state,
                    delivery.event(),
                    &bindings[&actor(seed)],
                    Some((&bindings[&actor(*recipient_seed)], &reader.keys)),
                    &policy,
                    at,
                )
                .unwrap();
            }
        }
    }
    publication
}

fn awaiting_admission() -> (
    BTreeMap<u8, Reader>,
    BTreeMap<String, VerifiedBinding>,
    String,
) {
    let (mut desks, bindings) = readers();
    let group_id = new_owned_id(&actor(2)).unwrap();
    send(
        &mut desks,
        &bindings,
        2,
        SocialAction::CreateGroup {
            id: group_id.clone(),
            name: "Private readers".into(),
            description: "Synthetic group".into(),
            private: true,
        },
        100,
    );
    for seed in [3, 4] {
        let group = desks[&2].state.groups[&group_id].clone();
        send(
            &mut desks,
            &bindings,
            2,
            SocialAction::Invite {
                id: format!("invite-{seed}"),
                group,
                invitee: actor(seed),
                expires_at: 900,
            },
            100 + u64::from(seed),
        );
        send(
            &mut desks,
            &bindings,
            seed,
            SocialAction::AcceptInvitation {
                admission_consent: None,
                invitation_id: format!("invite-{seed}"),
            },
            110 + u64::from(seed),
        );
    }
    // Neither invitee was an original recipient of the other's private invitation/acceptance.
    assert!(!desks[&3].state.invitations.contains_key("invite-4"));
    assert!(!desks[&4].state.invitations.contains_key("invite-3"));
    (desks, bindings, group_id)
}

#[test]
fn private_batch_admission_reaches_three_independent_addressed_desks() {
    let (mut desks, bindings, group_id) = awaiting_admission();
    send(
        &mut desks,
        &bindings,
        2,
        SocialAction::SetMembers {
            admission_proofs: Vec::new(),
            group_id: group_id.clone(),
            members: [actor(2), actor(3), actor(4)].into(),
            moderators: Default::default(),
        },
        120,
    );
    for reader in desks.values() {
        assert_eq!(reader.state.groups[&group_id].members.len(), 3);
    }
    send(
        &mut desks,
        &bindings,
        3,
        SocialAction::Publish {
            group_id: Some(group_id.clone()),
            post: Post {
                kind: PostKind::Discussion,
                book: None,
                body: "A private discussion after batch admission".into(),
                stars: None,
                reading_status: None,
                spoiler: false,
                reply_to: None,
            },
        },
        121,
    );
    for (seed, reader) in &desks {
        assert_eq!(feed(&reader.state, &actor(*seed), true).len(), 1);
        assert!(discover_groups(&reader.state, "").is_empty());
        // Portable export retains the verifiable batch and no other invitee's invitation.
        let restored: SocialState =
            booknook_core::strict_parse(&canonical(&reader.state).unwrap()).unwrap();
        assert_eq!(restored, reader.state);
    }
}

fn signed(event: SocialEvent, seed: u8) -> ApprovedPublication {
    let (request_json, presentation_json, _) = proof(&event, seed);
    ApprovedPublication {
        schema: "booknook.approved-publication.v1".into(),
        approved_at: event.at,
        event,
        request_json,
        presentation_json,
    }
}

#[test]
fn batch_membership_requires_independent_unexpired_explicit_consent() {
    let (desks, _, group_id) = awaiting_admission();
    let event = prepare(
        &desks[&2].state,
        &actor(2),
        SocialAction::SetMembers {
            group_id,
            members: [actor(2), actor(3), actor(4)].into(),
            moderators: Default::default(),
            admission_proofs: Vec::new(),
        },
        120,
    )
    .unwrap();
    let (_, _, policy) = proof(&event, 2);
    let mut missing = event.clone();
    if let SocialAction::SetMembers {
        admission_proofs, ..
    } = &mut missing.action
    {
        admission_proofs.clear();
    }
    // This is the old batch behavior: a valid owner's signature alone cannot consent for others.
    let mut receiver = desks[&3].state.clone();
    assert!(receive(&mut receiver, signed(missing, 2), &policy, 120).is_err());
    assert_eq!(receiver, desks[&3].state);

    for change in [
        "forged",
        "wrong-group",
        "wrong-epoch",
        "expired",
        "legacy",
        "duplicate",
    ] {
        let mut bad = event.clone();
        if let SocialAction::SetMembers {
            admission_proofs, ..
        } = &mut bad.action
        {
            let p = admission_proofs
                .iter_mut()
                .find(|p| p.event.actor == actor(4))
                .unwrap();
            match change {
                "forged" => {
                    p.event.id.push_str("-tampered");
                }
                "legacy" => {
                    if let SocialAction::AcceptInvitation {
                        admission_consent, ..
                    } = &mut p.event.action
                    {
                        *admission_consent = None;
                    }
                    *p = signed(p.event.clone(), 4);
                }
                "wrong-group" | "wrong-epoch" | "expired" => {
                    if let SocialAction::AcceptInvitation {
                        admission_consent: Some(c),
                        ..
                    } = &mut p.event.action
                    {
                        match change {
                            "wrong-group" => c.group_id.push_str("-other"),
                            "wrong-epoch" => c.epoch += 1,
                            _ => c.expires_at = 119,
                        }
                    }
                    // Even a real reader signature cannot change the group/epoch or extend expiry at reception.
                    *p = signed(p.event.clone(), 4);
                }
                "duplicate" => {}
                _ => unreachable!(),
            }
            if change == "duplicate" {
                admission_proofs.push(admission_proofs[0].clone());
            }
        }
        let mut receiver = desks[&3].state.clone();
        assert!(
            receive(&mut receiver, signed(bad, 2), &policy, 120).is_err(),
            "{change}"
        );
        assert_eq!(
            receiver, desks[&3].state,
            "Rejected proof must not mutate state"
        );
    }
}

#[test]
fn legacy_receipts_preserve_canonical_bytes_and_are_not_redistributed() {
    let (mut desks, _, group_id) = awaiting_admission();
    for seed in [3, 4] {
        let publication = desks
            .get_mut(&2)
            .unwrap()
            .state
            .publications
            .values_mut()
            .find(|p| {
                p.event.actor == actor(seed)
                    && matches!(p.event.action, SocialAction::AcceptInvitation { .. })
            })
            .unwrap();
        if let SocialAction::AcceptInvitation {
            admission_consent, ..
        } = &mut publication.event.action
        {
            *admission_consent = None;
        }
        *publication = signed(publication.event.clone(), seed);
        let raw = canonical(publication).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("admission_consent"));
        let decoded: ApprovedPublication = booknook_core::strict_parse(&raw).unwrap();
        assert_eq!(canonical(&decoded).unwrap(), raw);
        let (_, _, policy) = proof(&decoded.event, seed);
        verify_publication(&decoded, &policy, 120).unwrap();
    }
    assert!(
        prepare(
            &desks[&2].state,
            &actor(2),
            SocialAction::SetMembers {
                group_id: group_id.clone(),
                members: [actor(2), actor(3), actor(4)].into(),
                moderators: Default::default(),
                admission_proofs: Vec::new(),
            },
            120
        )
        .is_err(),
        "Legacy acceptances must be reviewed again before batch disclosure"
    );
    let event = prepare(
        &desks[&2].state,
        &actor(2),
        SocialAction::SetMembers {
            group_id,
            members: [actor(2), actor(3)].into(),
            moderators: Default::default(),
            admission_proofs: Vec::new(),
        },
        120,
    )
    .unwrap();
    assert!(!String::from_utf8_lossy(&canonical(&event).unwrap()).contains("admission_proofs"));
}
