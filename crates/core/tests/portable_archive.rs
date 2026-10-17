// SPDX-License-Identifier: MIT OR Apache-2.0
use base64::{Engine, engine::general_purpose::STANDARD};
use booknook_core::{Command, model::*, portable_archive as archive};
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Timestamp};
use serde_json::{Value, json};
use std::collections::BTreeMap;
fn minimal() -> archive::PortableArchive {
    archive::PortableArchive {
        schema: archive::SCHEMA.into(),
        state: State::default(),
        assets: BTreeMap::new(),
        social: None,
    }
}
fn raw(value: &impl serde::Serialize) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}
#[test]
fn legacy_optional_social_and_original_strings_roundtrip_without_hidden_fields() {
    let mut value = minimal();
    let legacy = raw(&value);
    assert_eq!(archive::roundtrip(&legacy).unwrap().as_bytes(), legacy);
    value.assets.insert(
        "legacy-unreferenced".into(),
        "uninterpreted legacy data".into(),
    );
    value.social = Some(
        json!({"schema":archive::SOCIAL_SCHEMA,"state":booknook_core::social_protocol::SocialState::default(),"bindings":[],"pending":[]}),
    );
    let decoded: Value = serde_json::from_str(&archive::roundtrip(&raw(&value)).unwrap()).unwrap();
    assert!(decoded["social"].get("binding_history").is_none());
    assert_eq!(
        decoded["assets"]["legacy-unreferenced"],
        "uninterpreted legacy data"
    );
    let mut bad = decoded.clone();
    bad["settings"] = json!({});
    assert!(archive::parse(&raw(&bad)).is_err());
    bad = decoded.clone();
    bad["social"]["credentials"] = json!({});
    assert!(archive::parse(&raw(&bad)).is_err());
    let duplicate = String::from_utf8(raw(&decoded)).unwrap().replacen(
        "\"assets\":",
        "\"assets\":{},\"assets\":",
        1,
    );
    assert!(archive::parse(duplicate.as_bytes()).is_err());
    value.state.revision = u64::MAX;
    assert!(
        archive::roundtrip(&raw(&value))
            .unwrap()
            .contains("18446744073709551615")
    );
}
#[test]
fn assets_are_complete_and_checksum_verified_without_rewriting_original_encoding() {
    let mut value = minimal();
    booknook_core::apply(
        &mut value.state,
        Command::AddBook {
            title: "Fixture".into(),
            author: "Writer".into(),
            at: 1,
        },
    )
    .unwrap();
    let book = value.state.books.values_mut().next().unwrap();
    let data = b"%PDF-1.4\nOriginal content\n";
    let hash = booknook_core::digest(data);
    let asset = ContentAsset {
        id: "asset".into(),
        media_type: "application/pdf".into(),
        storage_key: hash.clone(),
        sha256: hash,
        spine: vec![],
    };
    let edition = Edition {
        id: "edition".into(),
        book_id: book.id.clone(),
        identifiers: vec![],
        language: None,
        assets: vec![asset],
    };
    book.edition_ids.push(edition.id.clone());
    value.state.editions.insert(edition.id.clone(), edition);
    assert!(archive::validate(&value).is_err());
    value.assets.insert("asset".into(), STANDARD.encode(data));
    assert_eq!(archive::parse(&raw(&value)).unwrap().assets, value.assets);
    value
        .assets
        .insert("asset".into(), STANDARD.encode(b"Changed content"));
    assert!(
        archive::validate(&value)
            .unwrap_err()
            .to_string()
            .contains("checksum")
    );
    value.assets.insert("asset".into(), "not base64".into());
    assert!(
        archive::validate(&value)
            .unwrap_err()
            .to_string()
            .contains("base64")
    );
}
fn pending_archive() -> Value {
    let key = Keys::parse(&"03".repeat(32)).unwrap();
    let event = EventBuilder::new(Kind::Custom(30078), "Pending synthetic content")
        .custom_created_at(Timestamp::from(100))
        .sign_with_keys(&key)
        .unwrap();
    let exact = format!("\n{}\n", event.as_json());
    let mut value = minimal();
    value.social = Some(json!({"schema":archive::SOCIAL_SCHEMA,
        "state":booknook_core::social_protocol::SocialState::default(),"bindings":[],
        "binding_history":[],"pending":[{"id":event.id.to_hex(),"event_json":exact,"reason":"Waiting for parent"}]}));
    serde_json::to_value(value).unwrap()
}
#[test]
fn pending_events_keep_exact_signed_bytes_and_require_real_signature_integrity() {
    let value = pending_archive();
    let normalized: Value =
        serde_json::from_str(&archive::roundtrip(&raw(&value)).unwrap()).unwrap();
    assert_eq!(normalized, value);
    let mut bad = value.clone();
    let mut event: Value =
        serde_json::from_str(bad["social"]["pending"][0]["event_json"].as_str().unwrap()).unwrap();
    event["sig"] = "00".repeat(64).into();
    bad["social"]["pending"][0]["event_json"] = event.to_string().into();
    assert!(
        archive::parse(&raw(&bad))
            .unwrap_err()
            .to_string()
            .contains("Forged")
    );
    bad = value.clone();
    bad["social"]["pending"][0]["id"] = "00".repeat(32).into();
    assert!(
        archive::parse(&raw(&bad))
            .unwrap_err()
            .to_string()
            .contains("ID mismatch")
    );
    bad = value.clone();
    let duplicate = bad["social"]["pending"][0].clone();
    bad["social"]["pending"]
        .as_array_mut()
        .unwrap()
        .push(duplicate);
    assert!(archive::parse(&raw(&bad)).is_err());
    bad = value;
    bad["social"]["pending"][0]["reason"] = "x".repeat(4001).into();
    assert!(archive::parse(&raw(&bad)).is_err());
}
#[test]
fn whole_envelope_keeps_raw_limit_and_rejects_unsupported_versions() {
    let mut value = minimal();
    value.schema = "booknook.portable-export.v2".into();
    assert!(archive::parse(&raw(&value)).is_err());
    assert!(
        archive::parse(&vec![b' '; booknook_core::MAX_JSON_BYTES + 1])
            .unwrap_err()
            .to_string()
            .contains("128 MiB")
    );
}

#[test]
fn published_native_envelope_fixture_preserves_every_portable_record() {
    let raw = include_str!("fixtures/portable-archive.json");
    let before: Value = serde_json::from_str(raw).unwrap();
    let after: Value = serde_json::from_str(&archive::roundtrip(raw.as_bytes()).unwrap()).unwrap();
    assert_eq!(before, after);
    assert_eq!(after["assets"].as_object().unwrap().len(), 2);
    assert_eq!(after["social"]["pending"].as_array().unwrap().len(), 1);
    assert_eq!(after["state"]["schema"], "booknook.archive.v2");
}
