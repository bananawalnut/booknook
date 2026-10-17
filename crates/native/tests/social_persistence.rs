// SPDX-License-Identifier: MIT OR Apache-2.0
use booknook_core::{canonical, settings::*, social_protocol::*};
use booknook_native::Desk;
use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
#[path = "fixtures/social_receipts.rs"]
mod receipts;
fn root() -> PathBuf {
    std::env::temp_dir().join(format!("booknook-social-{}", booknook_core::new_id()))
}
fn open(root: &Path) -> Desk {
    let desk = Desk::new(root.to_string_lossy().into()).unwrap();
    let event = SocialEvent {
        schema: "booknook.social-event.v1".into(),
        id: "config".into(),
        actor: receipts::actor(2),
        at: 100,
        resource_version: 0,
        delivery: Delivery::Public,
        action: SocialAction::Publish {
            group_id: None,
            post: Post {
                kind: PostKind::Note,
                book: None,
                body: "test".into(),
                stars: None,
                reading_status: None,
                spoiler: false,
                reply_to: None,
            },
        },
    };
    let (_, _, policy) = receipts::proof(&event, 2);
    let pin = &policy.pins[0];
    let settings = ConnectionSettings {
        castalia: Some(CastaliaSettings {
            issuer: pin.issuer.clone(),
            key_id: pin.key_id.clone(),
            issuer_public_key: pin.public_key.clone(),
            audience: pin.audience.clone(),
            terminal_caller: policy.expected_caller.id.clone(),
            holder_public_key: policy.actor.clone(),
            policy_digest: policy.policy_digest,
            credential_endpoint: "http://127.0.0.1:65500/credential".into(),
        }),
        ..Default::default()
    };
    desk.save_connection_settings(String::from_utf8(canonical(&settings).unwrap()).unwrap())
        .unwrap();
    desk
}
#[test]
fn native_approval_persists_outbox_history_and_exports_without_keys() {
    let path = root();
    let desk = open(&path);
    let raw=desk.social_prepare(r#"{"op":"publish","group_id":null,"post":{"kind":"note","book":null,"body":"Selected synthetic note","stars":null,"reading_status":null,"spoiler":false,"reply_to":null}}"#.into()).unwrap();
    let event: SocialEvent = booknook_core::strict_parse(raw.as_bytes()).unwrap();
    let (r, p, _) = receipts::proof(&event, 2);
    desk.social_approve(raw, r, p).unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&desk.social_snapshot(false).unwrap()).unwrap();
    assert_eq!(json["state"]["outbox"].as_object().unwrap().len(), 1);
    assert_eq!(json["feed"][0]["post"]["body"], "Selected synthetic note");
    drop(desk);
    let desk = open(&path);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&desk.social_snapshot(false).unwrap()).unwrap(),
        json
    );
    let output = path.join("social.json");
    desk.social_export(output.to_string_lossy().into()).unwrap();
    let exported = std::fs::read_to_string(&output).unwrap();
    assert!(!exported.contains("transport_secret"));
    let other = root();
    let restored = open(&other);
    restored
        .social_restore(output.to_string_lossy().into())
        .unwrap();
    assert!(
        restored
            .social_restore(output.to_string_lossy().into())
            .is_err()
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&restored.social_snapshot(false).unwrap())
            .unwrap()["state"],
        json["state"]
    );
    restored
        .social_local(serde_json::json!({"op":"cancel_delivery","event_id":event.id}).to_string())
        .unwrap();
    drop(restored);
    let restored = open(&other);
    let cancelled: serde_json::Value =
        serde_json::from_str(&restored.social_snapshot(false).unwrap()).unwrap();
    assert!(cancelled["state"]["outbox"].as_object().unwrap().is_empty());
    assert_eq!(
        cancelled["state"]["publications"],
        json["state"]["publications"]
    );
    assert_eq!(cancelled["state"]["replay"], json["state"]["replay"]);
    drop(desk);
    drop(restored);
    std::fs::remove_dir_all(path).unwrap();
    std::fs::remove_dir_all(other).unwrap();
}
#[test]
fn native_transport_binding_persists_and_local_revocation_prevents_rollback() {
    let path = root();
    let desk = open(&path);
    let secret = desk.social_generate_transport_secret();
    assert_eq!(secret.len(), 32);
    let raw = desk.social_prepare_binding(secret).unwrap();
    let binding: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let (r, p, _) = receipts::receipt(
        &canonical(&binding).unwrap(),
        "nostr.bind",
        binding["version"].as_u64().unwrap(),
        at,
        2,
    );
    let document = desk.social_approve_binding(raw, r, p).unwrap();
    let actor = receipts::actor(2);
    assert_eq!(desk.social_export_binding(actor.clone()).unwrap(), document);
    desk.social_import_binding(document.clone()).unwrap();
    desk.social_revoke_binding(actor).unwrap();
    assert!(desk.social_import_binding(document).is_err());
    let json: serde_json::Value =
        serde_json::from_str(&desk.social_snapshot(false).unwrap()).unwrap();
    assert_eq!(json["bindings"][0]["minimum_version"], 2);
    drop(desk);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn forged_imported_group_projection_cannot_prepare_an_authorized_action() {
    let path = root();
    let desk = open(&path);
    let holder = receipts::actor(2);
    let mut social = SocialState::default();
    let id = format!("{holder}:forged");
    social.groups.insert(
        id.clone(),
        ReadingGroup {
            id: id.clone(),
            owner: holder.clone(),
            name: "Forged group".into(),
            description: String::new(),
            private: false,
            epoch: 1,
            members: [holder].into(),
            moderators: Default::default(),
            banned: Default::default(),
            closed: false,
        },
    );
    social.revision = 1;
    let export = path.join("forged.json");
    std::fs::write(&export, canonical(&social).unwrap()).unwrap();
    desk.social_restore(export.to_string_lossy().into())
        .unwrap();
    assert!(
        desk.social_prepare(serde_json::json!({"op":"close_group","group_id":id}).to_string())
            .is_err()
    );
    assert!(
        desk.social_local(
            serde_json::json!({"op":"block","actor":receipts::actor(3),"blocked":true}).to_string()
        )
        .is_ok()
    );
    drop(desk);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn offline_catalog_recommendations_need_no_castalia_configuration() {
    let path = root();
    let desk = Desk::new(path.to_string_lossy().into()).unwrap();
    desk.social_local(r#"{"op":"set_subjects","subjects":["fantasy"]}"#.into())
        .unwrap();
    let candidates =
        r#"[{"book_id":"catalog:one","title":"Synthetic Book","subjects":["Fantasy"]}]"#;
    let recommendations: serde_json::Value = serde_json::from_str(
        &desk
            .social_recommendations(candidates.into(), "[]".into())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(recommendations[0]["score"], 2);
    assert_eq!(
        recommendations[0]["reasons"][0],
        "Matches your interest in fantasy"
    );
    assert_eq!(
        desk.social_recommendations(candidates.into(), r#"["catalog:one"]"#.into())
            .unwrap(),
        "[]"
    );
    assert!(
        desk.social_prepare(
            r#"{"op":"create_list","id":"test","name":"Unapproved","description":"","books":[]}"#
                .into()
        )
        .is_err()
    );
    drop(desk);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn recommendation_social_evidence_requires_current_auditable_receipts() {
    let mut state = SocialState::default();
    local(
        &mut state,
        LocalSocialCommand::SetSubjects {
            subjects: ["fantasy".into()].into(),
        },
    )
    .unwrap();
    for (seed, action, at) in [
        (
            2,
            SocialAction::Friendship {
                peer: receipts::actor(3),
                requested: true,
            },
            100,
        ),
        (
            3,
            SocialAction::Friendship {
                peer: receipts::actor(2),
                requested: true,
            },
            101,
        ),
        (
            3,
            SocialAction::Publish {
                group_id: None,
                post: Post {
                    kind: PostKind::Rating,
                    book: Some(BookReference {
                        id: "catalog:one".into(),
                        title: "Synthetic Book".into(),
                        author: "Fixture".into(),
                    }),
                    body: String::new(),
                    stars: Some(4),
                    reading_status: None,
                    spoiler: false,
                    reply_to: None,
                },
            },
            102,
        ),
    ] {
        let event = prepare(&state, &receipts::actor(seed), action, at).unwrap();
        let (r, p, policy) = receipts::proof(&event, seed);
        approve(&mut state, event, r, p, &policy, at).unwrap();
    }
    let candidates =
        r#"[{"book_id":"catalog:one","title":"Synthetic Book","subjects":["fantasy"]}]"#;
    let trusted_path = root();
    let trusted = open(&trusted_path);
    let archive = trusted_path.join("social-history.json");
    std::fs::write(&archive, canonical(&state).unwrap()).unwrap();
    trusted
        .social_restore(archive.to_string_lossy().into())
        .unwrap();
    let score = |desk: &Desk| -> u64 {
        let result: serde_json::Value = serde_json::from_str(
            &desk
                .social_recommendations(candidates.into(), "[]".into())
                .unwrap(),
        )
        .unwrap();
        result[0]["score"].as_u64().unwrap()
    };
    assert_eq!(score(&trusted), 6); // Two catalog points plus the independently verified four-star rating.
    let altered = state
        .publications
        .values_mut()
        .find(|p| matches!(p.event.action, SocialAction::Publish { .. }))
        .unwrap();
    if let SocialAction::Publish { post, .. } = &mut altered.event.action {
        post.stars = Some(5);
    }
    state.outbox.get_mut(&altered.event.id).unwrap().publication = altered.clone();
    let untrusted_path = root();
    let untrusted = open(&untrusted_path);
    let archive = untrusted_path.join("altered-history.json");
    std::fs::write(&archive, canonical(&state).unwrap()).unwrap();
    untrusted
        .social_restore(archive.to_string_lossy().into())
        .unwrap();
    assert_eq!(score(&untrusted), 2); // A configured issuer alone does not make an imported rating trustworthy.
    let offline_path = root();
    let offline = Desk::new(offline_path.to_string_lossy().into()).unwrap();
    offline
        .social_restore(archive.to_string_lossy().into())
        .unwrap();
    assert_eq!(score(&offline), 2); // Unconfigured devices use only local interests/catalog evidence too.
    drop(trusted);
    drop(untrusted);
    drop(offline);
    for path in [trusted_path, untrusted_path, offline_path] {
        std::fs::remove_dir_all(path).unwrap();
    }
}
