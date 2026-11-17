// SPDX-License-Identifier: MIT OR Apache-2.0
use crate::Desk;
use booknook_core::{
    canonical, digest,
    model::{
        Alignment, CapturedReadingContext, CharacterInterpretation, Location, ProgressObservation,
        State,
    },
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
fn approved_sync_preserves_divergent_card_captures_and_replay_after_reopen() {
    let sender_root = Directory::new();
    let receiver_root = Directory::new();
    let sender = setup(&sender_root);
    let receiver = setup(&receiver_root);
    let sender_id = id(&sender);
    let receiver_id = id(&receiver);
    assert_ne!(sender_id, receiver_id);
    let key = "19".repeat(32);
    pair(&sender, &receiver_id, &key, true);
    pair(&receiver, &sender_id, &key, true);

    let execute = |desk: &Desk, command: Value| {
        value(desk.execute(command.to_string()).unwrap())["result"].clone()
    };
    let snapshot = |desk: &Desk| serde_json::from_str::<State>(&desk.snapshot().unwrap()).unwrap();
    add_book(&sender);
    let state = snapshot(&sender);
    let notebook = state.notebooks.values().next().unwrap();
    let edition_id = state.editions.keys().next().unwrap();
    let added = execute(
        &sender,
        json!({
            "op":"add_character", "notebook_id":notebook.id, "block_id":notebook.blocks[0].id,
            "name":"Mara", "at":at(),
        }),
    );
    let card_id = added["card_id"].as_str().unwrap();
    let original = snapshot(&sender).cards[card_id].clone();
    let parent = &original.versions[0].id;
    let baseline = exported(
        &sender,
        &receiver_id,
        &key,
        &sender_root.0.join("baseline.json"),
    );
    let received = value(
        receiver
            .sync_receive(baseline, sender_id.clone(), key.clone())
            .unwrap(),
    );
    assert_eq!(received["duplicate"], false);
    assert_eq!(snapshot(&receiver).cards[card_id], original);

    let capture = |chapter, quote: &str, milliseconds, observed_at| {
        let positions = vec![
            Location::Text {
                edition_id: edition_id.clone(),
                resource: format!("OPS/chapter-{chapter}.xhtml"),
                quote: quote.into(),
                prefix: "Before ".into(),
                suffix: " after.".into(),
            },
            Location::Audio {
                provider: "audible".into(),
                source_id: "sync-fixture".into(),
                milliseconds,
                chapter: None,
            },
        ];
        let position_observations = positions
            .iter()
            .enumerate()
            .map(|(index, location)| ProgressObservation {
                id: format!("chapter-{chapter}-source-{index}"),
                pass_id: notebook.pass_id.clone(),
                location: location.clone(),
                observed_at,
                received_at: observed_at,
                alignment: Alignment::Unmatched,
                provider_connection_id: (index == 1).then(|| "audible-fixture".into()),
                audio_metadata: None,
            })
            .collect();
        CapturedReadingContext {
            pass_id: notebook.pass_id.clone(),
            positions,
            position_observations,
        }
    };
    let first_time = at();
    let early = capture(3, "Mara's café 日本語", 5_000, first_time);
    let independent = capture(7, "Mara changes her mind", 9_000, first_time + 2);
    let later = capture(12, "Mara revisits the room", 12_000, first_time + 4);
    for observation in &later.position_observations {
        execute(
            &sender,
            json!({"op":"observe_position","observation":observation}),
        );
    }

    let save = |desk: &Desk, context: &CapturedReadingContext, description: &str, saved_at| {
        let interpretation = CharacterInterpretation {
            description: description.into(),
            passages: vec![context.positions[0].clone()],
            ..Default::default()
        };
        let response = execute(
            desk,
            json!({
                "op":"save_card_with_context", "card_id":card_id, "parents":[parent],
                "at":saved_at, "interpretation":interpretation, "capture_context":context,
            }),
        );
        let state = snapshot(desk);
        let version = state.cards[card_id].versions[1].clone();
        assert_eq!(Some(version.id.as_str()), response["version_id"].as_str());
        assert_eq!(version.parents.as_slice(), std::slice::from_ref(parent));
        assert_eq!(version.pass_id, context.pass_id);
        assert_eq!(version.at, saved_at);
        assert_eq!(version.interpretation, interpretation);
        assert_eq!(version.positions, context.positions);
        assert_eq!(version.position_observations, context.position_observations);
        version
    };
    let sender_version = save(
        &sender,
        &early,
        "Earlier private interpretation",
        first_time + 1,
    );
    let receiver_version = save(
        &receiver,
        &independent,
        "Independent interpretation",
        first_time + 3,
    );

    let path = exported(
        &sender,
        &receiver_id,
        &key,
        &sender_root.0.join("branches.json"),
    );
    let wire = std::fs::read_to_string(&path).unwrap();
    let packet: Value = serde_json::from_str(&wire).unwrap();
    assert!(
        packet["envelope"]["ciphertext"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
    );
    assert!(!wire.contains(&sender_version.interpretation.description));
    assert!(!wire.contains("Mara's café 日本語"));

    let received = value(
        receiver
            .sync_receive(path.clone(), sender_id.clone(), key.clone())
            .unwrap(),
    );
    assert_eq!(received["duplicate"], false);
    assert_eq!(received["conflicts"], 0);
    let merged = snapshot(&receiver);
    let card = &merged.cards[card_id];
    assert_eq!(card.versions.len(), 3);
    assert!(card.versions.contains(&original.versions[0]));
    let heads = card.heads();
    assert_eq!(heads.len(), 2);
    assert!(heads.contains(&&sender_version));
    assert!(heads.contains(&&receiver_version));
    for observation in &later.position_observations {
        assert_eq!(merged.positions.get(&observation.id), Some(observation));
    }

    drop(receiver);
    let reopened = Desk::new(receiver_root.path()).unwrap();
    assert_eq!(snapshot(&reopened), merged);
    assert_eq!(
        value(reopened.sync_receive(path, sender_id, key).unwrap())["duplicate"],
        true,
    );
    assert_eq!(snapshot(&reopened), merged);
}

#[test]
fn approved_sync_transfers_original_mobi_and_regenerates_the_same_reader_resource() {
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
    let state: State = serde_json::from_str(&a.snapshot().unwrap()).unwrap();
    let book = state.books.keys().next().unwrap();
    let original = include_bytes!("../tests/fixtures/mobi6-reader.mobi");
    let file = ar.0.join("source.mobi");
    std::fs::write(&file, original).unwrap();
    a.import_content(book.clone(), file.to_string_lossy().into_owned())
        .unwrap();
    let packet_path = exported(&a, &bi, &key, &ar.0.join("mobi-sync.json"));
    assert_eq!(
        value(b.sync_receive(packet_path, ai, key).unwrap())["duplicate"],
        false
    );
    let asset = digest(original);
    let source = PathBuf::from(a.asset_directory(asset.clone()).unwrap());
    let received = PathBuf::from(b.asset_directory(asset).unwrap());
    assert_eq!(
        std::fs::read(received.join("original.mobi")).unwrap(),
        original.as_slice()
    );
    assert_eq!(
        std::fs::read(received.join("mobi6-v1/book.xhtml")).unwrap(),
        std::fs::read(source.join("mobi6-v1/book.xhtml")).unwrap()
    );
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

#[test]
fn encrypted_two_replica_margin_notes_converge_after_offline_appends() {
    let ar = Directory::new();
    let br = Directory::new();
    let a = setup(&ar);
    let b = setup(&br);
    let ai = id(&a);
    let bi = id(&b);
    assert_ne!(ai, bi);
    let key = "7b".repeat(32);
    pair(&a, &bi, &key, true);
    pair(&b, &ai, &key, true);

    let state = |desk: &Desk| serde_json::from_str::<State>(&desk.snapshot().unwrap()).unwrap();
    add_book(&a);
    let initial = state(&a);
    let notebook = initial.notebooks.values().next().unwrap();
    let notebook_id = &notebook.id;
    let edition_id = initial.editions.keys().next().unwrap();
    let order = |desk: &Desk| {
        state(desk).notebooks[notebook_id]
            .blocks
            .iter()
            .map(|block| block.id.clone())
            .collect::<Vec<_>>()
    };
    let append = |desk: &Desk, chapter, text: &str, observed_at| {
        // Same add_block shape as the app's explicit Margin note creation.
        let context = CapturedReadingContext {
            pass_id: notebook.pass_id.clone(),
            positions: vec![Location::Text {
                edition_id: edition_id.clone(),
                resource: format!("OPS/chapter-{chapter}.xhtml"),
                quote: text.into(),
                prefix: String::new(),
                suffix: String::new(),
            }],
            position_observations: vec![],
        };
        let created = value(
            desk.execute(
                json!({"op":"add_block", "notebook_id":notebook_id, "face":"text",
                    "at":observed_at, "capture_context":context})
                .to_string(),
            )
            .unwrap(),
        );
        let block_id = created["result"]["block_id"].as_str().unwrap();
        let added = state(desk);
        let block = added.notebooks[notebook_id]
            .blocks
            .iter()
            .find(|block| block.id == block_id)
            .unwrap();
        desk.execute(
            json!({"op":"edit_block", "notebook_id":notebook_id, "block_id":block_id,
                "text":text, "base_update":block.update, "at":observed_at})
            .to_string(),
        )
        .unwrap();
        let saved = state(desk).notebooks[notebook_id]
            .blocks
            .iter()
            .find(|block| block.id == block_id)
            .unwrap()
            .clone();
        assert_eq!(saved.text, text);
        let origin = &saved.note.as_ref().unwrap().created;
        assert_eq!(origin.at, Some(observed_at));
        assert_eq!(origin.positions, context.positions);
        saved
    };
    let receive = |desk: &Desk, path: String, peer: &str, duplicate| {
        let result = value(desk.sync_receive(path, peer.into(), key.clone()).unwrap());
        assert_eq!(result["duplicate"], duplicate);
        assert_eq!(result["conflicts"], 0);
    };

    let touched_at = at();
    let anchor = append(&a, 1, "Shared synthetic baseline", touched_at - 1);
    let baseline = order(&a);
    receive(
        &b,
        exported(&a, &bi, &key, &ar.0.join("baseline-a.json")),
        &ai,
        false,
    );
    receive(
        &a,
        exported(&b, &ai, &key, &br.0.join("baseline-b.json")),
        &bi,
        false,
    );
    assert_eq!(order(&a), baseline);
    assert_eq!(order(&b), baseline);
    assert_eq!(baseline.last(), Some(&anchor.id));

    // Both append independently at the same second. Neither sees the other's row
    // until both approved snapshots have been exported.
    let left = append(&a, 2, "Replica A: café", touched_at);
    let right = append(&b, 3, "Replica B: 日本語", touched_at);
    assert_ne!(left.id, right.id);
    assert_eq!(
        order(&a),
        [baseline.clone(), vec![left.id.clone()]].concat()
    );
    assert_eq!(
        order(&b),
        [baseline.clone(), vec![right.id.clone()]].concat()
    );
    let from_a = exported(&a, &bi, &key, &ar.0.join("offline-a.json"));
    let from_b = exported(&b, &ai, &key, &br.0.join("offline-b.json"));
    for (path, text) in [(&from_a, &left.text), (&from_b, &right.text)] {
        assert!(!std::fs::read_to_string(path).unwrap().contains(text));
    }
    receive(&a, from_b.clone(), &bi, false);
    receive(&b, from_a.clone(), &ai, false);
    let before_reopen_a = a.snapshot().unwrap();
    let before_reopen_b = b.snapshot().unwrap();
    drop((a, b));
    let a = Desk::new(ar.path()).unwrap();
    let b = Desk::new(br.path()).unwrap();
    assert_eq!(a.snapshot().unwrap(), before_reopen_a);
    assert_eq!(b.snapshot().unwrap(), before_reopen_b);
    receive(&a, from_b, &bi, true);
    receive(&b, from_a, &ai, true);
    assert_eq!(a.snapshot().unwrap(), before_reopen_a);
    assert_eq!(b.snapshot().unwrap(), before_reopen_b);

    for desk in [&a, &b] {
        let snapshot = state(desk);
        let blocks = &snapshot.notebooks[notebook_id].blocks;
        assert_eq!(blocks.len(), baseline.len() + 2);
        for expected in [&left, &right] {
            assert_eq!(
                blocks.iter().find(|block| block.id == expected.id),
                Some(expected),
                "An independent Margin note and its first capture must survive byte-for-byte",
            );
        }
        assert!(order(desk).starts_with(&baseline));
    }
    assert_eq!(
        order(&a),
        order(&b),
        "Durable Margin note order must converge across opposite approved encrypted receives",
    );
}

#[test]
fn sync_status_projects_restored_notebook_order_reviews_without_writing_conflicts() {
    let source_root = Directory::new();
    let target_root = Directory::new();
    let source = Desk::new(source_root.path()).unwrap();
    let target = Desk::new(target_root.path()).unwrap();
    assert_eq!(
        value(target.sync_status().unwrap())["note_order_reviews"],
        json!([])
    );
    add_book(&source);
    let created: State = serde_json::from_str(&source.snapshot().unwrap()).unwrap();
    let notebook_id = created.notebooks.keys().next().unwrap().clone();
    for stamp in [2, 3] {
        source
            .execute(json!({"op":"add_block","notebook_id":notebook_id,"at":stamp}).to_string())
            .unwrap();
    }
    let left: State = serde_json::from_str(&source.snapshot().unwrap()).unwrap();
    let mut right = left.clone();
    right
        .notebooks
        .get_mut(&notebook_id)
        .unwrap()
        .blocks
        .swap(1, 2);
    let ids = |state: &State| {
        state.notebooks[&notebook_id]
            .blocks
            .iter()
            .map(|block| block.id.clone())
            .collect::<Vec<_>>()
    };
    let expected_paths = std::collections::BTreeSet::from([ids(&left), ids(&right)]);
    let merged = booknook_core::reconcile::reconcile(None, &left, &right).unwrap();
    assert!(merged.conflicts.is_empty());
    let path = source_root.0.join("unresolved-order-archive.json");
    std::fs::write(
        &path,
        canonical(&json!({
            "schema":"booknook.portable-export.v1", "state":merged.state,"assets":{}
        }))
        .unwrap(),
    )
    .unwrap();
    target
        .restore_archive(path.to_str().unwrap().into())
        .unwrap();
    drop(target);
    let target = Desk::new(target_root.path()).unwrap();

    // Keep one unrelated, ordinary conflict to prove both payloads stay separate.
    let normal = json!({"id":"ordinary","collection":"books","record_id":"example",
        "field":"review","local":"Local draft","incoming":"Incoming draft"});
    {
        let store = target.lock().unwrap();
        store
            .connection
            .execute(
                "INSERT INTO sync_conflicts VALUES(?1,?2)",
                ["ordinary", &normal.to_string()],
            )
            .unwrap();
    }
    let changes = |desk: &Desk| {
        desk.lock()
            .unwrap()
            .connection
            .query_row("SELECT total_changes()", [], |row| row.get::<_, i64>(0))
            .unwrap()
    };
    let before = target.snapshot().unwrap();
    let changed_rows = changes(&target);
    let status = value(target.sync_status().unwrap());
    assert_eq!(changes(&target), changed_rows);
    assert_eq!(target.snapshot().unwrap(), before);
    assert_eq!(status["conflicts"], json!([normal]));
    assert_eq!(status["peers"], json!([]));
    assert_eq!(status["outbox"], json!([]));
    let reviews = status["note_order_reviews"].as_array().unwrap();
    assert_eq!(reviews.len(), 1);
    let review = &reviews[0];
    assert_eq!(review["notebook_id"], notebook_id);
    assert_eq!(review["evidence"].as_str().unwrap().len(), 64);
    assert_eq!(review["heads"], json!([]));
    assert_eq!(review.as_object().unwrap().len(), 4);
    let snapshot: State = serde_json::from_str(&before).unwrap();
    assert_eq!(status["note_order_reviews_revision"], snapshot.revision);
    let visible = ids(&snapshot);
    let choices = review["choices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|choice| {
            choice
                .as_array()
                .unwrap()
                .iter()
                .map(|index| visible[usize::try_from(index.as_u64().unwrap()).unwrap()].clone())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(choices.len(), 2);
    assert_eq!(
        choices
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        expected_paths
    );
    assert_eq!(value(target.sync_status().unwrap()), status);
    assert_eq!(changes(&target), changed_rows);

    target
        .execute(
            json!({"op":"review_notebook_order","notebook_id":notebook_id,
                "review_id":"settled-archive-review","evidence":review["evidence"],
                "heads":review["heads"],"selected":choices[0]
            })
            .to_string(),
        )
        .unwrap();
    drop(target);
    let reopened = Desk::new(target_root.path()).unwrap();
    let completed = value(reopened.sync_status().unwrap());
    assert_eq!(completed["note_order_reviews"], json!([]));
    assert_eq!(
        completed["note_order_reviews_revision"],
        snapshot.revision + 1
    );
    for field in ["device_id", "peers", "conflicts", "outbox"] {
        assert_eq!(
            completed[field], status[field],
            "Changed existing {field} status"
        );
    }

    // Preview metadata is from the exact hashed plaintext, outside the approval batch.
    // Constants and wrapped batch fields remain unchanged for legacy clients.
    drop((reopened, source));
    let legacy = setup(&source_root);
    let v4 = setup(&target_root);
    let legacy_peer = id(&legacy);
    let v4_peer = id(&v4);
    let key = "4f".repeat(32);
    pair(&legacy, &v4_peer, &key, true);
    pair(&v4, &legacy_peer, &key, true);
    let check_prepared = |desk: &Desk, peer: &str, schema: &str| {
        let preview = value(desk.sync_prepare(peer.into()).unwrap());
        assert_eq!(preview["archive_schema"], schema);
        let id = preview["batch"]["id"].as_str().unwrap();
        let plaintext: String = desk
            .lock()
            .unwrap()
            .connection
            .query_row(
                "SELECT content FROM sync_prepared WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(value(plaintext.clone())["state"]["schema"], schema);
        assert_eq!(preview["bytes"], plaintext.len());
        assert_eq!(
            preview["batch"]["content_digest"],
            digest(plaintext.as_bytes())
        );
        assert!(preview["batch"].get("archive_schema").is_none());
        desk.sync_cancel_prepared(id.into()).unwrap();
    };
    check_prepared(&legacy, &v4_peer, "booknook.archive.v1");
    check_prepared(&v4, &legacy_peer, "booknook.archive.v4");
}

#[test]
fn encrypted_order_overflow_is_atomic_and_the_original_packet_retries_after_covering_history() {
    use booknook_core::{
        Command,
        model::{ORDER_ARCHIVE_SCHEMA, ORDER_ONTOLOGY},
        notebook_order::NotebookOrder,
        sync::ApprovedPacket,
    };
    use rusqlite::OptionalExtension;
    use std::{collections::BTreeMap, ops::Range};

    let ar = Directory::new();
    let br = Directory::new();
    let cr = Directory::new();
    let a = setup(&ar);
    let b = setup(&br);
    let c = setup(&cr);
    let ai = id(&a);
    let bi = id(&b);
    let ci = id(&c);
    let ab_key = "7c".repeat(32);
    let ac_key = "7d".repeat(32);
    pair(&a, &bi, &ab_key, true);
    pair(&b, &ai, &ab_key, true);
    pair(&a, &ci, &ac_key, true);
    pair(&c, &ai, &ac_key, true);

    let mut seed = State::default();
    booknook_core::apply(
        &mut seed,
        Command::AddBook {
            title: "Three synthetic ordered histories".into(),
            author: "Fixture".into(),
            at: 1,
        },
    )
    .unwrap();
    let notebook_id = seed.notebooks.keys().next().unwrap().clone();
    let template = &seed.notebooks[&notebook_id].blocks[0];
    let base_id = &template.id;
    let note_id = |index| format!("overflow-note-{index:02}");
    let ordered_ids: Vec<_> = std::iter::once(base_id.clone())
        .chain((0..34).map(note_id))
        .collect();
    let blocks: BTreeMap<_, _> = ordered_ids
        .iter()
        .map(|id| {
            let mut block = template.clone();
            block.id = id.clone();
            block.text = format!("Text for {id}");
            block.update = booknook_core::notebook::edit("", &block.text).unwrap();
            (id.clone(), block)
        })
        .collect();

    // Each side carries 17 individually admissible legacy paths. Only their
    // pairwise union exceeds the 32-path budget. C covers all 34 paths.
    let fork = |indices: Range<usize>| {
        indices.fold(NotebookOrder::default(), |history, index| {
            let branch = NotebookOrder::from_legacy(vec![base_id.clone(), note_id(index)]).unwrap();
            history.join(&branch).unwrap()
        })
    };
    let prepare_state = |order: NotebookOrder| {
        let mut state = seed.clone();
        state.schema = ORDER_ARCHIVE_SCHEMA.into();
        state.ontology_version = ORDER_ONTOLOGY.into();
        let notebook = state.notebooks.get_mut(&notebook_id).unwrap();
        notebook.blocks = order
            .view()
            .unwrap()
            .ids
            .iter()
            .map(|id| blocks[id].clone())
            .collect();
        notebook.order = Some(order);
        booknook_core::validate(&state).unwrap();
        state
    };
    let local = prepare_state(fork(0..17));
    let incoming = prepare_state(fork(17..34));
    let covering = prepare_state(NotebookOrder::from_legacy(ordered_ids).unwrap());
    for (desk, root, state) in [
        (&a, &ar, &local),
        (&b, &br, &incoming),
        (&c, &cr, &covering),
    ] {
        let path = root.0.join("seed-order.json");
        std::fs::write(
            &path,
            canonical(&json!({"schema":"booknook.portable-export.v1","state":state,"assets":{}}))
                .unwrap(),
        )
        .unwrap();
        desk.restore_archive(path.to_str().unwrap().into()).unwrap();
    }

    let packet_path = exported(&b, &ai, &ab_key, &br.0.join("original-b.json"));
    let packet_bytes = std::fs::read(&packet_path).unwrap();
    let packet: ApprovedPacket = booknook_core::strict_parse(&packet_bytes).unwrap();
    let batch_id = &packet.envelope.batch.id;
    let packet_digest = digest(&canonical(&packet).unwrap());
    let records = |desk: &Desk, peer: &str, batch: &str| {
        let store = desk.lock().unwrap();
        let received: Option<String> = store
            .connection
            .query_row(
                "SELECT digest FROM sync_received WHERE id=?1",
                [batch],
                |row| row.get(0),
            )
            .optional()
            .unwrap();
        let baseline: Option<String> = store
            .connection
            .query_row("SELECT json FROM sync_bases WHERE peer=?1", [peer], |row| {
                row.get(0)
            })
            .optional()
            .unwrap();
        (received, baseline.map(value))
    };

    let before = a.snapshot().unwrap();
    let before_status = a.sync_status().unwrap();
    let consumed = receipt_count(&a);
    assert_eq!(records(&a, &bi, batch_id), (None, None));
    let error = a
        .sync_receive(packet_path.clone(), bi.clone(), ab_key.clone())
        .expect_err("Both authenticated inputs fit, but their first union must exceed the budget");
    assert!(error.to_string().contains("LegacyBudget"), "{error}");
    assert_eq!(a.snapshot().unwrap(), before);
    assert_eq!(a.sync_status().unwrap(), before_status);
    assert_eq!(receipt_count(&a), consumed);
    assert_eq!(records(&a, &bi, batch_id), (None, None));

    drop(a);
    let a = Desk::new(ar.path()).unwrap();
    assert_eq!(a.snapshot().unwrap(), before);
    assert_eq!(records(&a, &bi, batch_id), (None, None));
    let cover_path = exported(&c, &ai, &ac_key, &cr.0.join("covering-c.json"));
    let covering_packet: ApprovedPacket =
        booknook_core::strict_parse(&std::fs::read(&cover_path).unwrap()).unwrap();
    let cover_batch_id = &covering_packet.envelope.batch.id;
    let cover_digest = digest(&canonical(&covering_packet).unwrap());
    assert_ne!(batch_id, cover_batch_id);

    let learned = value(a.sync_receive(cover_path, ci.clone(), ac_key).unwrap());
    assert_eq!(learned["duplicate"], false);
    assert_eq!(learned["conflicts"], 0);
    assert_eq!(learned["state"]["notebooks"], json!(covering.notebooks));
    let c_records = (Some(cover_digest), Some(value(c.snapshot().unwrap())));
    assert_eq!(records(&a, &ci, cover_batch_id), c_records);
    assert_eq!(records(&a, &bi, batch_id), (None, None));

    assert_eq!(std::fs::read(&packet_path).unwrap(), packet_bytes);
    let retried = value(
        a.sync_receive(packet_path.clone(), bi.clone(), ab_key.clone())
            .unwrap(),
    );
    assert_eq!(retried["duplicate"], false);
    assert_eq!(retried["conflicts"], 0);
    assert_eq!(retried["state"]["notebooks"], json!(covering.notebooks));
    let b_records = (Some(packet_digest), Some(value(b.snapshot().unwrap())));
    assert_eq!(records(&a, &bi, batch_id), b_records);
    assert_eq!(records(&a, &ci, cover_batch_id), c_records);

    let durable = a.snapshot().unwrap();
    drop(a);
    let a = Desk::new(ar.path()).unwrap();
    assert_eq!(a.snapshot().unwrap(), durable);
    assert_eq!(records(&a, &bi, batch_id), b_records);
    assert_eq!(records(&a, &ci, cover_batch_id), c_records);
    assert_eq!(receipt_count(&a), consumed);
    assert_eq!(std::fs::read(&packet_path).unwrap(), packet_bytes);
    let duplicate = value(a.sync_receive(packet_path, bi, ab_key).unwrap());
    assert_eq!(duplicate["duplicate"], true);
    assert_eq!(a.snapshot().unwrap(), durable);
}
