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

fn prepared_receipt(desk: &Desk, peer: &str) -> (String, String, String) {
    let prepared = value(desk.sync_prepare(peer.into()).unwrap());
    let batch = &prepared["batch"];
    let (request, presentation, _) = proofs::receipt(
        &canonical(batch).unwrap(),
        "sync.send",
        batch["revision"].as_u64().unwrap(),
        at(),
        2,
    );
    (batch["id"].as_str().unwrap().into(), request, presentation)
}

fn receipt_count(desk: &Desk) -> i64 {
    desk.lock()
        .unwrap()
        .connection
        .query_row("SELECT count(*) FROM consumed_receipts", [], |row| {
            row.get(0)
        })
        .unwrap()
}

fn refuse_stale_preparation(
    desk: &Desk,
    batch_id: &str,
    key: &str,
    request: String,
    presentation: String,
) {
    let before_receipts = receipt_count(desk);
    let before_outbox = value(desk.sync_status().unwrap())["outbox"].clone();
    let error = desk
        .sync_approve(batch_id.into(), key.into(), request, presentation)
        .expect_err("Stale or legacy preparation must not queue a transfer");
    assert!(
        error
            .to_string()
            .contains("Create and review a new encrypted snapshot")
    );
    assert_eq!(receipt_count(desk), before_receipts);
    assert_eq!(value(desk.sync_status().unwrap())["outbox"], before_outbox);
    desk.sync_cancel_prepared(batch_id.into()).unwrap();
    let remaining: i64 = desk
        .lock()
        .unwrap()
        .connection
        .query_row(
            "SELECT count(*) FROM sync_prepared WHERE id=?1",
            [batch_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(remaining, 0, "Refused preparation must remain cancellable");
}

#[test]
fn prepared_snapshot_cannot_be_approved_after_rekey_on_another_handle() {
    let ar = Directory::new();
    let br = Directory::new();
    let a = setup(&ar);
    let b = setup(&br);
    let rekey = Desk::new(ar.path()).unwrap();
    let ai = id(&a);
    let bi = id(&b);
    let old = "ab".repeat(32);
    let new = "cd".repeat(32);
    pair(&a, &bi, &old, true);
    add_book(&a);
    let (batch, request, presentation) = prepared_receipt(&a, &bi);
    pair(&rekey, &bi, &new, true);
    refuse_stale_preparation(&a, &batch, &new, request, presentation);

    pair(&b, &ai, &new, true);
    let fresh = approved(&a, &bi, &new);
    let path = ar.0.join("fresh.json");
    a.sync_export(fresh, path.to_string_lossy().into()).unwrap();
    b.sync_receive(path.to_string_lossy().into(), ai, new)
        .unwrap();
    let received: State = serde_json::from_str(&b.snapshot().unwrap()).unwrap();
    assert_eq!(
        received.books.values().next().unwrap().title,
        "Private source book"
    );
}

#[test]
fn prepared_snapshot_cannot_be_approved_after_same_key_disable_and_enable() {
    let root = Directory::new();
    let desk = setup(&root);
    let other = Desk::new(root.path()).unwrap();
    let key = "2c".repeat(32);
    pair(&desk, "peer", &key, true);
    let (batch, request, presentation) = prepared_receipt(&desk, "peer");
    pair(&other, "peer", &key, false);
    pair(&other, "peer", &key, true);
    refuse_stale_preparation(&desk, &batch, &key, request, presentation);
}

#[test]
fn legacy_preparation_is_cancellable_and_existing_approved_outbox_survives() {
    let root = Directory::new();
    let desk = setup(&root);
    let key = "3d".repeat(32);
    pair(&desk, "peer", &key, true);
    let existing = approved(&desk, "peer", &key);
    let (legacy, request, presentation) = prepared_receipt(&desk, "peer");
    // Reopen the old on-disk layout: old prepared rows do not have an independently
    // captured pairing. Already approved packets must remain exportable.
    desk.lock()
        .unwrap()
        .connection
        .execute_batch("DROP TABLE IF EXISTS sync_prepared_pairings;")
        .unwrap();
    drop(desk);
    let desk = Desk::new(root.path()).unwrap();
    refuse_stale_preparation(&desk, &legacy, &key, request, presentation);
    let path = root.0.join("prior-approval.json");
    desk.sync_export(existing, path.to_string_lossy().into())
        .unwrap();
    assert!(path.is_file());
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

fn before_receive_admission(change: impl FnOnce() + 'static) {
    crate::sync_bridge::BEFORE_RECEIVE_ADMISSION.with(|hook| {
        assert!(hook.borrow_mut().replace(Box::new(change)).is_none());
    });
}

fn exported(desk: &Desk, peer: &str, key: &str, path: &std::path::Path) -> String {
    let path = path.to_str().unwrap().to_owned();
    desk.sync_export(approved(desk, peer, key), path.clone())
        .unwrap();
    path
}

#[test]
fn receive_refuses_a_pairing_changed_after_verification() {
    let ar = Directory::new();
    let br = Directory::new();
    let a = setup(&ar);
    let b = setup(&br);
    let editor = Desk::new(br.path()).unwrap();
    let ai = id(&a);
    let bi = id(&b);
    let key = "e5".repeat(32);
    pair(&a, &bi, &key, true);
    pair(&b, &ai, &key, true);
    add_book(&a);
    let state: State = serde_json::from_str(&a.snapshot().unwrap()).unwrap();
    let book_id = state.books.keys().next().unwrap().clone();
    let pdf = ar.0.join("synthetic.pdf");
    let pdf_bytes = b"%PDF-1.4\n1 0 obj\n<<>>\nendobj\n%%EOF\n";
    std::fs::write(&pdf, pdf_bytes).unwrap();
    let imported = a
        .import_content(book_id, pdf.to_str().unwrap().into())
        .unwrap();
    let source: State = serde_json::from_str(&imported).unwrap();
    let path = exported(&a, &bi, &key, &ar.0.join("pairing.json"));
    let before = b.snapshot().unwrap();
    let before_status = value(b.sync_status().unwrap());
    let (race_id, race_key) = (ai.clone(), key.clone());
    before_receive_admission(move || {
        pair(&editor, &race_id, &race_key, false);
        pair(&editor, &race_id, &race_key, true);
    });
    let refusal = b
        .sync_receive(path, ai.clone(), key.clone())
        .expect_err("a changed pairing must stop already-verified input");
    assert!(refusal.to_string().contains("Device pairing changed"));
    assert_eq!(b.snapshot().unwrap(), before);
    assert_eq!(
        value(b.sync_status().unwrap())["conflicts"],
        before_status["conflicts"]
    );
    assert!(
        std::fs::read_dir(br.0.join("assets"))
            .unwrap()
            .next()
            .is_none()
    );
    {
        let store = b.lock().unwrap();
        for table in ["sync_received", "sync_bases"] {
            let count: i64 = store
                .connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0, "{table} must remain unchanged");
        }
    }

    let fresh = exported(&a, &bi, &key, &ar.0.join("fresh-after-pairing.json"));
    let applied = value(
        b.sync_receive(fresh.clone(), ai.clone(), key.clone())
            .unwrap(),
    );
    assert_eq!(applied["duplicate"], false);
    let accepted: State = serde_json::from_str(&b.snapshot().unwrap()).unwrap();
    assert_eq!(accepted.books, source.books);
    assert_eq!(accepted.editions, source.editions);
    let asset = source
        .editions
        .values()
        .find_map(|edition| edition.assets.first())
        .unwrap();
    let directory = b.asset_directory(asset.id.clone()).unwrap();
    assert_eq!(
        std::fs::read(std::path::Path::new(&directory).join("book.pdf")).unwrap(),
        pdf_bytes
    );
    let before_replay = b.snapshot().unwrap();
    assert_eq!(
        value(b.sync_receive(fresh, ai, key).unwrap())["duplicate"],
        true
    );
    assert_eq!(b.snapshot().unwrap(), before_replay);
}

#[test]
fn receive_refuses_an_authority_changed_after_verification() {
    let ar = Directory::new();
    let br = Directory::new();
    let a = setup(&ar);
    let b = setup(&br);
    let editor = Desk::new(br.path()).unwrap();
    let ai = id(&a);
    let bi = id(&b);
    let key = "e6".repeat(32);
    pair(&a, &bi, &key, true);
    pair(&b, &ai, &key, true);
    add_book(&a);
    let path = exported(&a, &bi, &key, &ar.0.join("authority.json"));
    let before = b.snapshot().unwrap();
    let original = b.connection_settings().unwrap();
    let mut changed: ConnectionSettings = serde_json::from_str(&original).unwrap();
    changed.castalia.as_mut().unwrap().policy_digest = "9e".repeat(32);
    before_receive_admission(move || {
        editor
            .save_connection_settings(serde_json::to_string(&changed).unwrap())
            .unwrap();
    });
    let refusal = b
        .sync_receive(path.clone(), ai.clone(), key.clone())
        .expect_err("new authority must fence an already-verified packet");
    assert!(refusal.to_string().contains("Castalia authority changed"));
    assert_eq!(b.snapshot().unwrap(), before);
    b.save_connection_settings(original).unwrap();
    assert_eq!(
        value(b.sync_receive(path, ai, key).unwrap())["duplicate"],
        false
    );
}

#[test]
fn receive_cannot_replace_a_newer_baseline_committed_by_another_handle() {
    let ar = Directory::new();
    let br = Directory::new();
    let a = setup(&ar);
    let b = setup(&br);
    let other = Desk::new(br.path()).unwrap();
    let ai = id(&a);
    let bi = id(&b);
    let key = "e7".repeat(32);
    pair(&a, &bi, &key, true);
    pair(&b, &ai, &key, true);
    add_book(&a);
    let old = exported(&a, &bi, &key, &ar.0.join("older.json"));
    a.execute(
        json!({"op":"add_book","title":"Newer source book","author":"Writer","at":at()})
            .to_string(),
    )
    .unwrap();
    let new = exported(&a, &bi, &key, &ar.0.join("newer.json"));
    let (race_id, race_key) = (ai.clone(), key.clone());
    let newer_result = std::rc::Rc::new(std::cell::RefCell::new(None));
    let result_for_hook = newer_result.clone();
    before_receive_admission(move || {
        let result = value(other.sync_receive(new, race_id, race_key).unwrap());
        assert_eq!(result["duplicate"], false);
        *result_for_hook.borrow_mut() = Some(other.snapshot().unwrap());
    });
    let refusal = b
        .sync_receive(old, ai.clone(), key)
        .expect_err("an older in-flight packet must not regress the baseline");
    assert!(refusal.to_string().contains("older device snapshot"));
    assert_eq!(
        b.snapshot().unwrap(),
        newer_result.borrow().as_ref().unwrap().as_str()
    );
    let store = b.lock().unwrap();
    let base: String = store
        .connection
        .query_row("SELECT json FROM sync_bases WHERE peer=?1", [&ai], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(value(base), value(a.snapshot().unwrap()));
    let count: i64 = store
        .connection
        .query_row("SELECT count(*) FROM sync_received", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}
