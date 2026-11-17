use booknook_core::{
    Command, apply,
    model::*,
    notebook_command::{CardContextCommand, NotebookCommand},
    validate,
};

fn desk() -> State {
    let mut state = State::default();
    apply(
        &mut state,
        Command::AddBook {
            title: "Fixture".into(),
            author: "Writer".into(),
            at: 10,
        },
    )
    .unwrap();
    state
}

#[test]
fn explicit_sessions_keep_their_recorded_interval_and_reject_invalid_changes_atomically() {
    let mut state = desk();
    let pass_id = state.passes.keys().next().unwrap().clone();
    let before = state.clone();
    for pass in ["missing".to_owned(), pass_id.clone()] {
        assert!(
            apply(
                &mut state,
                Command::StartSession {
                    pass_id: pass,
                    at: 9
                }
            )
            .is_err()
        );
        assert_eq!(state, before);
    }
    let id = apply(
        &mut state,
        Command::StartSession {
            pass_id: pass_id.clone(),
            at: 20,
        },
    )
    .unwrap()["session_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let active = state.clone();
    assert!(
        apply(
            &mut state,
            Command::EndSession {
                session_id: id.clone(),
                at: 19
            }
        )
        .is_err()
    );
    assert_eq!(state, active);
    apply(
        &mut state,
        Command::EndSession {
            session_id: id.clone(),
            at: 40,
        },
    )
    .unwrap();
    assert_eq!(state.sessions[&id].ended_at, Some(40));
    let ended = state.clone();
    assert!(
        apply(
            &mut state,
            Command::EndSession {
                session_id: id.clone(),
                at: 41
            }
        )
        .is_err()
    );
    assert_eq!(state, ended);
    apply(
        &mut state,
        Command::EndSession {
            session_id: id.clone(),
            at: 40,
        },
    )
    .unwrap();
    assert_eq!(state.sessions[&id], ended.sessions[&id]);
    let restarted = apply(&mut state, Command::StartSession { pass_id, at: 50 }).unwrap();
    assert_ne!(restarted["session_id"].as_str().unwrap(), id);
    assert_eq!(state.sessions.len(), 2);
    let restored: State =
        booknook_core::strict_parse(&booknook_core::canonical(&state).unwrap()).unwrap();
    validate(&restored).unwrap();
    assert_eq!(restored, state);
}

#[test]
fn session_validation_rejects_dangling_references_and_negative_or_out_of_pass_intervals() {
    let mut valid = desk();
    let pass = valid.passes.keys().next().unwrap().clone();
    let id = apply(
        &mut valid,
        Command::StartSession {
            pass_id: pass.clone(),
            at: 20,
        },
    )
    .unwrap()["session_id"]
        .as_str()
        .unwrap()
        .to_owned();
    for kind in 0..5 {
        let mut invalid = valid.clone();
        match kind {
            0 => invalid.sessions.get_mut(&id).unwrap().pass_id = "missing".into(),
            1 => invalid.sessions.get_mut(&id).unwrap().started_at = 9,
            2 => invalid.sessions.get_mut(&id).unwrap().ended_at = Some(19),
            3 => invalid.passes.get_mut(&pass).unwrap().finished_at = Some(5),
            _ => {
                invalid.passes.get_mut(&pass).unwrap().finished_at = Some(25);
                invalid.sessions.get_mut(&id).unwrap().ended_at = Some(26);
            }
        }
        assert!(validate(&invalid).is_err(), "invalid interval case {kind}");
    }
    valid.passes.get_mut(&pass).unwrap().finished_at = Some(25);
    let before = valid.clone();
    assert!(
        apply(
            &mut valid,
            Command::EndSession {
                session_id: id,
                at: 26
            }
        )
        .is_err()
    );
    assert_eq!(valid, before);
    assert!(
        apply(
            &mut valid,
            Command::StartSession {
                pass_id: pass,
                at: 26
            }
        )
        .is_err()
    );
    assert_eq!(valid, before);
}

#[test]
fn literature_specializations_are_book_scoped_relationship_targets_without_directory_authority() {
    let mut state = desk();
    let book_id = state.books.keys().next().unwrap().clone();
    let reader = state.local_reader.clone();
    let mut ids = vec![];
    for (kind, name) in [
        (LiteraryKind::Place, " The island "),
        (LiteraryKind::Theme, "Memory"),
        (LiteraryKind::Event, "The return"),
        (LiteraryKind::Concept, "Trust"),
    ] {
        let result = apply(
            &mut state,
            Command::CreateLiteraryEntity {
                book_id: book_id.clone(),
                kind: kind.clone(),
                name: name.into(),
            },
        )
        .unwrap();
        let id = result["entity_id"].as_str().unwrap().to_owned();
        assert_eq!(state.entities[&id].kind, kind);
        assert_eq!(state.entities[&id].name, name.trim());
        assert_eq!(state.entities[&id].book_id, book_id);
        ids.push(id);
    }
    let notebook = state.notebooks.values().next().unwrap().clone();
    let card = apply(
        &mut state,
        Command::AddCharacter {
            capture_context: None,
            note_creation: None,
            notebook_id: notebook.id,
            block_id: notebook.blocks[0].id.clone(),
            name: "Mara".into(),
            at: 30,
        },
    )
    .unwrap()["card_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let parents = vec![state.cards[&card].versions[0].id.clone()];
    apply(
        &mut state,
        Command::SaveCard {
            card_id: card,
            parents,
            at: 31,
            positions: vec![],
            interpretation: CharacterInterpretation {
                relationships: ids
                    .into_iter()
                    .map(|target| Relationship {
                        target,
                        description: "Related to Mara’s interpretation".into(),
                    })
                    .collect(),
                ..Default::default()
            },
        },
    )
    .unwrap();
    assert_eq!(state.local_reader, reader);
    assert!(state.groups.is_empty() && state.activity.is_empty());
    let before = state.clone();
    for (book, kind, name) in [
        (book_id.clone(), LiteraryKind::Character, "Wrong API"),
        (book_id.clone(), LiteraryKind::Place, "  "),
        ("missing".into(), LiteraryKind::Place, "Place"),
    ] {
        assert!(
            apply(
                &mut state,
                Command::CreateLiteraryEntity {
                    book_id: book,
                    kind,
                    name: name.into()
                }
            )
            .is_err()
        );
        assert_eq!(state, before);
    }
    let restored: State =
        booknook_core::strict_parse(&booknook_core::canonical(&state).unwrap()).unwrap();
    validate(&restored).unwrap();
    assert_eq!(restored, state);
}

#[test]
fn captured_card_versions_keep_exact_observations_when_source_positions_advance() {
    let mut state = desk();
    let notebook = state.notebooks.values().next().unwrap().clone();
    let edition = state.editions.keys().next().unwrap().clone();
    let card_id = apply(
        &mut state,
        Command::AddCharacter {
            notebook_id: notebook.id.clone(),
            block_id: notebook.blocks[0].id.clone(),
            name: "Mara".into(),
            at: 11,
            capture_context: None,
            note_creation: None,
        },
    )
    .unwrap()["card_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let original = state.cards[&card_id].versions[0].clone();
    let selected = Location::Text {
        edition_id: edition,
        resource: "OPS/chapter.xhtml".into(),
        quote: "The café opened. 日本語".into(),
        prefix: "Earlier. ".into(),
        suffix: " Later.".into(),
    };
    let listening = Location::Audio {
        provider: "audible".into(),
        source_id: "fixture-audio".into(),
        milliseconds: 32000,
        chapter: Some("Chapter 3".into()),
    };
    let captured = ProgressObservation {
        id: "capture-before-commit".into(),
        pass_id: notebook.pass_id.clone(),
        location: listening.clone(),
        observed_at: 19,
        received_at: 20,
        alignment: Alignment::Unmatched,
        provider_connection_id: Some("audible-fixture".into()),
        audio_metadata: None,
    };
    let later = ProgressObservation {
        id: "later-position".into(),
        observed_at: 30,
        received_at: 30,
        location: Location::Audio {
            provider: "audible".into(),
            source_id: "fixture-audio".into(),
            milliseconds: 60000,
            chapter: Some("Chapter 12".into()),
        },
        ..captured.clone()
    };
    apply(
        &mut state,
        Command::ObservePosition {
            observation: later.clone(),
        },
    )
    .unwrap();

    let context = CapturedReadingContext {
        pass_id: notebook.pass_id.clone(),
        positions: vec![listening, selected.clone()],
        position_observations: vec![captured],
    };
    let interpretation = CharacterInterpretation {
        description: "Earlier impression".into(),
        passages: vec![selected.clone()],
        ..Default::default()
    };
    let command = CardContextCommand::SaveCardWithContext {
        card_id: card_id.clone(),
        parents: vec![original.id.clone()],
        interpretation: interpretation.clone(),
        at: 20,
        capture_context: context.clone(),
    };
    let encoded = serde_json::to_vec(&command).unwrap();
    let old_revision = state.revision;
    let result = NotebookCommand::parse(&encoded)
        .unwrap()
        .apply(&mut state)
        .unwrap();
    let version = state.cards[&card_id].versions.last().unwrap().clone();
    assert_eq!(state.revision, old_revision + 1);
    assert_eq!(result["version_id"], version.id);
    assert_eq!(result["conflict"], false);
    assert_eq!(version.reader, state.local_reader);
    assert_eq!(version.at, 20);
    assert_eq!(version.pass_id, notebook.pass_id);
    assert_eq!(version.positions, context.positions);
    assert_eq!(version.position_observations, context.position_observations);
    assert_eq!(version.interpretation, interpretation);
    assert_eq!(state.cards[&card_id].versions[0], original);
    assert!(!state.positions.contains_key("capture-before-commit"));
    assert_eq!(
        (state.schema.as_str(), state.ontology_version.as_str()),
        (SCHEMA, ONTOLOGY)
    );

    CardContextCommand::SaveCardWithContext {
        card_id: card_id.clone(),
        parents: vec![version.id],
        interpretation: CharacterInterpretation::default(),
        at: 31,
        capture_context: CapturedReadingContext {
            pass_id: notebook.pass_id,
            positions: vec![],
            position_observations: vec![],
        },
    }
    .apply(&mut state)
    .unwrap();
    let empty = state.cards[&card_id].versions.last().unwrap();
    assert!(empty.positions.is_empty() && empty.position_observations.is_empty());

    // Old source-level calls still compile and append the latest committed observations.
    let empty_parent = empty.id.clone();
    apply(
        &mut state,
        Command::SaveCard {
            card_id: card_id.clone(),
            parents: vec![empty_parent],
            interpretation: CharacterInterpretation::default(),
            at: 32,
            positions: vec![selected.clone()],
        },
    )
    .unwrap();
    let legacy = state.cards[&card_id].versions.last().unwrap();
    assert_eq!(legacy.positions, [selected, later.location.clone()]);
    assert_eq!(legacy.position_observations, [later]);
}

#[test]
fn invalid_or_misshaped_card_captures_fail_without_mutating_state() {
    let mut state = desk();
    let notebook = state.notebooks.values().next().unwrap().clone();
    let other_book = apply(
        &mut state,
        Command::AddBook {
            title: "Another fixture".into(),
            author: "Author".into(),
            at: 10,
        },
    )
    .unwrap();
    let other_edition = state.books[other_book["book_id"].as_str().unwrap()].edition_ids[0].clone();
    let card = apply(
        &mut state,
        Command::AddCharacter {
            notebook_id: notebook.id.clone(),
            block_id: notebook.blocks[0].id.clone(),
            capture_context: None,
            note_creation: None,
            name: "Mara".into(),
            at: 11,
        },
    )
    .unwrap()["card_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let parent = state.cards[&card].versions[0].id.clone();
    let valid = serde_json::json!({
        "op": "save_card_with_context",
        "card_id": card,
        "parents": [parent],
        "interpretation": CharacterInterpretation::default(),
        "at": 12,
        "capture_context": {"pass_id": notebook.pass_id, "positions": [], "position_observations": []},
    });
    let before = state.clone();

    let mut wrong_pass = valid.clone();
    wrong_pass["capture_context"]["pass_id"] = "other-pass".into();
    let mut wrong_location = valid.clone();
    wrong_location["capture_context"]["positions"] = serde_json::json!([{
        "kind":"page", "edition_id":other_edition, "page":1,
    }]);
    let mut wrong_observation = valid.clone();
    wrong_observation["capture_context"]["position_observations"] = serde_json::json!([{
        "id":"wrong-observation","pass_id":"other-pass","location":{
            "kind":"audio","provider":"audible","source_id":"test","milliseconds":20,"chapter":null
        }, "observed_at":11,"received_at":11,"alignment":{"status":"unmatched"}
    }]);
    let mut wrong_alignment = wrong_observation.clone();
    wrong_alignment["capture_context"]["position_observations"][0]["pass_id"] =
        notebook.pass_id.into();
    wrong_alignment["capture_context"]["position_observations"][0]["alignment"] = serde_json::json!({"status":"candidate","edition_id":other_edition,"reason":"Another book"});
    for invalid in [
        wrong_pass,
        wrong_location,
        wrong_observation,
        wrong_alignment,
    ] {
        let command = NotebookCommand::parse(invalid.to_string().as_bytes()).unwrap();
        assert!(command.apply(&mut state).is_err());
        assert_eq!(state, before);
    }

    let mut unknown = valid.clone();
    unknown["reader"] = "spoofed".into();
    let mut legacy_with_new_shape = valid.clone();
    legacy_with_new_shape["op"] = "save_card".into();
    legacy_with_new_shape["positions"] = serde_json::json!([]);
    let duplicate = format!("{{\"at\":99,{}", &valid.to_string()[1..]);
    for raw in [
        unknown.to_string(),
        legacy_with_new_shape.to_string(),
        duplicate,
    ] {
        assert!(NotebookCommand::parse(raw.as_bytes()).is_err());
    }
    assert_eq!(state, before);
}
