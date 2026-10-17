// SPDX-License-Identifier: MIT OR Apache-2.0
use booknook_core::{social_protocol::*, *};
use std::collections::BTreeSet;

#[path = "fixtures/social_receipts.rs"]
mod receipts;
use receipts::{actor, proof};
fn execute(s: &mut SocialState, seed: u8, a: SocialAction, at: u64) -> ApprovedPublication {
    let e = prepare(s, &actor(seed), a, at).unwrap();
    let (r, p, policy) = proof(&e, seed);
    approve(s, e, r, p, &policy, at).unwrap()
}
fn note(body: &str, spoiler: bool) -> Post {
    Post {
        kind: PostKind::Note,
        book: None,
        body: body.into(),
        stars: None,
        reading_status: None,
        spoiler,
        reply_to: None,
    }
}
fn b() -> BookReference {
    BookReference {
        id: "urn:isbn:9780000000001".into(),
        title: "Synthetic Library".into(),
        author: "Fixture".into(),
    }
}
fn add_friends(s: &mut SocialState) {
    execute(
        s,
        2,
        SocialAction::Friendship {
            peer: actor(3),
            requested: true,
        },
        100,
    );
    execute(
        s,
        3,
        SocialAction::Friendship {
            peer: actor(2),
            requested: true,
        },
        101,
    );
}
fn make_group(s: &mut SocialState, private: bool) -> String {
    let id = new_owned_id(&actor(2)).unwrap();
    execute(
        s,
        2,
        SocialAction::CreateGroup {
            id: id.clone(),
            name: "Readers".into(),
            description: "Synthetic test".into(),
            private,
        },
        100,
    );
    id
}
fn join(s: &mut SocialState, id: &str, seed: u8, at: u64) {
    let g = s.groups[id].clone();
    let invite = crate::new_id();
    if !g.private {
        execute(
            s,
            seed,
            SocialAction::RequestJoin {
                group_id: id.into(),
            },
            at,
        );
    } else {
        execute(
            s,
            2,
            SocialAction::Invite {
                id: invite.clone(),
                group: g,
                invitee: actor(seed),
                expires_at: at + 1000,
            },
            at,
        );
        execute(
            s,
            seed,
            SocialAction::AcceptInvitation {
                admission_consent: None,
                invitation_id: invite,
            },
            at + 1,
        );
    }
    let mut members = s.groups[id].members.clone();
    members.insert(actor(seed));
    execute(
        s,
        2,
        SocialAction::SetMembers {
            admission_proofs: Vec::new(),
            group_id: id.into(),
            members,
            moderators: Default::default(),
        },
        at + 2,
    );
}

