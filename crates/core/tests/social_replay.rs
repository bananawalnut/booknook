// SPDX-License-Identifier: MIT OR Apache-2.0
use booknook_core::{Error, social_protocol::*};
use std::collections::BTreeMap;
#[path = "fixtures/social_receipts.rs"]
mod receipts;
use receipts::{actor, proof};

// All receipts deliberately share one second; identifiers run opposite causal order.
fn approved(s: &mut SocialState, seed: u8, id: &str, action: SocialAction) -> ApprovedPublication {
    let mut event = prepare(s, &actor(seed), action, 100).unwrap();
    event.id = id.into();
    let (r, p, policy) = proof(&event, seed);
    approve(s, event, r, p, &policy, 100).unwrap()
}
fn post(group: &str, parent: Option<&str>) -> SocialAction {
    SocialAction::Publish {
        group_id: Some(group.into()),
        post: Post {
            kind: PostKind::Discussion,
            book: None,
            body: "Synthetic reply".into(),
            stars: None,
            reading_status: None,
            spoiler: true,
            reply_to: parent.map(Into::into),
        },
    }
}
fn group(s: &mut SocialState) -> String {
    let group = format!("{}:same-second", actor(2));
    approved(
        s,
        2,
        "z-create",
        SocialAction::CreateGroup {
            id: group.clone(),
            name: "Synthetic readers".into(),
            description: String::new(),
            private: true,
        },
    );
    group
}
fn policy(p: &ApprovedPublication) -> booknook_core::authority::AuthorityPolicy {
    proof(&p.event, if p.event.actor == actor(2) { 2 } else { 3 }).2
}

