use booknook_core::feeds::{FeedConfiguration, FeedSection, ImportReview};
use booknook_native::{
    FeedExportReviewV1, FeedImportReviewV1, FeedProjectionReviewV1, empty_feed_configuration_v1,
    validate_feed_configuration_v1, validate_feed_section_v1,
};
use serde_json::{Value, json};
fn bytes(value: &impl serde::Serialize) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}
fn section() -> Value {
    serde_json::from_slice(include_bytes!("fixtures/feeds-v1/section.json")).unwrap()
}
fn config(feeds: Value) -> Value {
    json!({"schema":"booknook.feed-configuration.v1","revision":0,"feeds":feeds})
}
fn object(raw: String) -> Value {
    serde_json::from_str(&raw).unwrap()
}
#[test]
fn native_literal_string_configuration_projection_and_resolution_boundaries() {
    for (path, literal) in [
        ("/schema", "castalia.feeds-section.v1"),
        ("/subscriptions/0/format", "atom"),
        ("/subscriptions/0/disclosure", "selected"),
        ("/preferences/order", "newest-first"),
    ] {
        for bad in [
            Value::Null,
            json!(false),
            json!(1),
            json!([]),
            json!([literal]),
            json!({literal:null}),
        ] {
            let mut value = section();
            *value.pointer_mut(path).unwrap() = bad;
            assert!(validate_feed_section_v1(bytes(&value)).is_err(), "{value}");
        }
    }
    let mut value = config(section());
    value["schema"] = json!({"booknook.feed-configuration.v1":null});
    assert!(validate_feed_configuration_v1(bytes(&value)).is_err());
    let id = section()["subscriptions"][0]["id"].clone();
    let bad = json!({"schema":{"booknook.feed-disclosure-selection.v1":null},"origin":"https://reader.example.org","subscriptionIds":[id.clone()],"collectionIds":[],"memberships":[{"subscriptionId":id,"collectionIds":[]}]});
    assert!(FeedProjectionReviewV1::new(bytes(&config(section())), bytes(&bad)).is_err());
}
#[test]
fn native_adapter_matches_all_shared_raw_vectors_without_loss() {
    let corpus: Value =
        serde_json::from_slice(include_bytes!("fixtures/feeds-v1/conformance.json")).unwrap();
    assert_eq!(corpus["cases"].as_array().unwrap().len(), 102);
    for case in corpus["cases"].as_array().unwrap() {
        let raw = case["raw"].as_str().unwrap().as_bytes().to_vec();
        let result = validate_feed_section_v1(raw.clone());
        assert_eq!(
            result.is_ok(),
            case["valid"].as_bool().unwrap(),
            "{}",
            case["id"]
        );
        if let Ok(result) = result {
            assert_eq!(
                object(result),
                serde_json::from_slice::<Value>(&raw).unwrap()
            );
        }
    }
    assert!(validate_feed_section_v1(vec![0xc3, 0x28]).is_err());
    assert_eq!(
        object(empty_feed_configuration_v1().unwrap()),
        serde_json::to_value(FeedConfiguration::empty()).unwrap()
    );
    let current = config(section());
    assert_eq!(
        object(validate_feed_configuration_v1(bytes(&current)).unwrap()),
        current
    );
}
#[test]
fn native_review_objects_bind_exact_raw_and_configuration_and_resolve_through_core() {
    let current = config(section());
    let mut incoming = section();
    incoming["subscriptions"][0]["title"] = json!("Reviewed change");
    let current_raw = bytes(&current);
    let incoming_raw = bytes(&incoming);
    let review = FeedImportReviewV1::new(current_raw.clone(), Some(incoming_raw.clone())).unwrap();
    let preview = object(review.preview().unwrap());
    assert_eq!(preview["schema"], "booknook.feed-import-review.v1");
    assert_eq!(preview["conflicts"].as_array().unwrap().len(), 1);
    assert!(preview["candidate"].is_null());
    assert!(
        review
            .confirm(current_raw.clone(), Some(incoming_raw.clone()))
            .is_err()
    );
    let choices = json!([{"target":"subscription","id":incoming["subscriptions"][0]["id"],"choice":"incoming"}]);
    for (field, literal) in [("target", "subscription"), ("choice", "incoming")] {
        let mut bad = choices.clone();
        bad[0][field] = json!({literal:null});
        assert!(
            review
                .resolve(current_raw.clone(), Some(incoming_raw.clone()), bytes(&bad))
                .is_err()
        );
    }
    let resolved = review
        .resolve(
            current_raw.clone(),
            Some(incoming_raw.clone()),
            bytes(&choices),
        )
        .unwrap();
    let native = object(
        resolved
            .confirm(current_raw.clone(), Some(incoming_raw.clone()))
            .unwrap(),
    );
    let typed = FeedConfiguration::parse(&current_raw).unwrap();
    let core_review = ImportReview::prepare(&typed, Some(&incoming_raw)).unwrap();
    let core_choices: Vec<booknook_core::feeds::ConflictResolution> =
        booknook_core::strict_parse(&bytes(&choices)).unwrap();
    let core = core_review
        .resolve(&typed, Some(&incoming_raw), &core_choices)
        .unwrap()
        .confirm(&typed, Some(&incoming_raw))
        .unwrap();
    assert_eq!(native, serde_json::to_value(core).unwrap());
    let changed = [incoming_raw.as_slice(), b" "].concat();
    assert!(
        review
            .resolve(current_raw.clone(), Some(changed.clone()), bytes(&choices))
            .is_err()
    );
    assert!(
        resolved
            .confirm(current_raw.clone(), Some(changed))
            .is_err()
    );
    let mut edited = current.clone();
    edited["revision"] = json!(1);
    assert!(
        resolved
            .confirm(bytes(&edited), Some(incoming_raw.clone()))
            .is_err()
    );
    assert!(
        review
            .resolve(
                current_raw.clone(),
                Some(incoming_raw.clone()),
                b"[{\"target\":\"subscription\",\"target\":\"collection\"}]".to_vec()
            )
            .is_err()
    );
    assert!(FeedImportReviewV1::new(current_raw.clone(), Some(vec![])).is_err());
    let none = FeedImportReviewV1::new(current_raw.clone(), None).unwrap();
    assert_eq!(object(none.confirm(current_raw, None).unwrap()), current);
}
#[test]
fn native_projection_preserves_exact_origin_and_removes_unreferenced_private_labels() {
    let mut feeds = section();
    let source_id = feeds["subscriptions"][0]["id"].clone();
    let orphan = "b41ef5e8-0e06-43e0-aa47-000000000999";
    feeds["collections"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":orphan,"title":"Private orphan"}));
    let current = config(feeds);
    let current_raw = bytes(&current);
    let selection = json!({"schema":"booknook.feed-disclosure-selection.v1","origin":"https://reader.example.org","subscriptionIds":[source_id.clone()],"collectionIds":[orphan],"memberships":[{"subscriptionId":source_id,"collectionIds":[]}]});
    let review = FeedProjectionReviewV1::new(current_raw.clone(), bytes(&selection)).unwrap();
    let projection = object(
        review
            .confirm(current_raw.clone(), "https://reader.example.org".into())
            .unwrap(),
    );
    assert_eq!(projection["schema"], "booknook.selected-feeds.v1");
    assert_eq!(projection["feeds"]["collections"], json!([]));
    assert_eq!(
        projection["feeds"]["subscriptions"][0]["collectionIds"],
        json!([])
    );
    assert!(!projection.to_string().contains(orphan));
    assert!(!projection.to_string().contains("Private orphan"));
    assert_eq!(object(review.preview().unwrap()), projection);
    assert!(
        review
            .confirm(current_raw.clone(), "https://reader.example.org/".into())
            .is_err()
    );
    let mut changed = current.clone();
    changed["revision"] = json!(1);
    assert!(
        review
            .confirm(bytes(&changed), "https://reader.example.org".into())
            .is_err()
    );
    assert!(
        FeedProjectionReviewV1::new(
            current_raw.clone(),
            b"{\"origin\":\"https://reader.example.org\",\"origin\":\"https://other.example.org\"}"
                .to_vec()
        )
        .is_err()
    );
    assert_eq!(
        FeedSection::parse(&bytes(&current["feeds"]))
            .unwrap()
            .collections
            .len(),
        2
    );
}

#[test]
fn native_portable_export_is_origin_free_closed_and_bound_to_canonical_configuration() {
    let current = config(section());
    let current_raw = bytes(&current);
    let selection = json!({"schema":"booknook.feed-export-selection.v1",
        "subscriptionIds":[current["feeds"]["subscriptions"][0]["id"]],
        "collectionIds":[current["feeds"]["collections"][0]["id"]],
        "memberships":[{"subscriptionId":current["feeds"]["subscriptions"][0]["id"],
        "collectionIds":[current["feeds"]["collections"][0]["id"]]}]});
    let review = FeedExportReviewV1::new(current_raw.clone(), bytes(&selection)).unwrap();
    let output = review
        .confirm([current_raw.as_slice(), b" \n"].concat())
        .unwrap();
    assert_eq!(object(output.clone()), current["feeds"]);
    assert_eq!(object(review.preview().unwrap()), current["feeds"]);
    assert!(FeedSection::parse(output.as_bytes()).is_ok());
    let empty = bytes(&FeedConfiguration::empty());
    let imported = FeedImportReviewV1::new(empty.clone(), Some(output.into_bytes())).unwrap();
    assert_eq!(
        object(
            imported
                .confirm(empty, Some(review.preview().unwrap().into_bytes()))
                .unwrap()
        )["feeds"],
        current["feeds"]
    );
    let mut changed = current.clone();
    changed["revision"] = json!(1);
    assert!(review.confirm(bytes(&changed)).is_err());
    changed = current.clone();
    changed["feeds"]["subscriptions"][0]["title"] = json!("edited");
    assert!(review.confirm(bytes(&changed)).is_err());
    changed = current.clone();
    changed["feeds"]["subscriptions"][0]["disclosure"] = json!("private");
    assert!(FeedExportReviewV1::new(bytes(&changed), bytes(&selection)).is_err());
    let text = String::from_utf8(bytes(&selection)).unwrap();
    for invalid in [
        text.replacen('{', "{\"origin\":\"https://reader.example.org\",", 1),
        text.replacen('{', "{\"subscription\\u0049ds\":[],", 1),
        text.replace(
            "\"schema\":\"booknook.feed-export-selection.v1\"",
            "\"schema\":{\"booknook.feed-export-selection.v1\":null}",
        ),
    ] {
        assert!(FeedExportReviewV1::new(current_raw.clone(), invalid.into_bytes()).is_err());
    }
    assert!(FeedExportReviewV1::new(current_raw, vec![0xc3, 0x28]).is_err());
}
