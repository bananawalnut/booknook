// SPDX-License-Identifier: MIT OR Apache-2.0
use booknook_core::{authority::verify_receipt, canonical, digest, strict_parse, sync};
#[path = "fixtures/receipts.rs"]
mod receipts;

#[test]
fn portable_packet_accepts_legacy_wire_and_rechecks_approval_before_plaintext() {
    let (_, _, initial_policy) = receipts::receipt(b"x", "sync.send", 7);
    let bytes = b"private Unicode \xf0\x9f\x8c\xbf and binary \x00\xff";
    let batch = sync::prepare(
        &initial_policy.actor,
        7,
        vec!["device-\u{65e5}".into()],
        bytes,
    )
    .unwrap();
    let (request, presentation, policy) =
        receipts::receipt(&canonical(&batch).unwrap(), "sync.send", 7);
    let receipt = verify_receipt(
        &request,
        &presentation,
        &policy,
        &canonical(&batch).unwrap(),
        7,
        110,
    )
    .unwrap();
    // Construct exactly the pre-extraction native packet using the existing envelope API.
    let legacy = serde_json::json!({"schema":"booknook.sync-packet.v1",
        "envelope":sync::seal(batch.clone(), bytes, &[8;32], &receipt, 110).unwrap(),
        "approved_at":110,"request_json":String::from_utf8(request.clone()).unwrap(),
        "presentation_json":String::from_utf8(presentation.clone()).unwrap()});
    let packet: sync::ApprovedPacket = strict_parse(&canonical(&legacy).unwrap()).unwrap();
    assert_eq!(canonical(&packet).unwrap(), canonical(&legacy).unwrap());
    let opened = sync::open_packet(&packet, &[8; 32], "device-\u{65e5}", &policy, 10000).unwrap();
    assert_eq!(opened.plaintext, bytes);
    assert_eq!(opened.packet_digest, digest(&canonical(&legacy).unwrap()));
    assert_eq!(opened.replay_key, receipt.replay_key());
    let mut revoked = policy.clone();
    revoked.pins.clear();
    assert!(sync::open_packet(&packet, &[8; 32], "device-\u{65e5}", &revoked, 10000).is_err());
    assert!(sync::open_packet(&packet, &[9; 32], "device-\u{65e5}", &policy, 10000).is_err());
    assert!(sync::open_packet(&packet, &[8; 32], "stranger", &policy, 10000).is_err());
    assert!(sync::open_packet(&packet, &[8; 32], "device-\u{65e5}", &policy, 109).is_err());
    let mut tampered = packet.clone();
    tampered.approved_at = 161;
    assert!(sync::open_packet(&tampered, &[8; 32], "device-\u{65e5}", &policy, 10000).is_err());
    let request = String::from_utf8(request).unwrap();
    let presentation = String::from_utf8(presentation).unwrap();
    assert!(
        sync::seal_packet(
            batch.clone(),
            bytes,
            &[8; 32],
            request.clone(),
            presentation.clone(),
            &policy,
            161
        )
        .is_err()
    );
    assert!(
        sync::seal_packet(
            batch,
            b"changed after consent",
            &[8; 32],
            request,
            presentation,
            &policy,
            110
        )
        .is_err()
    );
}

#[test]
fn packet_rejects_wrong_operation_even_when_both_signatures_are_valid() {
    let (_, _, initial_policy) = receipts::receipt(b"x", "social.publish", 7);
    let batch = sync::prepare(&initial_policy.actor, 7, vec!["device".into()], b"private").unwrap();
    let (request, presentation, wrong_policy) =
        receipts::receipt(&canonical(&batch).unwrap(), "social.publish", 7);
    assert!(
        sync::seal_packet(
            batch.clone(),
            b"private",
            &[8; 32],
            String::from_utf8(request.clone()).unwrap(),
            String::from_utf8(presentation.clone()).unwrap(),
            &wrong_policy,
            110
        )
        .is_err()
    );
    let packet = sync::ApprovedPacket {
        schema: "booknook.sync-packet.v1".into(),
        envelope: sync::EncryptedBatch {
            batch,
            nonce: String::new(),
            ciphertext: String::new(),
        },
        approved_at: 110,
        request_json: String::from_utf8(request).unwrap(),
        presentation_json: String::from_utf8(presentation).unwrap(),
    };
    assert!(sync::verify_packet(&packet, &wrong_policy, 110).is_err());
    assert!(
        sync::prepare(
            &initial_policy.actor,
            7,
            vec!["device".into(), "device".into()],
            b"private"
        )
        .is_err()
    );
    assert!(sync::prepare(&initial_policy.actor, 7, vec![], b"private").is_err());
}