#[test]
fn friendship_requires_mutual_current_consent_and_feed_is_chronological() {
    let mut s = SocialState::default();
    execute(
        &mut s,
        2,
        SocialAction::Friendship {
            peer: actor(3),
            requested: true,
        },
        100,
    );
    assert!(!friends(&s, &actor(2), &actor(3)));
    execute(
        &mut s,
        3,
        SocialAction::Publish {
            group_id: None,
            post: note("Before friendship", false),
        },
        101,
    );
    assert!(feed(&s, &actor(2), false).is_empty());
    execute(
        &mut s,
        3,
        SocialAction::Friendship {
            peer: actor(2),
            requested: true,
        },
        102,
    );
    execute(
        &mut s,
        3,
        SocialAction::Publish {
            group_id: None,
            post: note("Latest secret twist", true),
        },
        103,
    );
    let f = feed(&s, &actor(2), false);
    assert_eq!(f.len(), 2);
    assert!(f[0].spoiler_hidden);
    assert_eq!(f[0].post.body, "");
    assert_eq!(
        feed(&s, &actor(2), true)[0].post.body,
        "Latest secret twist"
    );
    execute(
        &mut s,
        3,
        SocialAction::Friendship {
            peer: actor(2),
            requested: false,
        },
        104,
    );
    assert!(feed(&s, &actor(2), false).is_empty());
}
#[test]
fn publication_approval_is_exact_atomic_deduplicated_and_currently_pinned() {
    let mut s = SocialState::default();
    let e = prepare(
        &s,
        &actor(2),
        SocialAction::Publish {
            group_id: None,
            post: note("Selected only", false),
        },
        100,
    )
    .unwrap();
    let (r, p, policy) = proof(&e, 2);
    let mut modified = e.clone();
    if let SocialAction::Publish { post, .. } = &mut modified.action {
        post.body = "Unreviewed".into();
    }
    assert!(approve(&mut s, modified, r.clone(), p.clone(), &policy, 100).is_err());
    assert!(s.publications.is_empty());
    let approved = approve(&mut s, e, r, p, &policy, 100).unwrap();
    let before = s.clone();
    assert!(!receive(&mut s, approved.clone(), &policy, 200).unwrap());
    assert_eq!(s, before);
    let mut revoked = policy;
    revoked.pins.clear();
    assert!(receive(&mut s, approved, &revoked, 200).is_err());
}
#[test]
fn direct_invitation_bootstraps_private_group_and_requires_acceptance() {
    let mut owner = SocialState::default();
    let id = make_group(&mut owner, true);
    let g = owner.groups[&id].clone();
    let e = prepare(
        &owner,
        &actor(2),
        SocialAction::Invite {
            id: "invite-one".into(),
            group: g,
            invitee: actor(3),
            expires_at: 500,
        },
        101,
    )
    .unwrap();
    let (r, p, policy) = proof(&e, 2);
    let approved = approve(&mut owner, e, r, p, &policy, 101).unwrap();
    let mut invitee = SocialState::default();
    receive(&mut invitee, approved, &policy, 102).unwrap();
    assert_eq!(invitee.groups[&id].owner, actor(2));
    assert!(discover_groups(&invitee, "").is_empty());
    let mut members = owner.groups[&id].members.clone();
    members.insert(actor(3));
    assert!(
        prepare(
            &owner,
            &actor(2),
            SocialAction::SetMembers {
                admission_proofs: Vec::new(),
                group_id: id,
                members,
                moderators: Default::default()
            },
            103
        )
        .is_err()
    );
}
#[test]
fn revoked_member_and_stale_epoch_cannot_publish_or_retry_private_content() {
    let mut s = SocialState::default();
    let id = make_group(&mut s, true);
    join(&mut s, &id, 3, 101);
    let old = execute(
        &mut s,
        3,
        SocialAction::Publish {
            group_id: Some(id.clone()),
            post: note("Group-only note", false),
        },
        105,
    );
    let (_, _, policy) = proof(&old.event, 3);
    assert!(validate_outgoing(&s, &old, &policy, 106).is_ok());
    execute(
        &mut s,
        2,
        SocialAction::SetMembers {
            admission_proofs: Vec::new(),
            group_id: id.clone(),
            members: [actor(2)].into(),
            moderators: Default::default(),
        },
        106,
    );
    assert!(validate_outgoing(&s, &old, &policy, 107).is_err());
    assert!(
        prepare(
            &s,
            &actor(3),
            SocialAction::Publish {
                group_id: Some(id.clone()),
                post: note("After removal", false)
            },
            107
        )
        .is_err()
    );
    assert!(feed(&s, &actor(3), true).is_empty());
}
#[test]
fn moderator_permissions_blocks_and_hidden_posts_are_enforced() {
    let mut s = SocialState::default();
    let id = make_group(&mut s, false);
    join(&mut s, &id, 3, 101);
    join(&mut s, &id, 4, 105);
    let members = s.groups[&id].members.clone();
    execute(
        &mut s,
        2,
        SocialAction::SetMembers {
            admission_proofs: Vec::new(),
            group_id: id.clone(),
            members,
            moderators: [actor(3)].into(),
        },
        109,
    );
    let post = execute(
        &mut s,
        4,
        SocialAction::Publish {
            group_id: Some(id.clone()),
            post: note("Moderated discussion", false),
        },
        110,
    );
    assert!(
        prepare(
            &s,
            &actor(4),
            SocialAction::Moderate {
                group_id: id.clone(),
                event_id: Some(post.event.id.clone()),
                ban_actor: None
            },
            111
        )
        .is_err()
    );
    execute(
        &mut s,
        3,
        SocialAction::Moderate {
            group_id: id.clone(),
            event_id: Some(post.event.id),
            ban_actor: Some(actor(4)),
        },
        111,
    );
    assert!(feed(&s, &actor(2), true).is_empty());
    assert!(
        prepare(
            &s,
            &actor(4),
            SocialAction::Publish {
                group_id: Some(id),
                post: note("Banned", false)
            },
            112
        )
        .is_err()
    );
    add_friends(&mut s);
    local(
        &mut s,
        LocalSocialCommand::Block {
            actor: actor(3),
            blocked: true,
        },
    )
    .unwrap();
    assert!(!friends(&s, &actor(2), &actor(3)));
}
#[test]
fn shared_lists_are_owner_managed_and_versions_prevent_overwrites() {
    let mut s = SocialState::default();
    let id = new_owned_id(&actor(2)).unwrap();
    execute(
        &mut s,
        2,
        SocialAction::CreateList {
            id: id.clone(),
            name: "Weekend".into(),
            description: String::new(),
            books: vec![b()],
        },
        100,
    );
    assert!(
        prepare(
            &s,
            &actor(3),
            SocialAction::UpdateList {
                id: id.clone(),
                name: "Hijacked".into(),
                description: String::new(),
                books: vec![]
            },
            101
        )
        .is_err()
    );
    let stale = prepare(
        &s,
        &actor(2),
        SocialAction::UpdateList {
            id: id.clone(),
            name: "Stale".into(),
            description: String::new(),
            books: vec![],
        },
        101,
    )
    .unwrap();
    let (r, p, policy) = proof(&stale, 2);
    execute(
        &mut s,
        2,
        SocialAction::UpdateList {
            id: id.clone(),
            name: "Current".into(),
            description: String::new(),
            books: vec![b()],
        },
        102,
    );
    assert!(approve(&mut s, stale, r, p, &policy, 103).is_err());
    assert_eq!(s.lists[&id].name, "Current");
}
#[test]
fn acknowledgments_are_per_recipient_and_outages_retain_outbox() {
    let mut s = SocialState::default();
    let p = execute(
        &mut s,
        2,
        SocialAction::Friendship {
            peer: actor(3),
            requested: true,
        },
        100,
    );
    assert!(mark_delivered(&mut s, &p.event.id, &actor(2), BTreeSet::new()).is_err());
    assert_eq!(s.outbox.len(), 1);
    assert!(
        !mark_delivered(
            &mut s,
            &p.event.id,
            &actor(2),
            ["wss://relay.example".into()].into()
        )
        .unwrap()
    );
    assert_eq!(s.outbox.len(), 1);
    assert!(
        mark_delivered(
            &mut s,
            &p.event.id,
            &actor(3),
            ["wss://relay.example".into()].into()
        )
        .unwrap()
    );
    assert!(s.outbox.is_empty());
}
#[test]
fn recommendations_exclude_private_groups_and_deduplicate_repeated_ratings() {
    let mut s = SocialState::default();
    add_friends(&mut s);
    let id = make_group(&mut s, true);
    join(&mut s, &id, 3, 105);
    let rating = Post {
        kind: PostKind::Rating,
        book: Some(b()),
        body: String::new(),
        stars: Some(5),
        reading_status: None,
        spoiler: false,
        reply_to: None,
    };
    execute(
        &mut s,
        3,
        SocialAction::Publish {
            group_id: Some(id),
            post: rating.clone(),
        },
        110,
    );
    let candidates = || {
        vec![social::CatalogCandidate {
            book_id: b().id,
            title: b().title,
            subjects: vec![],
        }]
    };
    assert!(recommendations(&s, &actor(2), candidates(), Default::default()).is_empty());
    execute(
        &mut s,
        3,
        SocialAction::Publish {
            group_id: None,
            post: rating.clone(),
        },
        111,
    );
    execute(
        &mut s,
        3,
        SocialAction::Publish {
            group_id: None,
            post: rating,
        },
        112,
    );
    let recs = recommendations(&s, &actor(2), candidates(), Default::default());
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].score, 5);
    assert_eq!(recs[0].reasons.len(), 1);
    let viewer_event = &s
        .publications
        .values()
        .find(|p| p.event.actor == actor(2))
        .unwrap()
        .event;
    let (_, _, policy) = proof(viewer_event, 2);
    assert_eq!(
        canonical(
            &audited_recommendations(&s, &policy, candidates(), Default::default(), 1000).unwrap()
        )
        .unwrap(),
        canonical(&recs).unwrap()
    );
    let mut altered = s.clone();
    altered.friendship_intents.clear();
    assert!(
        audited_recommendations(&altered, &policy, candidates(), Default::default(), 1000).is_err()
    );
    let mut revoked = policy.clone();
    revoked.pins.clear();
    assert!(audited_recommendations(&s, &revoked, candidates(), Default::default(), 1000).is_err());
    let mut blocked = s.clone();
    blocked.blocked.insert(actor(3));
    assert!(
        audited_recommendations(&blocked, &policy, candidates(), Default::default(), 1000)
            .unwrap()
            .is_empty()
    );
}
#[test]
fn social_state_is_portable_without_requiring_changes_to_notebook_archives() {
    let mut s = SocialState::default();
    add_friends(&mut s);
    let id = make_group(&mut s, true);
    join(&mut s, &id, 3, 103);
    social_protocol::validate(&s).unwrap();
    let restored: SocialState = strict_parse(&canonical(&s).unwrap()).unwrap();
    assert_eq!(s, restored);
    social_protocol::validate(&restored).unwrap();
    let mut unknown = serde_json::to_value(s).unwrap();
    unknown["notebook"] = serde_json::json!({"text":"Must never enter social processing"});
    assert!(strict_parse::<SocialState>(&canonical(&unknown).unwrap()).is_err());
}

