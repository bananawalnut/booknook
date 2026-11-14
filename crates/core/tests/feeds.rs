use booknook_core::feeds::{
    ConflictResolution, ConflictTarget, DisclosureSelection, ExportReview, ExportSelection,
    FeedConfiguration, FeedDisclosure, FeedSection, ImportReview, MembershipSelection,
    ProjectionReview, ResolutionChoice,
};
use serde_json::{Value, json};

fn section() -> FeedSection {
    FeedSection::parse(include_bytes!("fixtures/feeds-v1/section.json")).unwrap()
}
fn raw(value: &impl serde::Serialize) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}
fn id(n: usize) -> String {
    format!("b41ef5e8-0e06-43e0-aa47-{n:012x}")
}
fn source(mut value: FeedSection, n: usize) -> FeedSection {
    value.subscriptions[0].id = id(n);
    value.collections[0].id = id(n + 100);
    value.subscriptions[0].collection_ids = vec![id(n + 100)];
    value
}
fn configured(feeds: FeedSection) -> FeedConfiguration {
    FeedConfiguration::new(0, feeds).unwrap()
}

#[test]
fn shared_raw_vectors_have_identical_verdicts_and_preserve_values() {
    let corpus: Value =
        serde_json::from_slice(include_bytes!("fixtures/feeds-v1/conformance.json")).unwrap();
    let cases = corpus["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 102);
    for case in cases {
        let original = case["raw"].as_str().unwrap();
        let result = FeedSection::parse(original.as_bytes());
        assert_eq!(
            result.is_ok(),
            case["valid"].as_bool().unwrap(),
            "{}: {result:?}",
            case["id"]
        );
        if let Ok(value) = result {
            assert_eq!(
                serde_json::to_value(value).unwrap(),
                serde_json::from_str::<Value>(original).unwrap(),
                "{}",
                case["id"]
            );
        }
    }
    let provenance: Value =
        serde_json::from_slice(include_bytes!("fixtures/feeds-v1/provenance.json")).unwrap();
    assert_eq!(
        booknook_core::digest(include_bytes!("fixtures/feeds-v1/conformance.json")),
        provenance["sha256"]["conformance.json"]
    );
    assert_eq!(
        booknook_core::digest(include_bytes!("fixtures/feeds-v1/section.json")),
        provenance["sha256"]["section.json"]
    );
}

#[test]
fn raw_utf8_duplicate_numeric_and_embedded_limits_fail_closed() {
    assert!(FeedSection::parse(&[0xc3, 0x28]).is_err());
    let original = raw(&section());
    for alias in ["1.0", "1e0", "1.0000000000000001"] {
        let replaced = String::from_utf8(original.clone()).unwrap().replacen(
            "\"version\":1",
            &format!("\"version\":{alias}"),
            1,
        );
        assert!(FeedSection::parse(replaced.as_bytes()).is_err(), "{alias}");
    }
    let exact = [
        original.as_slice(),
        &vec![b' '; 256 * 1024 - original.len()],
    ]
    .concat();
    assert!(FeedSection::parse(&exact).is_ok());
    assert!(FeedSection::parse(&[exact.as_slice(), b" "].concat()).is_err());
    let mut invalid = section();
    invalid.subscriptions[0].title = "é".repeat(257);
    assert!(invalid.validate().is_err());
    invalid.subscriptions[0].title = "e\u{301} 👩‍👩‍👧‍👦".into();
    assert!(invalid.validate().is_ok());
    invalid.subscriptions[0].title.push('\u{0085}');
    assert!(invalid.validate().is_err());
}

#[test]
fn absent_empty_and_repeated_imports_preserve_data_without_activation() {
    let initial = configured(section());
    let before = initial.clone();
    let review = ImportReview::prepare(&initial, None).unwrap();
    assert_eq!(review.confirm(&initial, None).unwrap(), initial);
    let empty = raw(&FeedSection::empty());
    let review = ImportReview::prepare(&initial, Some(&empty)).unwrap();
    assert_eq!(review.confirm(&initial, Some(&empty)).unwrap(), initial);
    let incoming = raw(&initial.feeds);
    let review = ImportReview::prepare(&initial, Some(&incoming)).unwrap();
    assert!(review.conflicts().is_empty());
    assert_eq!(review.confirm(&initial, Some(&incoming)).unwrap(), initial);
    assert_eq!(initial, before);
    // The portable enabled preference is retained as data, with no activation record.
    assert!(initial.feeds.subscriptions[0].enabled);
    let encoded = serde_json::to_value(initial).unwrap();
    assert!(encoded.get("activation").is_none());
    assert!(encoded.get("identity").is_none());
}

#[test]
fn additive_review_preserves_order_and_requires_exact_input_and_configuration() {
    let current = configured(source(section(), 1));
    let added = source(section(), 2);
    let incoming = raw(&added);
    let review = ImportReview::prepare(&current, Some(&incoming)).unwrap();
    assert!(review.conflicts().is_empty());
    let candidate = review.candidate().unwrap();
    assert_eq!(candidate.revision, 1);
    assert_eq!(
        candidate
            .feeds
            .subscriptions
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        vec![id(1), id(2)]
    );
    assert_eq!(current.feeds.subscriptions.len(), 1);
    let committed = review.confirm(&current, Some(&incoming)).unwrap();
    assert_eq!(committed, *candidate);
    let equivalent_json = [incoming.as_slice(), b" "].concat();
    assert!(review.confirm(&current, Some(&equivalent_json)).is_err());
    assert!(review.confirm(&current, None).is_err());
    assert!(review.confirm(&committed, Some(&incoming)).is_err());
    let mut edited = current.clone();
    edited.feeds.subscriptions[0].title = "Changed after review".into();
    assert!(review.confirm(&edited, Some(&incoming)).is_err());
}

#[test]
fn conflicts_preserve_both_versions_until_exact_explicit_resolution() {
    let current = configured(source(section(), 1));
    let mut incoming_section = current.feeds.clone();
    incoming_section.subscriptions[0].title = "New interpretation".into();
    incoming_section.collections[0].title = "New collection name".into();
    let incoming = raw(&incoming_section);
    let review = ImportReview::prepare(&current, Some(&incoming)).unwrap();
    assert_eq!(review.conflicts().len(), 2);
    assert!(review.candidate().is_none());
    assert!(review.confirm(&current, Some(&incoming)).is_err());
    assert!(
        serde_json::to_string(&review.conflicts())
            .unwrap()
            .contains("New interpretation")
    );
    let resolutions = vec![
        ConflictResolution {
            target: ConflictTarget::Collection,
            id: id(101),
            choice: ResolutionChoice::Current,
        },
        ConflictResolution {
            target: ConflictTarget::Subscription,
            id: id(1),
            choice: ResolutionChoice::Incoming,
        },
    ];
    let resolved = review
        .resolve(&current, Some(&incoming), &resolutions)
        .unwrap();
    let committed = resolved.confirm(&current, Some(&incoming)).unwrap();
    assert_eq!(committed.feeds.subscriptions[0].title, "New interpretation");
    assert_eq!(
        committed.feeds.collections[0].title,
        current.feeds.collections[0].title
    );
    assert!(
        review
            .resolve(
                &current,
                Some(&incoming),
                &[resolutions[0].clone(), resolutions[0].clone()]
            )
            .is_err()
    );
    let mut unknown = resolutions;
    unknown[0].id = id(999);
    assert!(review.resolve(&current, Some(&incoming), &unknown).is_err());
}

#[test]
fn combined_limits_and_revision_exhaustion_fail_without_mutation() {
    let mut feeds = FeedSection::empty();
    let template = section().subscriptions.remove(0);
    for n in 0..256 {
        let mut source = template.clone();
        source.id = id(n);
        source.collection_ids.clear();
        feeds.subscriptions.push(source);
    }
    let current = configured(feeds);
    let before = current.clone();
    let added = raw(&source(section(), 999));
    assert!(ImportReview::prepare(&current, Some(&added)).is_err());
    assert_eq!(current, before);
    let exhausted = FeedConfiguration::new(9_007_199_254_740_991, FeedSection::empty()).unwrap();
    let incoming = raw(&section());
    assert!(ImportReview::prepare(&exhausted, Some(&incoming)).is_err());
    assert_eq!(
        ImportReview::prepare(&exhausted, None)
            .unwrap()
            .confirm(&exhausted, None)
            .unwrap(),
        exhausted
    );
}

fn selection() -> DisclosureSelection {
    DisclosureSelection::new(
        "https://reader.example.org".into(),
        vec![id(1)],
        vec![id(101)],
        vec![MembershipSelection {
            subscription_id: id(1),
            collection_ids: vec![id(101)],
        }],
    )
}
fn selected_config() -> FeedConfiguration {
    let mut feeds = source(section(), 1);
    feeds.subscriptions[0].disclosure = FeedDisclosure::Selected;
    let mut private = source(section(), 2);
    private.subscriptions[0].disclosure = FeedDisclosure::Private;
    feeds.subscriptions.extend(private.subscriptions);
    feeds.collections.extend(private.collections);
    configured(feeds)
}

fn non_string_enum_values(literal: &str) -> Vec<Value> {
    vec![
        Value::Null,
        json!(true),
        json!(1),
        json!(1.0),
        json!([]),
        json!([literal]),
        json!({literal:null}),
    ]
}

#[test]
fn literal_string_feed_enums_reject_tag_objects_and_every_non_string_shape() {
    for (path, literals) in [
        ("/schema", vec!["castalia.feeds-section.v1"]),
        ("/subscriptions/0/format", vec!["auto", "rss", "atom"]),
        ("/subscriptions/0/disclosure", vec!["private", "selected"]),
        ("/preferences/order", vec!["newest-first"]),
    ] {
        for literal in literals {
            let mut value = serde_json::to_value(section()).unwrap();
            *value.pointer_mut(path).unwrap() = json!(literal);
            assert_eq!(
                serde_json::to_value(FeedSection::parse(&raw(&value)).unwrap()).unwrap(),
                value
            );
            for non_string in non_string_enum_values(literal) {
                *value.pointer_mut(path).unwrap() = non_string;
                assert!(FeedSection::parse(&raw(&value)).is_err(), "{path}: {value}");
            }
        }
    }
    for non_string in non_string_enum_values("booknook.feed-configuration.v1") {
        let mut value = serde_json::to_value(selected_config()).unwrap();
        value["schema"] = non_string;
        assert!(FeedConfiguration::parse(&raw(&value)).is_err(), "{value}");
    }
    for non_string in non_string_enum_values("booknook.feed-disclosure-selection.v1") {
        let mut value = serde_json::to_value(selection()).unwrap();
        value["schema"] = non_string;
        assert!(DisclosureSelection::parse(&raw(&value)).is_err(), "{value}");
    }
    for (field, literals) in [
        ("target", vec!["collection", "subscription"]),
        ("choice", vec!["current", "incoming"]),
    ] {
        for literal in literals {
            let mut value = json!([{"target":"subscription","id":id(1),"choice":"incoming"}]);
            value[0][field] = json!(literal);
            assert_eq!(
                serde_json::to_value(ConflictResolution::parse_all(&raw(&value)).unwrap()).unwrap(),
                value
            );
            for non_string in non_string_enum_values(literal) {
                value[0][field] = non_string;
                assert!(
                    ConflictResolution::parse_all(&raw(&value)).is_err(),
                    "{value}"
                );
            }
        }
    }
}

fn export_selection() -> ExportSelection {
    ExportSelection::parse(&raw(&json!({
        "schema":"booknook.feed-export-selection.v1",
        "subscriptionIds":[id(1)], "collectionIds":[id(101)],
        "memberships":[{"subscriptionId":id(1),"collectionIds":[id(101)]}]
    })))
    .unwrap()
}

#[test]
fn portable_export_returns_reimportable_raw_section_without_mutating_source() {
    let mut current = selected_config();
    current.feeds.subscriptions[0].title = "e\u{301} 👩‍👩‍👧‍👦".into();
    current.feeds.subscriptions[0].url = "https://reader.example.org/%c3%a9/feed.xml".into();
    let before = current.clone();
    let review = ExportReview::prepare(&current, &export_selection()).unwrap();
    let exported = review.confirm(&current).unwrap();
    assert_eq!(review.candidate(), &exported);
    assert_eq!(exported.subscriptions, current.feeds.subscriptions[..1]);
    assert_eq!(exported.collections, current.feeds.collections[..1]);
    let encoded = raw(&exported);
    let value: Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(value["schema"], "castalia.feeds-section.v1");
    assert!(value.get("origin").is_none());
    assert!(value.get("feeds").is_none());
    assert_eq!(FeedSection::parse(&encoded).unwrap(), exported);
    let empty = FeedConfiguration::empty();
    assert_eq!(
        ImportReview::prepare(&empty, Some(&encoded))
            .unwrap()
            .confirm(&empty, Some(&encoded))
            .unwrap()
            .feeds,
        exported
    );
    assert_eq!(current, before);
    let equivalent = FeedConfiguration::parse(&[raw(&current), b" \n".to_vec()].concat()).unwrap();
    assert_eq!(review.confirm(&equivalent).unwrap(), exported);
    let mut stale = current.clone();
    stale.revision += 1;
    assert!(review.confirm(&stale).is_err());
    stale = current.clone();
    stale.feeds.subscriptions[0].title.push('!');
    assert!(review.confirm(&stale).is_err());
}

#[test]
fn portable_export_closes_labels_and_requires_explicit_eligible_memberships() {
    let current = selected_config();
    let before = current.clone();
    let mut chosen = export_selection();
    chosen.collection_ids.push(id(102));
    chosen.memberships[0].collection_ids.clear();
    let exported = ExportReview::prepare(&current, &chosen)
        .unwrap()
        .confirm(&current)
        .unwrap();
    assert!(exported.collections.is_empty());
    assert!(exported.subscriptions[0].collection_ids.is_empty());
    assert!(
        !String::from_utf8(raw(&exported))
            .unwrap()
            .contains(&id(102))
    );
    chosen.subscription_ids.clear();
    chosen.memberships.clear();
    assert_eq!(
        ExportReview::prepare(&current, &chosen)
            .unwrap()
            .confirm(&current)
            .unwrap(),
        FeedSection::empty()
    );
    let good = export_selection();
    let mut cases = Vec::new();
    let mut bad = good.clone();
    bad.subscription_ids.push(id(1));
    cases.push(bad);
    let mut bad = good.clone();
    bad.subscription_ids[0] = id(999);
    cases.push(bad);
    let mut bad = good.clone();
    bad.memberships.clear();
    cases.push(bad);
    let mut bad = good.clone();
    bad.memberships.push(bad.memberships[0].clone());
    cases.push(bad);
    let mut bad = good.clone();
    bad.collection_ids.clear();
    cases.push(bad);
    let mut bad = good.clone();
    bad.collection_ids.push(id(102));
    bad.memberships[0].collection_ids.push(id(102));
    cases.push(bad);
    let mut bad = good;
    bad.subscription_ids[0] = id(2);
    bad.memberships[0].subscription_id = id(2);
    bad.memberships[0].collection_ids.clear();
    cases.push(bad);
    for bad in cases {
        assert!(ExportReview::prepare(&current, &bad).is_err(), "{bad:?}");
    }
    assert_eq!(current, before);
}

#[test]
fn portable_export_selection_raw_json_is_closed_bounded_and_duplicate_safe() {
    let chosen = raw(&export_selection());
    let text = String::from_utf8(chosen.clone()).unwrap();
    for bad in [
        text.replacen("{", "{\"origin\":\"https://reader.example.org\",", 1),
        text.replacen("{", "{\"subscriptionIds\":[],", 1),
        text.replacen("{", "{\"subscription\\u0049ds\":[],", 1),
        text.replace(
            "\"schema\":\"booknook.feed-export-selection.v1\"",
            "\"schema\":{\"booknook.feed-export-selection.v1\":null}",
        ),
        text.replacen(
            "booknook.feed-export-selection.v1",
            "booknook.feed-disclosure-selection.v1",
            1,
        ),
    ] {
        assert!(ExportSelection::parse(bad.as_bytes()).is_err(), "{bad}");
    }
    assert!(ExportSelection::parse(&[0xc3, 0x28]).is_err());
    assert!(ExportSelection::parse(&[]).is_err());
    let exact = [chosen.as_slice(), &vec![b' '; 256 * 1024 - chosen.len()]].concat();
    assert!(ExportSelection::parse(&exact).is_ok());
    assert!(ExportSelection::parse(&[exact.as_slice(), b" "].concat()).is_err());
}

#[test]
fn projection_discloses_only_explicit_source_labels_and_memberships_to_exact_origin() {
    let current = selected_config();
    let before = current.clone();
    let chosen = selection();
    let review = ProjectionReview::prepare(&current, &chosen).unwrap();
    let projection = review
        .confirm(&current, "https://reader.example.org")
        .unwrap();
    assert_eq!(projection.origin(), "https://reader.example.org");
    assert_eq!(projection.feeds().subscriptions.len(), 1);
    assert_eq!(projection.feeds().collections.len(), 1);
    assert_eq!(
        projection.feeds().subscriptions[0].collection_ids,
        vec![id(101)]
    );
    let encoded = serde_json::to_string(&projection).unwrap();
    assert!(!encoded.contains(&id(2)));
    assert!(!encoded.contains(&id(102)));
    assert!(!encoded.contains("configurationDigest"));
    assert_eq!(current, before);
    for origin in [
        "https://other.example.org",
        "https://reader.example.org/",
        "https://reader.example.org:443",
        "https://reader.example.org.evil.org",
    ] {
        assert!(review.confirm(&current, origin).is_err());
    }
    let mut changed = current.clone();
    changed.revision += 1;
    assert!(
        review
            .confirm(&changed, "https://reader.example.org")
            .is_err()
    );
}

#[test]
fn labels_and_memberships_are_separate_choices_not_automatic_disclosure() {
    let current = selected_config();
    let mut chosen = selection();
    chosen.collection_ids.clear();
    chosen.memberships[0].collection_ids.clear();
    let review = ProjectionReview::prepare(&current, &chosen).unwrap();
    let projected = review
        .confirm(&current, "https://reader.example.org")
        .unwrap();
    assert!(projected.feeds().collections.is_empty());
    assert!(projected.feeds().subscriptions[0].collection_ids.is_empty());
    assert!(!current.feeds.subscriptions[0].collection_ids.is_empty());
    chosen.collection_ids = vec![id(101)];
    let review = ProjectionReview::prepare(&current, &chosen).unwrap();
    let projected = review
        .confirm(&current, "https://reader.example.org")
        .unwrap();
    assert!(projected.feeds().collections.is_empty());
    assert!(projected.feeds().subscriptions[0].collection_ids.is_empty());
}

#[test]
fn projection_rejects_private_unknown_duplicate_and_unreviewed_choices() {
    let current = selected_config();
    let good = selection();
    let mut cases = vec![];
    let mut private = good.clone();
    private.subscription_ids = vec![id(2)];
    private.memberships[0].subscription_id = id(2);
    private.memberships[0].collection_ids.clear();
    cases.push(private);
    let mut unknown = good.clone();
    unknown.subscription_ids = vec![id(999)];
    cases.push(unknown);
    let mut duplicate = good.clone();
    duplicate.subscription_ids.push(id(1));
    cases.push(duplicate);
    let mut missing_membership = good.clone();
    missing_membership.memberships.clear();
    cases.push(missing_membership);
    let mut unselected_label = good.clone();
    unselected_label.collection_ids.clear();
    cases.push(unselected_label);
    let mut foreign_membership = good.clone();
    foreign_membership.collection_ids.push(id(102));
    foreign_membership.memberships[0]
        .collection_ids
        .push(id(102));
    cases.push(foreign_membership);
    for case in cases {
        assert!(
            ProjectionReview::prepare(&current, &case).is_err(),
            "{case:?}"
        );
    }
    for origin in [
        "http://reader.example.org",
        "https://reader.example.org/",
        "https://READER.example.org",
        "https://user@reader.example.org",
        "https://reader.example.org?q=secret",
        "https://127.0.0.1",
    ] {
        let mut case = good.clone();
        case.origin = origin.into();
        assert!(
            ProjectionReview::prepare(&current, &case).is_err(),
            "{origin}"
        );
    }
}

#[test]
fn empty_selection_is_explicitly_empty_even_when_sources_are_selected() {
    let current = selected_config();
    let chosen =
        DisclosureSelection::new("https://reader.example.org".into(), vec![], vec![], vec![]);
    let review = ProjectionReview::prepare(&current, &chosen).unwrap();
    assert_eq!(
        *review
            .confirm(&current, "https://reader.example.org")
            .unwrap()
            .feeds(),
        FeedSection::empty()
    );
    // Closed outer config means a whole Castaway vault or identity cannot masquerade as a section.
    let wrong = json!({"schema":"castalia.castaway-contents.v3","version":3,"sections":{"feeds":section(),"identity":{}}});
    assert!(FeedSection::parse(&raw(&wrong)).is_err());
}

#[test]
fn empty_source_selection_cannot_disclose_orphan_collection_labels() {
    let mut current = selected_config();
    current.feeds.collections[1].title = "Unselected source private collection".into();
    let before = current.clone();
    let chosen = DisclosureSelection::new(
        "https://reader.example.org".into(),
        vec![],
        vec![id(102)],
        vec![],
    );
    let review = ProjectionReview::prepare(&current, &chosen).unwrap();
    let projection = review
        .confirm(&current, "https://reader.example.org")
        .unwrap();
    assert_eq!(*projection.feeds(), FeedSection::empty());
    let serialized = serde_json::to_string(&projection).unwrap();
    assert!(!serialized.contains(&id(102)));
    assert!(!serialized.contains("Unselected source private collection"));
    assert_eq!(current, before);
}

#[test]
fn selected_source_cannot_carry_labels_from_unselected_private_source() {
    let mut current = selected_config();
    current.feeds.collections[1].title = "Unselected source private collection".into();
    let before = current.clone();
    let mut chosen = selection();
    chosen.collection_ids = vec![id(102)];
    chosen.memberships[0].collection_ids.clear();
    let review = ProjectionReview::prepare(&current, &chosen).unwrap();
    let projection = review
        .confirm(&current, "https://reader.example.org")
        .unwrap();
    assert_eq!(projection.feeds().subscriptions.len(), 1);
    assert!(projection.feeds().collections.is_empty());
    assert!(
        projection.feeds().subscriptions[0]
            .collection_ids
            .is_empty()
    );
    let serialized = serde_json::to_string(&projection).unwrap();
    assert!(!serialized.contains(&id(102)));
    assert!(!serialized.contains("Unselected source private collection"));
    assert_eq!(current, before);
}

#[test]
fn conflict_choices_are_bound_to_the_original_review_not_reused_after_source_changes() {
    let current = configured(source(section(), 1));
    let mut incoming_section = current.feeds.clone();
    incoming_section.subscriptions[0].title = "Reviewed interpretation".into();
    let incoming = raw(&incoming_section);
    let review = ImportReview::prepare(&current, Some(&incoming)).unwrap();
    let choices = [ConflictResolution {
        target: ConflictTarget::Subscription,
        id: id(1),
        choice: ResolutionChoice::Incoming,
    }];
    assert!(review.resolve(&current, Some(&incoming), &[]).is_err());
    incoming_section.subscriptions[0].title =
        "Different interpretation requiring new review".into();
    let changed_input = raw(&incoming_section);
    assert!(
        review
            .resolve(&current, Some(&changed_input), &choices)
            .is_err()
    );
    assert!(
        review
            .resolve(
                &current,
                Some(&[incoming.as_slice(), b" "].concat()),
                &choices
            )
            .is_err()
    );
    let mut changed_configuration = current.clone();
    changed_configuration.revision += 1;
    assert!(
        review
            .resolve(&changed_configuration, Some(&incoming), &choices)
            .is_err()
    );
    let mut same_revision_edit = current.clone();
    same_revision_edit.feeds.subscriptions[0].title = "Local edit after review".into();
    assert!(
        review
            .resolve(&same_revision_edit, Some(&incoming), &choices)
            .is_err()
    );
    let resolved = review.resolve(&current, Some(&incoming), &choices).unwrap();
    let confirmed = resolved.confirm(&current, Some(&incoming)).unwrap();
    assert_eq!(
        confirmed.feeds.subscriptions[0].title,
        "Reviewed interpretation"
    );
    assert!(
        resolved
            .resolve(&current, Some(&incoming), &choices)
            .is_err()
    );
    assert_eq!(
        resolved
            .resolve(&current, Some(&incoming), &[])
            .unwrap()
            .confirm(&current, Some(&incoming))
            .unwrap(),
        confirmed
    );
    assert_eq!(current.revision, 0);
}

#[test]
fn raw_binding_requests_remain_bounded_and_reject_duplicates_before_typed_decoding() {
    let selection = raw(&selection());
    assert!(DisclosureSelection::parse(&selection).is_ok());
    let selection_text = String::from_utf8(selection.clone()).unwrap();
    let duplicate = selection_text.replacen(
        "\"origin\":",
        "\"origin\":\"https://other.example.org\",\"origin\":",
        1,
    );
    assert!(DisclosureSelection::parse(duplicate.as_bytes()).is_err());
    assert!(DisclosureSelection::parse(&[0xc3, 0x28]).is_err());
    let exact = [
        selection.as_slice(),
        &vec![b' '; 256 * 1024 - selection.len()],
    ]
    .concat();
    assert!(DisclosureSelection::parse(&exact).is_ok());
    assert!(DisclosureSelection::parse(&[exact.as_slice(), b" "].concat()).is_err());
    let choices = json!([{"target":"subscription","id":id(1),"choice":"incoming"}]);
    assert_eq!(
        ConflictResolution::parse_all(&raw(&choices)).unwrap().len(),
        1
    );
    assert!(
        ConflictResolution::parse_all(b"[{\"target\":\"subscription\",\"target\":\"collection\"}]")
            .is_err()
    );
    assert!(
        ConflictResolution::parse_all(&[b"[]".as_slice(), &vec![b' '; 256 * 1024]].concat())
            .is_err()
    );
    assert!(ConflictResolution::parse_all(&raw(&vec![choices[0].clone(); 321])).is_err());
}
