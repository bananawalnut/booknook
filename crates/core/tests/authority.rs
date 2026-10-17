use booknook_core::{authority::*, model::*, *};
use ed25519_dalek::SigningKey;
#[path = "fixtures/receipts.rs"]
mod fixtures;
use fixtures::receipt;

#[test]
fn valid_v2_receipt_binds_exact_payload_scope_version_and_expiry() {
    let (request, presentation, policy) = receipt(b"selected", "social.publish", 7);
    let auth = verify_receipt(&request, &presentation, &policy, b"selected", 7, 110).unwrap();
    assert!(
        auth.require("social.publish", &policy.actor, b"selected", 110)
            .is_ok()
    );
    for (payload, version, now) in [
        (b"changed".as_slice(), 7, 110),
        (b"selected".as_slice(), 8, 110),
        (b"selected".as_slice(), 7, 161),
    ] {
        assert!(verify_receipt(&request, &presentation, &policy, payload, version, now).is_err())
    }
    let mut revoked = policy.clone();
    revoked.pins.clear();
    assert!(verify_receipt(&request, &presentation, &revoked, b"selected", 7, 110).is_err());
}
#[test]
fn forged_signature_duplicate_keys_and_unsafe_disclosure_rejected() {
    let (request, presentation, policy) = receipt(b"x", "social.publish", 0);
    let mut value: serde_json::Value = serde_json::from_slice(&presentation).unwrap();
    value["signature"] = "00".repeat(64).into();
    assert!(verify_receipt(&request, &canonical(&value).unwrap(), &policy, b"x", 0, 110).is_err());
    let mut value: serde_json::Value = serde_json::from_slice(&request).unwrap();
    value["disclosure"]["title"] = "Hidden\u{202e}title".into();
    assert!(
        verify_receipt(
            &canonical(&value).unwrap(),
            &presentation,
            &policy,
            b"x",
            0,
            110
        )
        .is_err()
    );
    let duplicated = String::from_utf8(request)
        .unwrap()
        .replacen('{', "{\"schema\":\"wrong\",", 1);
    assert!(verify_receipt(duplicated.as_bytes(), &presentation, &policy, b"x", 0, 110).is_err());
}
#[test]
fn encrypted_sync_rejects_tampering_recipient_changes_and_postapproval_edits() {
    let holder = hex::encode(SigningKey::from_bytes(&[2; 32]).verifying_key().as_bytes());
    let batch = sync::prepare(&holder, 3, vec!["device-b".into()], b"private notebook").unwrap();
    let payload = canonical(&batch).unwrap();
    let (r, p, policy) = receipt(&payload, "sync.send", 3);
    let auth = verify_receipt(&r, &p, &policy, &payload, 3, 110).unwrap();
    let key = [8; 32];
    assert!(sync::seal(batch.clone(), b"changed", &key, &auth, 110).is_err());
    let envelope = sync::seal(batch, b"private notebook", &key, &auth, 110).unwrap();
    assert_eq!(
        sync::open(&envelope, &key, "device-b").unwrap(),
        b"private notebook"
    );
    assert!(sync::open(&envelope, &[9; 32], "device-b").is_err());
    assert!(sync::open(&envelope, &key, "stranger").is_err());
    let mut modified = envelope;
    modified.batch.recipients.push("stranger".into());
    assert!(sync::open(&modified, &key, "stranger").is_err());
}
#[test]
fn group_membership_epoch_is_checked_before_accepting_activity() {
    let actor = hex::encode(SigningKey::from_bytes(&[2; 32]).verifying_key().as_bytes());
    let activity = Activity {
        id: "a".into(),
        actor: actor.clone(),
        published_at: 110,
        kind: "discussion".into(),
        book_id: None,
        body: "A thought".into(),
        spoiler: false,
        group_id: Some("g".into()),
        group_epoch: Some(1),
    };
    let bytes = canonical(&activity).unwrap();
    let (r, p, policy) = receipt(&bytes, "social.publish", 1);
    let auth = verify_receipt(&r, &p, &policy, &bytes, 1, 110).unwrap();
    let mut s = State::default();
    s.groups.insert(
        "g".into(),
        Group {
            id: "g".into(),
            owner: actor.clone(),
            name: "Readers".into(),
            private: false,
            epoch: 1,
            members: [actor.clone()].into(),
            moderators: Default::default(),
        },
    );
    assert!(social::receive(&mut s, activity.clone(), &auth, 110).unwrap());
    assert!(!social::receive(&mut s, activity.clone(), &auth, 110).unwrap());
    s.activity.clear();
    s.groups.get_mut("g").unwrap().epoch = 2;
    assert!(social::receive(&mut s, activity, &auth, 110).is_err());
}

#[test]
fn membership_changes_require_exact_fresh_owner_approval() {
    let owner = hex::encode(SigningKey::from_bytes(&[2; 32]).verifying_key().as_bytes());
    let mut group = Group {
        id: "g".into(),
        owner: owner.clone(),
        name: "Readers".into(),
        private: true,
        epoch: 1,
        members: [owner.clone(), "old-member".into()].into(),
        moderators: ["old-member".into()].into(),
    };
    let change = social::MembershipChange {
        group_id: "g".into(),
        owner,
        expected_epoch: 1,
        members: [group.owner.clone(), "new-member".into()].into(),
    };
    let bytes = canonical(&change).unwrap();
    let (r, p, policy) = receipt(&bytes, "group.membership", 1);
    let auth = verify_receipt(&r, &p, &policy, &bytes, 1, 110).unwrap();
    let before = group.clone();
    assert!(social::update_members(&mut group, &change, &auth, 161).is_err());
    let mut tampered = change.clone();
    tampered.members.insert("stranger".into());
    assert!(social::update_members(&mut group, &tampered, &auth, 110).is_err());
    assert_eq!(group, before);
    social::update_members(&mut group, &change, &auth, 110).unwrap();
    assert_eq!(group.epoch, 2);
    assert!(group.moderators.is_empty());
    assert!(social::update_members(&mut group, &change, &auth, 110).is_err());
}