#[test]
fn human_consent_delay_is_preserved_in_historical_approval_time() {
    let mut state = SocialState::default();
    let event = prepare(
        &state,
        &actor(2),
        SocialAction::Publish {
            group_id: None,
            post: note("Reviewed after a short delay", false),
        },
        100,
    )
    .unwrap();
    let (r, p, policy) = receipts::receipt(
        &canonical(&event).unwrap(),
        ACTION,
        event.resource_version,
        105,
        2,
    );
    let publication = approve(&mut state, event, r, p, &policy, 106).unwrap();
    assert_eq!(publication.approved_at, 106);
    let mut remote = SocialState::default();
    assert!(receive(&mut remote, publication, &policy, 500).unwrap());
}

#[test]
fn public_admission_is_verified_from_public_history_and_requires_current_reader_consent() {
    let mut owner = SocialState::default();
    let group_id = make_group(&mut owner, false);
    assert!(
        prepare(
            &owner,
            &actor(2),
            SocialAction::SetMembers {
                admission_proofs: Vec::new(),
                group_id: group_id.clone(),
                members: [actor(2), actor(3)].into(),
                moderators: Default::default()
            },
            101
        )
        .is_err()
    );
    let request = execute(
        &mut owner,
        3,
        SocialAction::RequestJoin {
            group_id: group_id.clone(),
        },
        102,
    );
    assert_eq!(request.event.delivery, Delivery::Public);
    let admission = execute(
        &mut owner,
        2,
        SocialAction::SetMembers {
            admission_proofs: Vec::new(),
            group_id: group_id.clone(),
            members: [actor(2), actor(3)].into(),
            moderators: Default::default(),
        },
        103,
    );
    let mut observer = SocialState::default();
    let mut public_history: Vec<_> = owner
        .publications
        .values()
        .filter(|p| p.event.delivery == Delivery::Public)
        .cloned()
        .collect();
    public_history.sort_by_key(|p| p.event.at);
    for publication in public_history {
        let (_, _, policy) = proof(
            &publication.event,
            if publication.event.actor == actor(2) {
                2
            } else {
                3
            },
        );
        receive(&mut observer, publication, &policy, 104).unwrap();
    }
    assert_eq!(owner.groups, observer.groups);
    assert_eq!(owner.join_requests, observer.join_requests);
    assert_eq!(
        observer.groups[&group_id].epoch,
        admission.event.resource_version + 1
    );
    let private_id = make_group(&mut owner, true);
    assert!(
        prepare(
            &owner,
            &actor(4),
            SocialAction::RequestJoin {
                group_id: private_id
            },
            105
        )
        .is_err()
    );
    execute(
        &mut owner,
        4,
        SocialAction::RequestJoin {
            group_id: group_id.clone(),
        },
        106,
    );
    assert!(
        prepare(
            &owner,
            &actor(2),
            SocialAction::SetMembers {
                admission_proofs: Vec::new(),
                group_id,
                members: [actor(2), actor(3), actor(4)].into(),
                moderators: Default::default()
            },
            106 + 7 * 86400
        )
        .is_err()
    );
    let mut legacy = serde_json::to_value(SocialState::default()).unwrap();
    legacy.as_object_mut().unwrap().remove("join_requests");
    assert!(
        serde_json::from_value::<SocialState>(legacy)
            .unwrap()
            .join_requests
            .is_empty()
    );
}