#[test]
fn causal_replay_orders_same_second_admission_and_reply_chain_before_epoch_change() {
    let mut state = SocialState::default();
    let group = group(&mut state);
    let snapshot = state.groups[&group].clone();
    approved(
        &mut state,
        2,
        "y-invite",
        SocialAction::Invite {
            id: "invite".into(),
            group: snapshot,
            invitee: actor(3),
            expires_at: 200,
        },
    );
    approved(
        &mut state,
        3,
        "x-accept",
        SocialAction::AcceptInvitation {
            invitation_id: "invite".into(),
            admission_consent: None,
        },
    );
    approved(
        &mut state,
        2,
        "w-admit",
        SocialAction::SetMembers {
            group_id: group.clone(),
            members: [actor(2), actor(3)].into(),
            moderators: Default::default(),
            admission_proofs: Vec::new(),
        },
    );
    for (id, parent) in [
        ("v-parent", None),
        ("u-reply", Some("v-parent")),
        ("t-reply", Some("u-reply")),
        ("s-reply", Some("t-reply")),
    ] {
        approved(&mut state, 3, id, post(&group, parent));
    }
    approved(
        &mut state,
        3,
        "r-retract",
        SocialAction::Retract {
            event_id: "s-reply".into(),
        },
    );
    let before_removal = state.clone();
    approved(
        &mut state,
        2,
        "q-remove",
        SocialAction::SetMembers {
            group_id: group.clone(),
            members: [actor(2)].into(),
            moderators: Default::default(),
            admission_proofs: Vec::new(),
        },
    );
    let expected = [
        "z-create",
        "y-invite",
        "x-accept",
        "w-admit",
        "v-parent",
        "u-reply",
        "t-reply",
        "s-reply",
        "r-retract",
        "q-remove",
    ];
    let ascending: Vec<_> = state.publications.values().collect();
    let descending: Vec<_> = ascending.iter().rev().copied().collect();
    // Descending inputs cache each reply parent before its child. Ascending starts at a leaf.
    for input in [ascending, descending] {
        let ordered = causal_replay_order(input);
        assert_eq!(
            ordered
                .iter()
                .map(|p| p.event.id.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        let mut rebuilt = SocialState::default();
        for publication in ordered {
            assert!(receive(&mut rebuilt, publication.clone(), &policy(publication), 100).unwrap());
        }
        assert_eq!(rebuilt.groups, state.groups);
        assert_eq!(rebuilt.invitations, state.invitations);
        assert_eq!(rebuilt.retracted, state.retracted);
        assert_eq!(rebuilt.replay, state.replay);
        assert_eq!(rebuilt.publications, state.publications);
        assert_eq!(rebuilt.revision, state.revision);
    }
    // Scheduling never authorizes a second mutation of the same version or an old-epoch post.
    let mut alternate = before_removal.clone();
    let conflict = approved(
        &mut alternate,
        2,
        "a-conflict",
        SocialAction::SetMembers {
            group_id: group.clone(),
            members: [actor(2), actor(3)].into(),
            moderators: [actor(3)].into(),
            admission_proofs: Vec::new(),
        },
    );
    let mut prior = before_removal;
    let stale = approved(&mut prior, 3, "b-stale", post(&group, None));
    let before = state.clone();
    for p in causal_replay_order([&conflict, &stale]) {
        assert!(matches!(
            receive(&mut state, p.clone(), &policy(p), 100),
            Err(Error::Conflict(_))
        ));
        assert_eq!(state, before);
    }
}

#[test]
fn causal_replay_keeps_missing_and_cyclic_reply_history_unresolved() {
    let mut state = SocialState::default();
    let group = group(&mut state);
    let mut publications = BTreeMap::new();
    for (id, parent) in [
        ("a-missing", "unknown"),
        ("b-cycle", "c-cycle"),
        ("c-cycle", "b-cycle"),
        ("d-child", "b-cycle"),
    ] {
        // A malicious but correctly signed publication can name a missing/cyclic parent.
        let mut event = prepare(&state, &actor(2), post(&group, None), 100).unwrap();
        event.id = id.into();
        if let SocialAction::Publish { post, .. } = &mut event.action {
            post.reply_to = Some(parent.into());
        }
        let (request_json, presentation_json, authority) = proof(&event, 2);
        let publication = ApprovedPublication {
            schema: "booknook.approved-publication.v1".into(),
            event,
            approved_at: 100,
            request_json,
            presentation_json,
        };
        verify_publication(&publication, &authority, 100).unwrap();
        publications.insert(id.to_string(), publication);
    }
    let ascending: Vec<_> = publications.values().collect();
    let descending: Vec<_> = ascending.iter().rev().copied().collect();
    let expected = ["a-missing", "b-cycle", "c-cycle", "d-child"];
    for input in [ascending, descending] {
        let ordered = causal_replay_order(input);
        assert_eq!(
            ordered
                .iter()
                .map(|p| p.event.id.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        let before = state.clone();
        // Retry passes cannot manufacture missing dependencies or accept a cycle.
        for _ in 0..2 {
            for p in &ordered {
                assert!(matches!(
                    receive(&mut state, (*p).clone(), &policy(p), 100),
                    Err(Error::NotFound(_))
                ));
            }
            assert_eq!(state, before);
        }
    }
    approved(
        &mut state,
        2,
        "close",
        SocialAction::CloseGroup { group_id: group },
    );
    let before = state.clone();
    for p in causal_replay_order(publications.values()) {
        assert!(matches!(
            receive(&mut state, p.clone(), &policy(p), 100),
            Err(Error::Conflict(_))
        ));
        assert_eq!(state, before);
    }
}

#[test]
fn repeated_public_join_requests_converge_without_accepting_stale_epochs() {
    let mut base = SocialState::default();
    let group = format!("{}:public", actor(2));
    approved(
        &mut base,
        2,
        "create",
        SocialAction::CreateGroup {
            id: group.clone(),
            name: "Public fixture".into(),
            description: String::new(),
            private: false,
        },
    );
    let mut producer = base.clone();
    let first = approved(
        &mut producer,
        3,
        "a-first",
        SocialAction::RequestJoin {
            group_id: group.clone(),
        },
    );
    let latest = approved(
        &mut producer,
        3,
        "z-latest",
        SocialAction::RequestJoin {
            group_id: group.clone(),
        },
    );
    let mut forwards = base.clone();
    let mut reverse = base.clone();
    for p in [&first, &latest] {
        receive(&mut forwards, p.clone(), &policy(p), 100).unwrap();
    }
    for p in [&latest, &first] {
        receive(&mut reverse, p.clone(), &policy(p), 100).unwrap();
    }
    assert_eq!(forwards.join_requests, reverse.join_requests);
    assert_eq!(forwards.publications, reverse.publications);
    assert_eq!(forwards.replay, reverse.replay);
    assert_eq!(
        forwards.join_requests.values().next().unwrap().id,
        "z-latest"
    );
    // Later signed time wins over event ID; all valid consent proofs remain recoverable.
    let mut event = prepare(
        &base,
        &actor(3),
        SocialAction::RequestJoin {
            group_id: group.clone(),
        },
        101,
    )
    .unwrap();
    event.id = "0-later-time".into();
    let (r, p, auth) = proof(&event, 3);
    let later = approve(&mut producer, event, r, p, &auth, 101).unwrap();
    receive(&mut reverse, later.clone(), &auth, 101).unwrap();
    assert_eq!(
        reverse.join_requests.values().next().unwrap().id,
        later.event.id
    );
    let mut out_of_order = base.clone();
    for p in [&later, &latest, &first] {
        receive(&mut out_of_order, p.clone(), &policy(p), 101).unwrap();
    }
    assert_eq!(reverse.join_requests, out_of_order.join_requests);
    assert_eq!(reverse.publications, out_of_order.publications);
    assert_eq!(reverse.replay, out_of_order.replay);
    // Advancing the epoch never makes an unseen stale request acceptable through scheduling.
    approved(
        &mut base,
        2,
        "membership-change",
        SocialAction::SetMembers {
            group_id: group,
            members: [actor(2)].into(),
            moderators: Default::default(),
            admission_proofs: Vec::new(),
        },
    );
    let before = base.clone();
    for p in causal_replay_order([&later, &first, &latest]) {
        assert!(matches!(
            receive(&mut base, p.clone(), &policy(p), 101),
            Err(Error::Conflict(_))
        ));
        assert_eq!(base, before);
    }
}
