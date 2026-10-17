// SPDX-License-Identifier: MIT OR Apache-2.0
use booknook_core::{Command, apply, canonical, model::*, validate};
fn fixture() -> (State, String, String, String) {
    let mut s = State {
        local_reader: "PRIVATE-READER".into(),
        ..Default::default()
    };
    apply(
        &mut s,
        Command::AddBook {
            title: "A reading of café 🌿".into(),
            author: "Visible Writer".into(),
            at: 1,
        },
    )
    .unwrap();
    let n = s.notebooks.values().next().unwrap().clone();
    apply(
        &mut s,
        Command::EditBlock {
            capture_context: None,
            note_creation: None,
            notebook_id: n.id.clone(),
            block_id: n.blocks[0].id.clone(),
            text: "PRIVATE NEIGHBOURING NOTE".into(),
            base_update: n.blocks[0].update.clone(),
            at: 2,
        },
    )
    .unwrap();
    let result = apply(
        &mut s,
        Command::AddCharacter {
            capture_context: None,
            note_creation: None,
            notebook_id: n.id.clone(),
            block_id: n.blocks[0].id.clone(),
            name: "Mara".into(),
            at: 3,
        },
    )
    .unwrap();
    let card = result["card_id"].as_str().unwrap().to_string();
    let parent = s.cards[&card].versions[0].id.clone();
    let place = apply(
        &mut s,
        Command::CreateLiteraryEntity {
            book_id: n.book_id.clone(),
            kind: LiteraryKind::Place,
            name: "The Orchard".into(),
        },
    )
    .unwrap()["entity_id"]
        .as_str()
        .unwrap()
        .to_string();
    let edition = s.books[&n.book_id].edition_ids[0].clone();
    let interpretation = CharacterInterpretation {
        description: "Early impression: kind".into(),
        traits: vec!["Curious".into(), "Reserved".into()],
        relationships: vec![Relationship {
            target: place,
            description: "Finds solace here".into(),
        }],
        observations: vec!["Helps a stranger".into()],
        questions: vec!["Why does she leave?".into()],
        passages: vec![
            Location::Text {
                edition_id: edition,
                resource: "PRIVATE-resource.xhtml".into(),
                quote: "She waited by the apple tree.".into(),
                prefix: "PRIVATE PREFIX".into(),
                suffix: "PRIVATE SUFFIX".into(),
            },
            Location::Audio {
                provider: "PRIVATE-PROVIDER".into(),
                source_id: "PRIVATE-SOURCE".into(),
                milliseconds: 1234567,
                chapter: Some("PRIVATE CHAPTER".into()),
            },
        ],
    };
    let old = apply(
        &mut s,
        Command::SaveCard {
            card_id: card.clone(),
            parents: vec![parent.clone()],
            interpretation,
            positions: vec![Location::Provider {
                provider: "PRIVATE-PROVIDER".into(),
                source_id: "PRIVATE-SOURCE".into(),
                value: "PRIVATE-POSITION".into(),
            }],
            at: 30,
        },
    )
    .unwrap()["version_id"]
        .as_str()
        .unwrap()
        .to_string();
    let later = apply(
        &mut s,
        Command::SaveCard {
            card_id: card.clone(),
            parents: vec![parent],
            interpretation: CharacterInterpretation {
                description: "Later branch: unreliable".into(),
                questions: vec!["Was it deception?".into()],
                ..Default::default()
            },
            positions: vec![],
            at: 120,
        },
    )
    .unwrap()["version_id"]
        .as_str()
        .unwrap()
        .to_string();
    (s, card, old, later)
}
fn draft(state: &mut State, card: &str, version: &str) -> ShareDraft {
    let result = apply(
        state,
        Command::PrepareCharacterShare {
            card_id: card.into(),
            version_id: version.into(),
            at: 200,
        },
    )
    .unwrap();
    assert_eq!(result["published"], false);
    state.drafts[result["draft_id"].as_str().unwrap()].clone()
}
#[test]
fn selected_old_and_branch_versions_become_separate_private_history_free_drafts() {
    let (mut s, card, old, later) = fixture();
    let before = s.clone();
    let first = draft(&mut s, &card, &old);
    let second = draft(&mut s, &card, &later);
    let repeated = draft(&mut s, &card, &old);
    assert_eq!(first.body, repeated.body);
    assert_ne!(first.id, repeated.id);
    assert_eq!(first.kind, "selected_note");
    assert!(first.book_id.is_none());
    assert!(first.spoiler);
    for text in [
        "Book: A reading of café 🌿",
        "Authors: Visible Writer",
        "Character: Mara",
        "Early impression: kind",
        "Curious",
        "Reserved",
        "The Orchard: Finds solace here",
        "Helps a stranger",
        "Why does she leave?",
        "She waited by the apple tree.",
        "1 supporting reference(s) have no stored passage text",
    ] {
        assert!(first.body.contains(text), "missing {text}");
    }
    assert!(!first.body.contains("Later branch"));
    assert!(!first.body.contains("Was it deception"));
    assert!(second.body.contains("Later branch: unreliable"));
    assert!(!second.body.contains("Early impression"));
    assert!(!second.body.contains("apple tree"));
    let bytes = String::from_utf8(canonical(&s.drafts).unwrap()).unwrap();
    assert!(!bytes.contains("PRIVATE"));
    for private in before
        .notebooks
        .keys()
        .chain(before.passes.keys())
        .chain(before.editions.keys())
        .chain(before.cards.keys())
        .chain(before.entities.keys())
        .chain(before.books.keys())
    {
        assert!(!bytes.contains(private));
    }
    for n in before.notebooks.values() {
        for block in &n.blocks {
            assert!(!bytes.contains(&block.id));
            assert!(!bytes.contains(&block.update));
        }
    }
    for v in &before.cards[&card].versions {
        assert!(!bytes.contains(&v.id));
    }
    assert_eq!(s.cards, before.cards);
    assert_eq!(s.notebooks, before.notebooks);
    assert_eq!(s.passes, before.passes);
    assert!(s.activity.is_empty());
    let restored: State = booknook_core::strict_parse(&canonical(&s).unwrap()).unwrap();
    validate(&restored).unwrap();
    assert_eq!(restored.drafts, s.drafts);
}
#[test]
fn missing_or_wrong_card_version_fails_atomically() {
    let (mut s, card, old, _) = fixture();
    let before = s.clone();
    for (selected_card, selected_version) in [("missing", old.as_str()), (card.as_str(), "missing")]
    {
        assert!(
            apply(
                &mut s,
                Command::PrepareCharacterShare {
                    card_id: selected_card.into(),
                    version_id: selected_version.into(),
                    at: 200
                }
            )
            .is_err()
        );
        assert_eq!(s, before);
    }
    let n = s.notebooks.values().next().unwrap().clone();
    let other = apply(
        &mut s,
        Command::AddCharacter {
            capture_context: None,
            note_creation: None,
            notebook_id: n.id,
            block_id: n.blocks[0].id.clone(),
            name: "Another character".into(),
            at: 201,
        },
    )
    .unwrap()["card_id"]
        .as_str()
        .unwrap()
        .to_string();
    let before = s.clone();
    assert!(
        apply(
            &mut s,
            Command::PrepareCharacterShare {
                card_id: other,
                version_id: old,
                at: 202
            }
        )
        .is_err()
    );
    assert_eq!(s, before);
}
#[test]
fn oversize_or_invalid_text_is_never_truncated_into_a_draft() {
    let (mut s, card, old, _) = fixture();
    for description in ["🌿".repeat(3000), "Contains\0a null".into()] {
        s.cards
            .get_mut(&card)
            .unwrap()
            .versions
            .iter_mut()
            .find(|v| v.id == old)
            .unwrap()
            .interpretation
            .description = description;
        let before = s.clone();
        let error = apply(
            &mut s,
            Command::PrepareCharacterShare {
                card_id: card.clone(),
                version_id: old.clone(),
                at: 300,
            },
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("No draft was created"));
        assert_eq!(s, before);
    }
    // Existing portable drafts remain readable; only new creation receives the early bound.
    s.drafts.insert(
        "old-draft".into(),
        ShareDraft {
            id: "old-draft".into(),
            created_at: 0,
            kind: "selected_note".into(),
            book_id: None,
            body: "x".repeat(12001),
            spoiler: false,
        },
    );
    validate(&s).unwrap();
    let before = s.clone();
    assert!(
        apply(
            &mut s,
            Command::PrepareShare {
                kind: "selected_note".into(),
                book_id: None,
                body: "x".repeat(12001),
                spoiler: false,
                at: 300
            }
        )
        .is_err()
    );
    assert_eq!(s, before);
}