#[test]
fn retraction_and_moderation_cancel_unsent_content_and_reject_stale_retry_copies() {
    for private_group in [false, true] {
        let mut state = SocialState::default();
        let group_id = private_group.then(|| make_group(&mut state, true));
        let publication = execute(
            &mut state,
            2,
            SocialAction::Publish {
                group_id: group_id.clone(),
                post: note("Approved offline, withdrawn before connecting", false),
            },
            110,
        );
        let old_retry = state.outbox[&publication.event.id].clone();
        let removal = if let Some(group_id) = group_id {
            SocialAction::Moderate {
                group_id,
                event_id: Some(publication.event.id.clone()),
                ban_actor: None,
            }
        } else {
            SocialAction::Retract {
                event_id: publication.event.id.clone(),
            }
        };
        execute(&mut state, 2, removal, 111);
        assert!(!state.outbox.contains_key(&publication.event.id));
        assert_eq!(state.publications[&publication.event.id], publication);
        let (_, _, policy) = proof(&publication.event, 2);
        assert!(validate_outgoing(&state, &publication, &policy, 112).is_err());
        // An older portable retry copy cannot bypass the current retraction projection.
        state.outbox.insert(publication.event.id.clone(), old_retry);
        assert!(
            validate_outgoing(&state, &publication, &policy, 112)
                .unwrap_err()
                .to_string()
                .contains("retracted")
        );
    }
}

#[test]
fn cancelling_delivery_preserves_approved_history_and_receipt_replay_protection() {
    let mut state = SocialState::default();
    let publication = execute(
        &mut state,
        2,
        SocialAction::Publish {
            group_id: None,
            post: note("Retain locally without retrying", false),
        },
        110,
    );
    let receipts = state.replay.clone();
    local(
        &mut state,
        LocalSocialCommand::CancelDelivery {
            event_id: publication.event.id.clone(),
        },
    )
    .unwrap();
    assert!(state.outbox.is_empty());
    assert_eq!(state.publications[&publication.event.id], publication);
    assert_eq!(state.replay, receipts);
    assert!(!state.retracted.contains(&publication.event.id));
    let (_, _, policy) = proof(&publication.event, 2);
    assert!(validate_outgoing(&state, &publication, &policy, 111).is_err());
    assert!(!receive(&mut state, publication, &policy, 111).unwrap());
    assert!(state.outbox.is_empty());
}
