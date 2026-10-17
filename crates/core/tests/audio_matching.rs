// SPDX-License-Identifier: MIT OR Apache-2.0
use booknook_core::{Command, apply, audio::*, model::*};

fn book(
    state: &mut State,
    title: &str,
    author: &str,
    edition: &str,
    ids: Vec<SourceRef>,
    language: Option<&str>,
) -> (String, String) {
    let result = apply(
        state,
        Command::AddBook {
            title: title.into(),
            author: author.into(),
            at: 100,
        },
    )
    .unwrap();
    let id = result["book_id"].as_str().unwrap().to_owned();
    for previous in &state.books[&id].edition_ids {
        state.editions.remove(previous);
    }
    state.books.get_mut(&id).unwrap().edition_ids = vec![edition.into()];
    state.editions.insert(
        edition.into(),
        Edition {
            id: edition.into(),
            book_id: id.clone(),
            identifiers: ids,
            language: language.map(str::to_owned),
            assets: vec![],
        },
    );
    let pass = state
        .passes
        .values()
        .find(|p| p.book_id == id)
        .unwrap()
        .id
        .clone();
    (id, pass)
}
fn observation(metadata: Option<AudioMetadata>) -> AudioObservation {
    AudioObservation {
        asin: "B012345678".into(),
        position_milliseconds: 12_345,
        source_updated_at: 100,
        fetched_at: 200,
        duration_milliseconds: Some(50_000),
        edition_identifier: Some("opaque-legacy-id".into()),
        alignment: "unmatched".into(),
        metadata,
    }
}
#[test]
fn identifiers_are_namespaced_and_guesses_remain_explicit_candidates() {
    let mut state = State::default();
    book(
        &mut state,
        "Different title",
        "Someone",
        "identifier",
        vec![SourceRef {
            provider: "kindle".into(),
            id: "B098765432".into(),
        }],
        Some("fr"),
    );
    book(
        &mut state,
        "Café — Solitude",
        "Élodie",
        "metadata",
        vec![],
        Some("en-CA"),
    );
    book(
        &mut state,
        "Cafe\u{301} Solitude",
        "Different author",
        "title-only",
        vec![],
        None,
    );
    book(
        &mut state,
        "Unrelated",
        "Élodie",
        "author-only",
        vec![SourceRef {
            provider: "audible".into(),
            id: "B098765432".into(),
        }],
        None,
    );
    let metadata = AudioMetadata {
        title: Some("CAFE\u{301} solitude".into()),
        authors: vec!["ÉLODIE".into()],
        ebook_identifiers: vec![SourceRef {
            provider: "kindle".into(),
            id: "B098765432".into(),
        }],
        language: Some("en-US".into()),
    };
    let candidates = edition_candidates(&state, Some(&metadata)).unwrap();
    assert_eq!(
        candidates
            .iter()
            .map(|c| (&*c.edition_id, c.confidence))
            .collect::<Vec<_>>(),
        vec![
            ("identifier", MatchConfidence::Identifier),
            ("metadata", MatchConfidence::TitleAndAuthor),
            ("title-only", MatchConfidence::TitleOnly)
        ]
    );
    assert!(
        candidates[0]
            .warnings
            .iter()
            .any(|w| w.contains("Language differs"))
    );
    assert!(
        candidates[0]
            .warnings
            .iter()
            .any(|w| w.contains("titles differ"))
    );
    assert!(
        !candidates[1]
            .warnings
            .iter()
            .any(|w| w.contains("Language differs"))
    );
    assert!(edition_candidates(&state, None).unwrap().is_empty());
    assert!(
        edition_candidates(&state, Some(&AudioMetadata::default()))
            .unwrap()
            .is_empty()
    );
}
#[test]
fn confirmation_rechecks_book_and_preserves_audio_time_and_source_metadata() {
    let mut state = State::default();
    let (_, pass) = book(&mut state, "A Quiet Book", "Writer", "chosen", vec![], None);
    let (_, other_pass) = book(&mut state, "Another book", "Writer", "other", vec![], None);
    let source = observation(Some(AudioMetadata {
        title: Some("a quiet book".into()),
        authors: vec!["Writer".into()],
        ..Default::default()
    }));
    let unmatched = reviewed_observation(&state, &source, &pass, None, 2000).unwrap();
    assert_eq!(unmatched.alignment, Alignment::Unmatched);
    assert!(source.stale(2000));
    let confirmed = reviewed_observation(&state, &source, &pass, Some("chosen"), 2000).unwrap();
    assert_eq!(confirmed.location, unmatched.location);
    assert_eq!(confirmed.observed_at, 100);
    assert_eq!(confirmed.received_at, 200);
    assert_eq!(confirmed.audio_metadata, source.metadata);
    assert!(
        matches!(&confirmed.alignment, Alignment::Confirmed { evidence, .. } if evidence.contains("not mapped"))
    );
    assert!(reviewed_observation(&state, &source, &other_pass, Some("chosen"), 2000).is_err());
    assert!(reviewed_observation(&state, &source, &pass, Some("other"), 2000).is_err());
    apply(
        &mut state,
        Command::ObservePosition {
            observation: confirmed,
        },
    )
    .unwrap();
    let notebook = state
        .notebooks
        .values()
        .find(|n| n.pass_id == pass)
        .unwrap()
        .clone();
    apply(
        &mut state,
        Command::AddCharacter {
            capture_context: None,
            note_creation: None,
            notebook_id: notebook.id,
            block_id: notebook.blocks[0].id.clone(),
            name: "Character".into(),
            at: 201,
        },
    )
    .unwrap();
    let snapshot = &state.cards.values().next().unwrap().versions[0].position_observations[0];
    assert_eq!(snapshot.audio_metadata, source.metadata);
    let encoded = serde_json::to_value(&state).unwrap();
    let restored: State = serde_json::from_value(encoded.clone()).unwrap();
    booknook_core::validate(&restored).unwrap();
    assert_eq!(encoded, serde_json::to_value(restored).unwrap());
}
#[test]
fn old_observations_and_archive_positions_keep_their_serialized_shape() {
    let old = serde_json::json!({"asin":"B012345678","position_milliseconds":12345,"source_updated_at":100,"fetched_at":200,"duration_milliseconds":50000,"edition_identifier":"opaque-legacy-id","alignment":"unmatched"});
    let parsed: AudioObservation = serde_json::from_value(old.clone()).unwrap();
    assert!(parsed.metadata.is_none());
    assert_eq!(serde_json::to_value(&parsed).unwrap(), old);
    let old_position = serde_json::json!({"id":"p","pass_id":"pass","location":{"kind":"audio","provider":"audible","source_id":"B012345678","milliseconds":12345,"chapter":null},"observed_at":100,"received_at":200,"alignment":{"status":"unmatched"}});
    let parsed: ProgressObservation = serde_json::from_value(old_position.clone()).unwrap();
    assert!(parsed.audio_metadata.is_none());
    assert_eq!(serde_json::to_value(parsed).unwrap(), old_position);
}
#[test]
fn unqualified_metadata_cannot_force_a_match_or_confirm_an_audio_offset() {
    let mut state = State::default();
    let (_, pass) = book(&mut state, "A Book", "Writer", "chosen", vec![], None);
    let mut source = observation(None);
    assert!(reviewed_observation(&state, &source, &pass, Some("chosen"), 200).is_err());
    source.metadata = Some(AudioMetadata {
        title: Some("A Book".into()),
        language: Some("en\nmalformed".into()),
        ..Default::default()
    });
    assert!(
        source
            .validate(std::slice::from_ref(&source.asin), 200)
            .is_err()
    );
    source.metadata = None;
    source.alignment = "confirmed".into();
    assert!(
        source
            .validate(std::slice::from_ref(&source.asin), 200)
            .is_err()
    );
}
