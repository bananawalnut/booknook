// SPDX-License-Identifier: MIT OR Apache-2.0
use booknook_core::{Command, apply, model::*, reconcile};
fn base() -> State {
    let mut state = State::default();
    apply(
        &mut state,
        Command::AddBook {
            title: "Shared book".into(),
            author: "Writer".into(),
            at: 1,
        },
    )
    .unwrap();
    let notebook = state.notebooks.values().next().unwrap().clone();
    apply(
        &mut state,
        Command::EditBlock {
            capture_context: None,
            note_creation: None,
            notebook_id: notebook.id.clone(),
            block_id: notebook.blocks[0].id.clone(),
            base_update: notebook.blocks[0].update.clone(),
            text: "An impression".into(),
            at: 2,
        },
    )
    .unwrap();
    apply(
        &mut state,
        Command::AddCharacter {
            capture_context: None,
            note_creation: None,
            notebook_id: notebook.id,
            block_id: notebook.blocks[0].id.clone(),
            name: "Mara".into(),
            at: 3,
        },
    )
    .unwrap();
    state
}
fn edit(state: &mut State, text: &str, at: u64) {
    let n = state.notebooks.values().next().unwrap().clone();
    apply(
        state,
        Command::EditBlock {
            capture_context: None,
            note_creation: None,
            notebook_id: n.id,
            block_id: n.blocks[0].id.clone(),
            base_update: n.blocks[0].update.clone(),
            text: text.into(),
            at,
        },
    )
    .unwrap();
}
fn card(state: &mut State, description: &str, at: u64) {
    let c = state.cards.values().next().unwrap().clone();
    apply(
        state,
        Command::SaveCard {
            card_id: c.id,
            parents: vec![c.versions[0].id.clone()],
            interpretation: CharacterInterpretation {
                description: description.into(),
                ..Default::default()
            },
            positions: vec![],
            at,
        },
    )
    .unwrap();
}
#[test]
fn divergent_text_and_character_interpretations_survive_merge_export_and_reimport() {
    let base = base();
    let mut left = base.clone();
    let mut right = base.clone();
    edit(&mut left, "An impression from the beginning", 4);
    edit(&mut right, "My impression", 5);
    card(&mut left, "Trustworthy in chapter 3", 6);
    card(&mut right, "Unreliable by chapter 12", 7);
    let merged = reconcile::reconcile(Some(&base), &left, &right).unwrap();
    assert!(merged.conflicts.is_empty());
    let reverse = reconcile::reconcile(Some(&base), &right, &left).unwrap();
    let text = &merged.state.notebooks.values().next().unwrap().blocks[0].text;
    assert_eq!(
        text,
        &reverse.state.notebooks.values().next().unwrap().blocks[0].text
    );
    assert!(text.contains("from the beginning"));
    assert!(text.contains("My"));
    let card = merged.state.cards.values().next().unwrap();
    assert_eq!(card.heads().len(), 2);
    assert!(
        card.versions
            .iter()
            .any(|v| v.interpretation.description == "Trustworthy in chapter 3")
    );
    assert!(
        card.versions
            .iter()
            .any(|v| v.interpretation.description == "Unreliable by chapter 12")
    );
    let encoded = serde_json::to_vec(&merged.state).unwrap();
    let restored: State = booknook_core::strict_parse(&encoded).unwrap();
    booknook_core::validate(&restored).unwrap();
    assert_eq!(restored, merged.state);
}
#[test]
fn conflicting_personal_reviews_require_resolution_and_new_edits_are_not_overwritten() {
    let base = base();
    let mut left = base.clone();
    let mut right = base.clone();
    let id = base.books.keys().next().unwrap().clone();
    left.books.get_mut(&id).unwrap().review = "Left interpretation".into();
    right.books.get_mut(&id).unwrap().review = "Right interpretation".into();
    let merged = reconcile::reconcile(Some(&base), &left, &right).unwrap();
    let conflict = merged
        .conflicts
        .iter()
        .find(|c| c.field == "review")
        .unwrap();
    assert_eq!(merged.state.books[&id].review, "Left interpretation");
    let resolved = reconcile::resolve(&merged.state, conflict, true).unwrap();
    assert_eq!(resolved.books[&id].review, "Right interpretation");
    let mut subsequently_edited = merged.state;
    subsequently_edited.books.get_mut(&id).unwrap().review = "A newer thought".into();
    assert!(reconcile::resolve(&subsequently_edited, conflict, true).is_err());
    assert_eq!(
        reconcile::resolve(&subsequently_edited, conflict, false)
            .unwrap()
            .books[&id]
            .review,
        "A newer thought"
    );
}
#[test]
fn missing_remote_records_do_not_delete_local_work_and_reused_history_ids_are_rejected() {
    let base = base();
    let merged = reconcile::reconcile(None, &base, &State::default()).unwrap();
    assert_eq!(merged.state.books, base.books);
    assert_eq!(merged.state.cards, base.cards);
    let mut forged = base.clone();
    forged.cards.values_mut().next().unwrap().versions[0]
        .interpretation
        .description = "Changed immutable snapshot".into();
    assert!(reconcile::reconcile(Some(&base), &base, &forged).is_err());
}
