use booknook_core::{model::*, *};

fn desk() -> State {
    let mut s = State::default();
    apply(
        &mut s,
        Command::AddBook {
            title: "A book".into(),
            author: "A writer".into(),
            at: 1,
        },
    )
    .unwrap();
    s
}

#[test]
fn goodreads_reimport_preserves_personal_edits_and_provenance() {
    let csv = "Book Id,Title,Author,My Rating,My Review,Exclusive Shelf,Date Read,ISBN13\n42,Example,Writer,4,First impression,read,2025/01/03,=\"9780000000001\"\n";
    let mut s = State::default();
    let report = imports::goodreads(&mut s, csv, 1).unwrap();
    assert_eq!(report.inserted, 1);
    assert_eq!(imports::goodreads(&mut s, csv, 2).unwrap().unchanged, 1);
    let book = s.books.values().next().unwrap().clone();
    apply(
        &mut s,
        Command::UpdateBook {
            book_id: book.id.clone(),
            title: book.title,
            shelf: "read".into(),
            rating: Some(5),
            review: "My newer review".into(),
        },
    )
    .unwrap();
    imports::goodreads(
        &mut s,
        &csv.replace("First impression", "Updated source"),
        3,
    )
    .unwrap();
    assert_eq!(s.books.len(), 1);
    assert_eq!(s.books[&book.id].review, "My newer review");
    assert_eq!(s.books[&book.id].rating, Some(5));
    assert_eq!(s.books[&book.id].provenance.len(), 2);
}
#[test]
fn book_subject_edits_are_normalized_private_and_survive_import() {
    let csv = "Book Id,Title,Author,My Rating\n42,Example,Writer,4\n";
    let mut state = State::default();
    imports::goodreads(&mut state, csv, 1).unwrap();
    let id = state.books.keys().next().unwrap().clone();
    let command: Command = serde_json::from_value(serde_json::json!({
        "op": "set_book_subjects", "book_id": id, "subjects": [" Nature ", "nature", "Fiction"]
    }))
    .unwrap();
    apply(&mut state, command).unwrap();
    assert_eq!(state.books[&id].subjects, ["fiction", "nature"]);
    assert!(state.books[&id].edited_fields.contains("subjects"));
    imports::goodreads(&mut state, &csv.replace("Example", "Updated"), 2).unwrap();
    assert_eq!(state.books[&id].subjects, ["fiction", "nature"]);
    assert!(state.activity.is_empty());
    let before = state.clone();
    assert!(
        apply(
            &mut state,
            Command::SetBookSubjects {
                book_id: id,
                subjects: vec!["x".repeat(129)]
            }
        )
        .is_err()
    );
    assert_eq!(state, before);
}

