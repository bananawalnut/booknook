// SPDX-License-Identifier: MIT OR Apache-2.0
use crate::Desk;
use booknook_core::{
    canonical, digest,
    model::State,
    settings::{CastaliaSettings, ConnectionSettings},
};
use serde_json::{Value, json};
use std::path::PathBuf;
#[allow(dead_code)]
#[path = "../tests/fixtures/social_receipts.rs"]
mod proofs;
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("booknook-sync-{}", booknook_core::new_id()));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn path(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn at() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
fn value(raw: String) -> Value {
    serde_json::from_str(&raw).unwrap()
}
fn setup(root: &Directory) -> Desk {
    let d = Desk::new(root.path()).unwrap();
    let (_, _, p) = proofs::receipt(b"setup", "sync.send", 0, at(), 2);
    let settings = ConnectionSettings {
        castalia: Some(CastaliaSettings {
            issuer: p.pins[0].issuer.clone(),
            key_id: p.pins[0].key_id.clone(),
            issuer_public_key: p.pins[0].public_key.clone(),
            audience: p.audience,
            terminal_caller: p.expected_caller.id,
            holder_public_key: p.actor,
            policy_digest: p.policy_digest,
            credential_endpoint: String::new(),
        }),
        ..Default::default()
    };
    d.save_connection_settings(serde_json::to_string(&settings).unwrap())
        .unwrap();
    d
}
fn id(d: &Desk) -> String {
    value(d.sync_status().unwrap())["device_id"]
        .as_str()
        .unwrap()
        .into()
}
fn pair(d: &Desk, peer: &str, key: &str, enabled: bool) {
    let p = d
        .sync_prepare_peer(peer.into(), "Test peer".into(), key.into(), enabled)
        .unwrap();
    let v: Value = serde_json::from_str(&p).unwrap();
    let (r, s, _) = proofs::receipt(
        &canonical(&v).unwrap(),
        "device.enroll",
        v["version"].as_u64().unwrap(),
        at(),
        2,
    );
    d.sync_approve_peer(p, r, s).unwrap();
}
fn approved(d: &Desk, peer: &str, key: &str) -> String {
    let result = value(d.sync_prepare(peer.into()).unwrap());
    let batch = &result["batch"];
    let (r, s, _) = proofs::receipt(
        &canonical(batch).unwrap(),
        "sync.send",
        batch["revision"].as_u64().unwrap(),
        at(),
        2,
    );
    d.sync_approve(batch["id"].as_str().unwrap().into(), key.into(), r, s)
        .unwrap()
}
fn add_book(d: &Desk) {
    d.execute(
        json!({"op":"add_book","title":"Private source book","author":"Writer","at":at()})
            .to_string(),
    )
    .unwrap();
}

#[test]
fn approved_outbox_and_two_replica_data_survive_reopen_duplicate_receive_and_corruption() {
    let ar = Directory::new();
    let br = Directory::new();
    let a = setup(&ar);
    let b = setup(&br);
    let ai = id(&a);
    let bi = id(&b);
    let key = "12".repeat(32);
    pair(&a, &bi, &key, true);
    pair(&b, &ai, &key, true);
    add_book(&a);
    let batch = approved(&a, &bi, &key);
    drop(a);
    let a = Desk::new(ar.path()).unwrap();
    assert_eq!(
        value(a.sync_status().unwrap())["outbox"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let path = ar.0.join("batch.json");
    a.sync_export(batch.clone(), path.to_string_lossy().into())
        .unwrap();
    let plain = std::fs::read_to_string(&path).unwrap();
    assert!(!plain.contains("Private source book"));
    let merged = value(
        b.sync_receive(path.to_string_lossy().into(), ai.clone(), key.clone())
            .unwrap(),
    );
    assert_eq!(merged["duplicate"], false);
    let before = b.snapshot().unwrap();
    assert_eq!(
        value(
            b.sync_receive(path.to_string_lossy().into(), ai.clone(), key.clone())
                .unwrap()
        )["duplicate"],
        true
    );
    assert_eq!(b.snapshot().unwrap(), before);
    drop(b);
    let b = Desk::new(br.path()).unwrap();
    assert_eq!(b.snapshot().unwrap(), before);
    let mut bad: Value = serde_json::from_str(&plain).unwrap();
    bad["envelope"]["ciphertext"] = "AA==".into();
    std::fs::write(&path, serde_json::to_vec(&bad).unwrap()).unwrap();
    assert!(
        b.sync_receive(path.to_string_lossy().into(), ai, key)
            .is_err()
    );
    assert_eq!(b.snapshot().unwrap(), before);
}
#[test]
fn native_transfer_wire_opens_in_portable_core_and_core_packets_keep_native_replay_guards() {
    use booknook_core::sync;
    let ar = Directory::new();
    let br = Directory::new();
    let a = setup(&ar);
    let b = setup(&br);
    let ai = id(&a);
    let bi = id(&b);
    let key = "12".repeat(32);
    pair(&a, &bi, &key, true);
    pair(&b, &ai, &key, true);
    add_book(&a);
    let batch_id = approved(&a, &bi, &key);
    let path = ar.0.join("portable-packet.json");
    a.sync_export(batch_id, path.to_string_lossy().into())
        .unwrap();
    let packet: sync::ApprovedPacket =
        booknook_core::strict_parse(&std::fs::read(&path).unwrap()).unwrap();
    let policy = a.policy("sync.send", None).unwrap();
    let opened = sync::open_packet(&packet, &[0x12; 32], &bi, &policy, at()).unwrap();
    let content: Value = serde_json::from_slice(&opened.plaintext).unwrap();
    assert_eq!(content["schema"], "booknook.sync-content.v1");
    assert_eq!(content["source_device"], ai);
    let batch = sync::prepare(
        &policy.actor,
        packet.envelope.batch.revision,
        vec![bi.clone()],
        &opened.plaintext,
    )
    .unwrap();
    let when = at();
    let (r, p, _) = proofs::receipt(
        &canonical(&batch).unwrap(),
        "sync.send",
        batch.revision,
        when,
        2,
    );
    let sealed = sync::seal_packet(
        batch.clone(),
        &opened.plaintext,
        &[0x12; 32],
        r.clone(),
        p.clone(),
        &policy,
        when,
    )
    .unwrap();
    std::fs::write(&path, canonical(&sealed.packet).unwrap()).unwrap();
    assert_eq!(
        value(
            b.sync_receive(path.to_string_lossy().into(), ai.clone(), key.clone())
                .unwrap()
        )["duplicate"],
        false
    );
    let before = b.snapshot().unwrap();
    assert_eq!(
        value(
            b.sync_receive(path.to_string_lossy().into(), ai.clone(), key.clone())
                .unwrap()
        )["duplicate"],
        true
    );
    assert_eq!(b.snapshot().unwrap(), before);
    // Re-encryption is not a retry: same batch ID with a different nonce is rejected.
    let altered =
        sync::seal_packet(batch, &opened.plaintext, &[0x12; 32], r, p, &policy, when).unwrap();
    assert_eq!(sealed.replay_key, altered.replay_key);
    assert_ne!(sealed.packet.envelope.nonce, altered.packet.envelope.nonce);
    std::fs::write(&path, canonical(&altered.packet).unwrap()).unwrap();
    assert!(
        b.sync_receive(path.to_string_lossy().into(), ai, key)
            .is_err()
    );
    assert_eq!(b.snapshot().unwrap(), before);
}

#[test]
fn denied_expired_and_replayed_receipts_never_create_an_outbox() {
    let root = Directory::new();
    let desk = setup(&root);
    let key = "34".repeat(32);
    pair(&desk, "peer-device", &key, true);
    add_book(&desk);
    let prepared = value(desk.sync_prepare("peer-device".into()).unwrap());
    let batch = &prepared["batch"];
    let batch_id = batch["id"].as_str().unwrap().to_string();
    let payload = canonical(batch).unwrap();
    let revision = batch["revision"].as_u64().unwrap();
    let (r, s, _) = proofs::receipt(&payload, "sync.send", revision, at() - 180, 2);
    assert!(
        desk.sync_approve(batch_id.clone(), key.clone(), r, s)
            .is_err()
    );
    let (r, s, _) = proofs::receipt(&payload, "sync.send", revision, at(), 3);
    assert!(
        desk.sync_approve(batch_id.clone(), key.clone(), r, s)
            .is_err()
    );
    assert!(
        value(desk.sync_status().unwrap())["outbox"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let (r, s, _) = proofs::receipt(&payload, "sync.send", revision, at(), 2);
    desk.sync_approve(batch_id.clone(), key.clone(), r.clone(), s.clone())
        .unwrap();
    assert!(desk.sync_approve(batch_id, key, r, s).is_err());
    assert_eq!(
        value(desk.sync_status().unwrap())["outbox"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn disabled_peer_blocks_preparation_receiving_and_already_approved_export() {
    let ar = Directory::new();
    let br = Directory::new();
    let a = setup(&ar);
    let b = setup(&br);
    let ai = id(&a);
    let bi = id(&b);
    let key = "56".repeat(32);
    pair(&a, &bi, &key, true);
    pair(&b, &ai, &key, true);
    add_book(&a);
    let batch = approved(&a, &bi, &key);
    let path = ar.0.join("packet.json");
    a.sync_export(batch.clone(), path.to_string_lossy().into())
        .unwrap();
    pair(&a, &bi, &key, false);
    assert!(a.sync_prepare(bi).is_err());
    assert!(a.sync_export(batch, path.to_string_lossy().into()).is_err());
    pair(&b, &ai, &key, false);
    let before = b.snapshot().unwrap();
    assert!(
        b.sync_receive(path.to_string_lossy().into(), ai, key)
            .is_err()
    );
    assert_eq!(b.snapshot().unwrap(), before);
}
#[test]
fn corrupt_local_asset_refuses_snapshot_without_committing_prepared_data() {
    let root = Directory::new();
    let d = setup(&root);
    let key = "78".repeat(32);
    pair(&d, "peer", &key, true);
    add_book(&d);
    let state: State = serde_json::from_str(&d.snapshot().unwrap()).unwrap();
    let book = state.books.keys().next().unwrap();
    let path = root.0.join("book.pdf");
    std::fs::write(&path, b"%PDF-1.4\n%%EOF\n").unwrap();
    d.import_content(book.clone(), path.to_string_lossy().into())
        .unwrap();
    let state: State = serde_json::from_str(&d.snapshot().unwrap()).unwrap();
    let asset = state
        .editions
        .values()
        .flat_map(|e| &e.assets)
        .next()
        .unwrap();
    let stored = PathBuf::from(d.asset_directory(asset.id.clone()).unwrap()).join("book.pdf");
    std::fs::write(stored, b"corrupted").unwrap();
    assert!(d.sync_prepare("peer".into()).is_err());
    assert!(
        value(d.sync_status().unwrap())["outbox"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn direct_pairing_rekey_cannot_bypass_pending_outbox_guard() {
    let root = Directory::new();
    let d = setup(&root);
    let key = "90".repeat(32);
    pair(&d, "peer", &key, true);
    add_book(&d);
    approved(&d, "peer", &key);
    assert!(
        d.sync_prepare_peer("peer".into(), "Rekey".into(), "91".repeat(32), true)
            .is_err()
    );
    let proposed = json!({"schema":"booknook.device-pairing.v1","local_device":id(&d),"device_id":"peer","owner":proofs::actor(2),"label":"Rekey","key_id":digest(&[0x91;32]),"version":2,"enabled":true});
    let (r, s, _) = proofs::receipt(&canonical(&proposed).unwrap(), "device.enroll", 2, at(), 2);
    assert!(
        d.sync_approve_peer(proposed.to_string(), r, s).is_err(),
        "Approval must recheck the same invariant as preparation"
    );
}

#[test]
fn same_source_revision_with_different_state_is_rejected_without_changing_receiver() {
    let ar = Directory::new();
    let br = Directory::new();
    let a = setup(&ar);
    let b = setup(&br);
    let ai = id(&a);
    let bi = id(&b);
    let key = "a2".repeat(32);
    pair(&a, &bi, &key, true);
    pair(&b, &ai, &key, true);
    add_book(&a);
    let batch = approved(&a, &bi, &key);
    let path = ar.0.join("first.json");
    a.sync_export(batch, path.to_string_lossy().into()).unwrap();
    b.sync_receive(path.to_string_lossy().into(), ai.clone(), key.clone())
        .unwrap();
    // Simulate a restored/corrupt source reusing its durable revision for divergent content.
    {
        let store = a.lock().unwrap();
        let mut state = store.load().unwrap();
        state.books.values_mut().next().unwrap().title =
            "Different bytes at the same revision".into();
        store
            .connection
            .execute(
                "UPDATE desk SET json=?1 WHERE singleton=1",
                [serde_json::to_string(&state).unwrap()],
            )
            .unwrap();
    }
    let batch = approved(&a, &bi, &key);
    let path = ar.0.join("second.json");
    a.sync_export(batch, path.to_string_lossy().into()).unwrap();
    let before = b.snapshot().unwrap();
    assert!(
        b.sync_receive(path.to_string_lossy().into(), ai, key)
            .is_err()
    );
    assert_eq!(b.snapshot().unwrap(), before);
}
