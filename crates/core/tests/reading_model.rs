use booknook_core::{Command, apply, model::*, validate};

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
