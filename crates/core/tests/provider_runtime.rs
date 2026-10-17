// SPDX-License-Identifier: MIT OR Apache-2.0
use booknook_core::{canonical, model::State, provider_runtime::*};
use serde_json::{Value, json};
fn registry(namespace: &str, capabilities: Value) -> ProviderRegistry {
    parse_registry(&serde_json::to_vec(&json!({"schema":"booknook.provider-registry.v1","providers":[{"id":"test.source","source_namespace":namespace,"capabilities":capabilities,"available":true,"reason":null,"connection":null}]})).unwrap()).unwrap()
}
fn request(registry: &ProviderRegistry, op: Value) -> ProviderRequest {
    prepare_operation(
        registry,
        1,
        "test.source",
        &serde_json::to_vec(&op).unwrap(),
        "test-request",
    )
    .unwrap()
}
fn reply(request: &ProviderRequest, result: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"schema":"booknook.provider-response.v1","request_id":request.request_id,"generation":request.generation,"provider_id":request.provider_id,"connection_id":request.connection_id,"result":result})).unwrap()
}
fn catalog(title: &str) -> Value {
    json!({"kind":"catalog","catalog":{"format":"normalized","entries":[{"source_id":"source-1","title":title,"authors":["Écrivain"],"subjects":["nature"],"edition_identifiers":[{"provider":"isbn","id":"123"}],"language":"fr"}]}})
}
#[test]
fn strict_registry_selection_and_raw_correlation_reject_ambiguous_or_stale_providers() {
    let registry = registry("example.library", json!(["catalog"]));
    let req = request(
        &registry,
        json!({"kind":"catalog","source_id":"opaque-selected-file"}),
    );
    let raw = reply(&req, catalog("Première"));
    assert!(validate_reply(&registry, 1, &req, &raw, &[], 100).is_ok());
    assert!(validate_reply(&registry, 2, &req, &raw, &[], 100).is_err());
    let mut changed = registry.clone();
    changed.providers[0].source_namespace = "different.library".into();
    assert!(validate_reply(&changed, 1, &req, &raw, &[], 100).is_err());
    changed = registry.clone();
    changed.providers[0].available = false;
    assert!(
        prepare_operation(
            &changed,
            1,
            "test.source",
            br#"{"kind":"catalog","source_id":"x"}"#,
            "x"
        )
        .is_err()
    );
    assert!(
        prepare_operation(
            &registry,
            1,
            "test.source",
            br#"{"kind":"content","source_id":"x","media_type":"application/pdf"}"#,
            "x"
        )
        .is_err()
    );
    for bad in [
        String::from_utf8(raw.clone()).unwrap().replacen(
            "\"generation\":1",
            "\"generation\":1,\"generation\":1",
            1,
        ),
        String::from_utf8(raw.clone())
            .unwrap()
            .replacen("\"connection_id\":null,", "", 1),
        String::from_utf8(raw).unwrap().replacen(
            "\"language\":\"fr\"",
            "\"language\":\"fr\",\"private_key\":\"forbidden\"",
            1,
        ),
    ] {
        assert!(validate_reply(&registry, 1, &req, bad.as_bytes(), &[], 100).is_err());
    }
    let mut raw = serde_json::to_value(&registry).unwrap();
    raw["providers"][0]
        .as_object_mut()
        .unwrap()
        .remove("connection");
    assert!(parse_registry(&serde_json::to_vec(&raw).unwrap()).is_err());
    raw = serde_json::to_value(&registry).unwrap();
    let duplicate = raw["providers"][0].clone();
    raw["providers"].as_array_mut().unwrap().push(duplicate);
    assert!(parse_registry(&serde_json::to_vec(&raw).unwrap()).is_err());
}
#[test]
fn normalized_catalog_is_atomic_idempotent_and_preserves_personal_work() {
    let registry = registry("example.library", json!(["catalog"]));
    let req = request(&registry, json!({"kind":"catalog","source_id":"selected"}));
    let execution = validate_reply(
        &registry,
        1,
        &req,
        &reply(&req, catalog("Original")),
        &[],
        100,
    )
    .unwrap();
    let mut state = State::default();
    assert_eq!(
        apply_catalog(
            &mut state,
            &registry,
            1,
            &canonical(&execution).unwrap(),
            100
        )
        .unwrap()
        .inserted,
        1
    );
    let id = state.books.keys().next().unwrap().clone();
    let book = state.books.get_mut(&id).unwrap();
    book.title = "My title".into();
    book.edited_fields.insert("title".into());
    book.review = "Private review".into();
    book.rating = Some(5);
    book.shelf = "reading".into();
    let notebook = booknook_core::ensure_notebook(&mut state, &id, 101).unwrap();
    let notebook_before = canonical(&state.notebooks[&notebook.id]).unwrap();
    let updated = validate_reply(
        &registry,
        1,
        &req,
        &reply(&req, catalog("Changed metadata")),
        &[],
        102,
    )
    .unwrap();
    assert_eq!(
        apply_catalog(&mut state, &registry, 1, &canonical(&updated).unwrap(), 102)
            .unwrap()
            .updated,
        1
    );
    assert_eq!(state.books[&id].title, "My title");
    assert_eq!(state.books[&id].review, "Private review");
    assert_eq!(state.books[&id].rating, Some(5));
    assert_eq!(state.books[&id].shelf, "reading");
    assert_eq!(
        canonical(&state.notebooks[&notebook.id]).unwrap(),
        notebook_before
    );
    assert_eq!(
        apply_catalog(&mut state, &registry, 1, &canonical(&updated).unwrap(), 103)
            .unwrap()
            .unchanged,
        1
    );
    assert_eq!(state.books.len(), 1);
    assert_eq!(state.books[&id].provenance.len(), 2);
    let before = state.clone();
    let mut bad = serde_json::to_value(updated).unwrap();
    bad["response"]["result"]["catalog"]["entries"][0]["title"] = json!("");
    assert!(
        apply_catalog(
            &mut state,
            &registry,
            1,
            &serde_json::to_vec(&bad).unwrap(),
            104
        )
        .is_err()
    );
    assert_eq!(state, before);
    let restored: State = booknook_core::strict_parse(&canonical(&state).unwrap()).unwrap();
    booknook_core::validate(&restored).unwrap();
    assert_eq!(restored, state);
}
#[test]
fn legacy_catalog_identity_is_unchanged_and_progress_is_a_separate_capability() {
    let registry = registry("kindle", json!(["catalog"]));
    let req = request(&registry, json!({"kind":"catalog","source_id":"selected"}));
    let legacy=json!({"schema":"booknook.kindle-catalog.v1","items":[{"asin":"B000000001","title":"Legacy book","authors":["Writer"],"position":null,"observed_at":null}]}).to_string();
    let execution = validate_reply(
        &registry,
        1,
        &req,
        &reply(
            &req,
            json!({"kind":"catalog","catalog":{"format":"kindle","json":legacy}}),
        ),
        &[],
        100,
    )
    .unwrap();
    let mut composed = State::default();
    apply_catalog(
        &mut composed,
        &registry,
        1,
        &canonical(&execution).unwrap(),
        100,
    )
    .unwrap();
    let mut direct = State {
        local_reader: composed.local_reader.clone(),
        ..Default::default()
    };
    booknook_core::imports::kindle(&mut direct, &legacy, 100).unwrap();
    composed.revision = direct.revision;
    assert_eq!(composed, direct);
    let with_progress = legacy
        .replace("\"position\":null", "\"position\":\"chapter-2\"")
        .replace("\"observed_at\":null", "\"observed_at\":99");
    assert!(
        validate_reply(
            &registry,
            1,
            &req,
            &reply(
                &req,
                json!({"kind":"catalog","catalog":{"format":"kindle","json":with_progress}})
            ),
            &[],
            100
        )
        .is_err()
    );
    let mut capable = registry.clone();
    capable.providers[0]
        .capabilities
        .push(RuntimeCapability::Positions);
    let req = request(&capable, json!({"kind":"catalog","source_id":"selected"}));
    let execution = validate_reply(
        &capable,
        1,
        &req,
        &reply(
            &req,
            json!({"kind":"catalog","catalog":{"format":"kindle","json":with_progress}}),
        ),
        &[],
        100,
    )
    .unwrap();
    apply_catalog(
        &mut direct,
        &capable,
        1,
        &canonical(&execution).unwrap(),
        100,
    )
    .unwrap();
    assert_eq!(direct.positions.len(), 1);
}
#[test]
fn binary_content_and_identity_results_cannot_change_the_selected_source() {
    let registry = registry("example.library", json!(["content", "identity"]));
    let req = request(
        &registry,
        json!({"kind":"content","source_id":"selected","media_type":"application/pdf"}),
    );
    let bytes = b"%PDF-synthetic";
    let result = json!({"kind":"content","source_id":"selected","media_type":"application/pdf","sha256":booknook_core::digest(bytes)});
    assert!(validate_reply(&registry, 1, &req, &reply(&req, result.clone()), bytes, 100).is_ok());
    assert!(validate_reply(&registry, 1, &req, &reply(&req, result), b"changed", 100).is_err());
    let req = request(
        &registry,
        json!({"kind":"identity","entity_iri":"did:example:reader"}),
    );
    let result = json!({"kind":"identity","reference":{"entity_iri":"did:example:other","kind":"Person","display_name":null}});
    assert!(validate_reply(&registry, 1, &req, &reply(&req, result), &[], 100).is_err());
}
#[test]
fn connection_provenance_requires_matching_source_and_explicit_capability() {
    let mut registry = registry("kindle", json!(["catalog"]));
    registry.providers[0].connection=Some(booknook_core::strict_parse(br#"{"schema":"castalia.provider-connection.v1","connection_id":"audible-a","provider_id":"com.audible","display_name":"Account","capabilities":["audible.positions"],"status":"available","credential":null}"#).unwrap());
    assert!(
        prepare_operation(
            &registry,
            1,
            "test.source",
            br#"{"kind":"catalog","source_id":"x"}"#,
            "x"
        )
        .is_err()
    );
    let c = registry.providers[0].connection.as_mut().unwrap();
    c.provider_id = "com.amazon.kindle".into();
    c.capabilities = vec!["kindle.catalog".into()];
    let req = request(&registry, json!({"kind":"catalog","source_id":"x"}));
    assert_eq!(req.connection_id.as_deref(), Some("audible-a"));
}
#[test]
fn recommendations_cannot_exfiltrate_private_fields_or_invent_candidates() {
    let registry = registry("example.library", json!(["recommendations"]));
    let input = json!({"candidates":[{"book_id":"b","title":"Book","subjects":[]}],"preferred_subjects":[],"shared_ratings":[],"friends":[],"excluded":[]});
    let req = request(&registry, json!({"kind":"recommendations","input":input}));
    assert!(validate_reply(&registry,1,&req,&reply(&req,json!({"kind":"recommendations","recommendations":[{"book_id":"invented","score":1,"reasons":["reason"]}]})),&[],100).is_err());
    let mut input = input;
    input["notebook"] = json!("private");
    assert!(
        prepare_operation(
            &registry,
            1,
            "test.source",
            &serde_json::to_vec(&json!({"kind":"recommendations","input":input})).unwrap(),
            "x"
        )
        .is_err()
    );
}

#[test]
fn alternate_source_positions_remain_opaque_and_bind_to_explicit_reading_pass() {
    let registry = registry("example.library", json!(["catalog", "positions"]));
    let catalog_req = request(&registry, json!({"kind":"catalog","source_id":"selected"}));
    let catalog = validate_reply(
        &registry,
        1,
        &catalog_req,
        &reply(&catalog_req, catalog("Book")),
        &[],
        100,
    )
    .unwrap();
    let mut state = State::default();
    apply_catalog(&mut state, &registry, 1, &canonical(&catalog).unwrap(), 100).unwrap();
    let id = state.books.keys().next().unwrap().clone();
    let pass = booknook_core::ensure_notebook(&mut state, &id, 100)
        .unwrap()
        .pass_id;
    let req = request(
        &registry,
        json!({"kind":"source_positions","source_ids":["source-1"]}),
    );
    let observation =
        json!({"source_id":"source-1","value":"chapter:iv/location:42","observed_at":95});
    let execution = validate_reply(
        &registry,
        1,
        &req,
        &reply(
            &req,
            json!({"kind":"source_positions","observations":[observation]}),
        ),
        &[],
        100,
    )
    .unwrap();
    apply_source_position(
        &mut state,
        &registry,
        1,
        &canonical(&execution).unwrap(),
        &serde_json::to_vec(&observation).unwrap(),
        &pass,
        100,
    )
    .unwrap();
    let position = state.positions.values().next().unwrap();
    assert_eq!(position.pass_id, pass);
    assert_eq!(
        position.alignment,
        booknook_core::model::Alignment::Unmatched
    );
    assert!(
        matches!(&position.location,booknook_core::model::Location::Provider{provider,value,..} if provider=="example.library"&&value=="chapter:iv/location:42")
    );
    let before = state.clone();
    let other = json!({"source_id":"source-1","value":"unfetched","observed_at":95});
    assert!(
        apply_source_position(
            &mut state,
            &registry,
            1,
            &canonical(&execution).unwrap(),
            &serde_json::to_vec(&other).unwrap(),
            &pass,
            100
        )
        .is_err()
    );
    assert_eq!(state, before);
}