#[test]
fn malformed_csv_is_atomic() {
    let mut s = State::default();
    let before = s.clone();
    assert!(
        imports::goodreads(
            &mut s,
            "Book Id,Title,Author,My Rating\n1,Good,Author,4\n2,Bad,Author,9\n",
            1
        )
        .is_err()
    );
    assert_eq!(s, before);
}
#[test]
fn kindle_catalog_without_content_still_has_a_notebook() {
    let mut s = State::default();
    imports::kindle(&mut s,r#"{"schema":"booknook.kindle-catalog.v1","items":[{"asin":"B000000001","title":"Owned book","authors":["Author"],"position":"kindle-location:35","observed_at":50}]}"#,60).unwrap();
    assert_eq!(s.books.len(), 1);
    assert!(s.editions.values().next().unwrap().assets.is_empty());
    assert_eq!(s.positions.len(), 1);
    assert_eq!(s.notebooks.len(), 1);
    validate(&s).unwrap();
}

#[test]
fn annotations_cannot_silently_move_between_books() {
    let mut s = desk();
    let pass = s.passes.keys().next().unwrap().clone();
    apply(
        &mut s,
        Command::AddBook {
            title: "Another book".into(),
            author: "Writer".into(),
            at: 2,
        },
    )
    .unwrap();
    let other = s
        .editions
        .values()
        .find(|edition| edition.book_id != s.passes[&pass].book_id)
        .unwrap()
        .id
        .clone();
    let before = s.clone();
    assert!(
        apply(
            &mut s,
            Command::SaveAnnotation {
                annotation: Annotation {
                    id: "mismatched".into(),
                    pass_id: pass,
                    anchor: Location::Text {
                        edition_id: other,
                        resource: "chapter.xhtml".into(),
                        quote: "Wrong book".into(),
                        prefix: "".into(),
                        suffix: "".into()
                    },
                    original_context: "Wrong book".into(),
                    layout_fingerprint: None,
                    strokes: vec![],
                    native_drawing: None,
                    ink_geometry: None,
                }
            }
        )
        .is_err()
    );
    assert_eq!(s, before);
}
#[test]
fn unicode_edits_and_concurrent_offline_changes_converge() {
    let base = notebook::edit("", "こんにちは 🌿 café e\u{301}").unwrap();
    let a = notebook::edit(&base, "Left こんにちは 🌿 café e\u{301}").unwrap();
    let b = notebook::edit(&base, "こんにちは 🌿 café e\u{301} Right").unwrap();
    let (ab, text) = notebook::merge(&a, &b).unwrap();
    let (_, reverse) = notebook::merge(&b, &a).unwrap();
    assert_eq!(text, reverse);
    assert_eq!(text, "Left こんにちは 🌿 café e\u{301} Right");
    assert_eq!(notebook::merge(&ab, &a).unwrap().1, text);
    let changed = notebook::edit(&ab, "Left こんにちは 🦉 café e\u{301} Right").unwrap();
    assert!(notebook::text(&changed).unwrap().contains('🦉'));
}
#[test]
fn long_document_edits_preserve_unrelated_content() {
    let text = "A quiet thought 🌱\n".repeat(12000);
    let first = notebook::edit("", &text).unwrap();
    let appended = format!("{text}最後");
    let second = notebook::edit(&first, &appended).unwrap();
    assert_eq!(notebook::text(&second).unwrap(), appended);
}
#[test]
fn card_history_branches_and_explicit_reconciliation_survive_export() {
    let mut s = desk();
    let n = s.notebooks.values().next().unwrap().clone();
    apply(
        &mut s,
        Command::AddCharacter {
            capture_context: None,
            note_creation: None,
            notebook_id: n.id,
            block_id: n.blocks[0].id.clone(),
            name: "Mara".into(),
            at: 3,
        },
    )
    .unwrap();
    let c = s.cards.values().next().unwrap().clone();
    let parent = c.versions[0].id.clone();
    for description in [
        "Chapter 3: I trust her.",
        "Chapter 12: She may be manipulating them.",
    ] {
        apply(
            &mut s,
            Command::SaveCard {
                card_id: c.id.clone(),
                parents: vec![parent.clone()],
                interpretation: CharacterInterpretation {
                    description: description.into(),
                    ..Default::default()
                },
                positions: vec![],
                at: 4,
            },
        )
        .unwrap();
    }
    assert_eq!(s.cards[&c.id].heads().len(), 2);
    let exported = canonical(&s).unwrap();
    let mut restored: State = strict_parse(&exported).unwrap();
    validate(&restored).unwrap();
    assert_eq!(s, restored);
    let parents = restored.cards[&c.id]
        .heads()
        .iter()
        .map(|v| v.id.clone())
        .collect();
    apply(
        &mut restored,
        Command::SaveCard {
            card_id: c.id.clone(),
            parents,
            interpretation: CharacterInterpretation {
                description: "Both readings matter".into(),
                ..Default::default()
            },
            positions: vec![],
            at: 5,
        },
    )
    .unwrap();
    assert_eq!(restored.cards[&c.id].heads().len(), 1);
    assert_eq!(restored.cards[&c.id].versions.len(), 4);
}
#[test]
fn duplicate_keys_unknown_fields_and_future_schema_fail() {
    assert!(strict_parse::<serde_json::Value>(br#"{"a":1,"a":2}"#).is_err());
    assert!(
        strict_parse::<Command>(
            br#"{"op":"add_book","title":"T","author":"A","at":0,"admin":true}"#
        )
        .is_err()
    );
    let mut s = desk();
    s.ontology_version = "999".into();
    assert!(validate(&s).is_err());
}
#[test]
fn anchors_never_guess_repeated_text_or_mismatched_layout() {
    assert_eq!(
        anchors::resolve("a rose, a rose", "rose", "", ""),
        anchors::Resolution::Ambiguous {
            matches: 2,
            context: "rose".into()
        }
    );
    assert_eq!(
        anchors::resolve("🌳 a rose, a rose", "rose", "🌳 a ", ", a "),
        anchors::Resolution::Resolved {
            start_utf16: 5,
            end_utf16: 9
        }
    );
    assert!(!anchors::ink_can_place("portrait", "landscape", true));
    assert!(!anchors::ink_can_place("same", "same", false));
}
#[test]
fn share_selection_contains_no_surrounding_notebook_history() {
    let mut s = desk();
    let n = s.notebooks.values().next().unwrap().clone();
    apply(
        &mut s,
        Command::EditBlock {
            capture_context: None,
            note_creation: None,
            notebook_id: n.id.clone(),
            block_id: n.blocks[0].id.clone(),
            text: "SECRET private reflection".into(),
            base_update: n.blocks[0].update.clone(),
            at: 2,
        },
    )
    .unwrap();
    apply(
        &mut s,
        Command::PrepareShare {
            kind: "selected_note".into(),
            book_id: Some(n.book_id),
            body: "Only this sentence".into(),
            spoiler: false,
            at: 3,
        },
    )
    .unwrap();
    let bytes = String::from_utf8(canonical(&s.drafts).unwrap()).unwrap();
    assert!(!bytes.contains("SECRET"));
    assert!(!bytes.contains("update"));
    assert!(s.activity.is_empty());
}
#[test]
fn feed_requires_mutual_friendship_deduplicates_and_respects_blocking() {
    let mut s = State::default();
    let me = s.local_reader.clone();
    s.friendship_requests.insert((me.clone(), "friend".into()));
    for (id, at) in [("a", 10), ("b", 20)] {
        s.activity.insert(
            id.into(),
            Activity {
                id: id.into(),
                actor: "friend".into(),
                published_at: at,
                kind: "review".into(),
                book_id: None,
                body: "hello".into(),
                spoiler: false,
                group_id: None,
                group_epoch: None,
            },
        );
    }
    assert!(social::feed(&s, &me).is_empty());
    s.friendship_requests.insert(("friend".into(), me.clone()));
    assert_eq!(social::feed(&s, &me)[0].id, "b");
    s.blocked.insert("friend".into());
    assert!(social::feed(&s, &me).is_empty());
}
#[test]
fn recommendation_contract_cannot_include_private_notebooks() {
    use social::*;
    let input = RecommendationInput {
        candidates: vec![CatalogCandidate {
            book_id: "b".into(),
            title: "Trees".into(),
            subjects: vec![" Nature ".into(), "nature".into()],
        }],
        preferred_subjects: vec!["NATURE".into()],
        shared_ratings: vec![],
        friends: Default::default(),
        excluded: Default::default(),
    };
    let results = recommend(&input);
    assert_eq!(results[0].score, 2);
    assert_eq!(results[0].reasons, ["Matches your interest in nature"]);
    let mut raw = serde_json::to_value(input).unwrap();
    raw["notebooks"] = serde_json::json!(["secret"]);
    assert!(serde_json::from_value::<RecommendationInput>(raw).is_err());
}
#[test]
fn stale_audio_is_not_an_ebook_percentage() {
    let p = ProgressObservation {
        provider_connection_id: None,
        id: "p".into(),
        pass_id: "pass".into(),
        location: Location::Audio {
            provider: "audible".into(),
            source_id: "ASIN".into(),
            milliseconds: 5000,
            chapter: None,
        },
        observed_at: 10,
        received_at: 100,
        alignment: Alignment::Unmatched,
        audio_metadata: None,
    };
    assert!(p.stale(100, 30));
    assert!(!p.stale(20, 30));
    assert_eq!(p.alignment, Alignment::Unmatched);
}

#[test]
fn rich_text_styles_survive_unicode_edit_merge_and_export() {
    let initial = notebook::rich_edit(
        "",
        "A 🌿 thought",
        &[InlineStyle {
            start_utf16: 2,
            length_utf16: 2,
            bold: true,
            italic: false,
            underline: true,
        }],
    )
    .unwrap();
    let styles = notebook::styles(&initial).unwrap();
    assert_eq!(styles.len(), 1);
    assert!(styles[0].bold && styles[0].underline);
    let a = notebook::edit(&initial, "A 🌿 thought blooms").unwrap();
    let b = notebook::edit(&initial, "A 🌿 bright thought").unwrap();
    let (merged, text) = notebook::merge(&a, &b).unwrap();
    assert!(text.contains("bright") && text.contains("blooms"));
    assert_eq!(notebook::styles(&merged).unwrap(), styles);
    assert!(
        notebook::rich_edit(
            &initial,
            "A 🌿 thought",
            &[InlineStyle {
                start_utf16: 3,
                length_utf16: 1,
                bold: true,
                italic: false,
                underline: false
            }]
        )
        .is_err()
    );
}

#[test]
fn card_history_merges_offline_branches_without_losing_interpretations() {
    let mut original = desk();
    let n = original.notebooks.values().next().unwrap().clone();
    apply(
        &mut original,
        Command::AddCharacter {
            capture_context: None,
            note_creation: None,
            notebook_id: n.id,
            block_id: n.blocks[0].id.clone(),
            name: "Mara".into(),
            at: 2,
        },
    )
    .unwrap();
    let c = original.cards.values().next().unwrap().clone();
    let mut left = original.clone();
    let mut right = original;
    for (state, description) in [(&mut left, "Trust"), (&mut right, "Doubt")] {
        apply(
            state,
            Command::SaveCard {
                card_id: c.id.clone(),
                parents: vec![c.versions[0].id.clone()],
                interpretation: CharacterInterpretation {
                    description: description.into(),
                    ..Default::default()
                },
                positions: vec![],
                at: 3,
            },
        )
        .unwrap();
    }
    let merged = history::merge_card(&left.cards[&c.id], &right.cards[&c.id]).unwrap();
    assert_eq!(merged.heads().len(), 2);
    assert_eq!(merged.versions.len(), 3);
    assert_eq!(
        merged,
        history::merge_card(&right.cards[&c.id], &left.cards[&c.id]).unwrap()
    );
    assert_eq!(merged, history::merge_card(&merged, &merged).unwrap());
}
