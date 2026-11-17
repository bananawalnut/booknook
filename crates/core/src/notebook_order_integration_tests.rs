//! Production command, validation, portable archive and reconciliation coverage.
use super::*;
use crate::{
    self as core, model::*, notebook_command::NotebookCommand, portable_archive, reconcile,
};
use serde_json::{Value, json};

fn run(state: &mut State, command: Value) -> core::Result<Value> {
    NotebookCommand::parse(&serde_json::to_vec(&command)?)?.apply(state)
}
fn desk() -> (State, String) {
    let mut state = State::default();
    run(
        &mut state,
        json!({"op":"add_book","title":"Notebook fixture","author":"Synthetic","at":1}),
    )
    .unwrap();
    let notebook = state.notebooks.keys().next().unwrap().clone();
    (state, notebook)
}
fn ids(state: &State, notebook: &str) -> Vec<String> {
    state.notebooks[notebook]
        .blocks
        .iter()
        .map(|block| block.id.clone())
        .collect()
}
fn old_add(state: &mut State, notebook: &str, face: &str) -> String {
    run(
        state,
        json!({"op":"add_block","notebook_id":notebook,"at":3,"face":face}),
    )
    .unwrap()["block_id"]
        .as_str()
        .unwrap()
        .into()
}
fn order_request(notebook: &str, block: &str, visible: &[String]) -> Value {
    json!({
        "op":"append_ordered_note","notebook_id":notebook,"block_id":block,
        "visible_block_ids":visible,"at":5,"face":"text"
    })
}
fn round_trip(state: &State) -> State {
    let archive = serde_json::to_vec(&json!({
        "schema":portable_archive::SCHEMA,"state":state,"assets":{}
    }))
    .unwrap();
    let parsed = portable_archive::parse(&archive).unwrap();
    assert_eq!(parsed.state, *state);
    assert_eq!(
        portable_archive::roundtrip(&archive).unwrap(),
        portable_archive::roundtrip(portable_archive::roundtrip(&archive).unwrap().as_bytes())
            .unwrap()
    );
    parsed.state
}
fn set_ink(state: &mut State, notebook: &str, block: &str, id: &str) {
    run(state, json!({
        "op":"set_notebook_ink","notebook_id":notebook,"block_id":block,
        "version_id":id,"parents":[],"canvas_width":768,"canvas_height":960,
        "strokes":[{"color":"#123456","width":2,"points":[{"x":0,"y":0,"pressure":0.7,"time":0}]}],
        "at":6
    })).unwrap();
}

#[test]
fn read_export_and_unchanged_merge_keep_legacy_versions_and_dominant_sequences() {
    let (mut state, notebook) = desk();
    let first = ids(&state, &notebook)[0].clone();
    for expected in [SCHEMA, INK_ARCHIVE_SCHEMA, NOTE_ARCHIVE_SCHEMA] {
        match expected {
            INK_ARCHIVE_SCHEMA => set_ink(&mut state, &notebook, &first, "legacy-ink"),
            NOTE_ARCHIVE_SCHEMA => {
                old_add(&mut state, &notebook, "text");
            }
            _ => {}
        }
        assert_eq!(state.schema, expected);
        assert!(round_trip(&state).notebooks[&notebook].order.is_none());
        let identical = reconcile::reconcile(None, &state, &state).unwrap().state;
        assert_eq!(identical.schema, expected);
        assert!(identical.notebooks[&notebook].order.is_none());
    }
    let mut longer = state.clone();
    let appended = old_add(&mut longer, &notebook, "whiteboard");
    let blocks = &mut longer.notebooks.get_mut(&notebook).unwrap().blocks;
    blocks.rotate_right(1);
    assert_eq!(blocks[0].id, appended);
    for (local, incoming) in [(&state, &longer), (&longer, &state)] {
        let merged = reconcile::reconcile(Some(&state), local, incoming)
            .unwrap()
            .state;
        assert_eq!(merged.schema, NOTE_ARCHIVE_SCHEMA);
        assert!(merged.notebooks[&notebook].order.is_none());
        assert_eq!(ids(&merged, &notebook), ids(&longer, &notebook));
    }
}

#[test]
fn ordered_append_bind_full_captured_request_and_old_edits_cannot_downgrade() {
    let (mut state, notebook) = desk();
    let captured = ids(&state, &notebook);
    let request = order_request(&notebook, "ordered-note", &captured);
    let late = old_add(&mut state, &notebook, "text");
    run(&mut state, request.clone()).unwrap();
    assert_eq!(state.schema, ORDER_ARCHIVE_SCHEMA);
    let order = serde_json::to_value(&state.notebooks[&notebook].order).unwrap();
    assert_eq!(order["inserts"]["ordered-note"]["after"], captured[0]);
    assert!(order["frontiers"].as_object().unwrap().is_empty());
    assert!(is_digest(
        order["inserts"]["ordered-note"]["request"]
            .as_str()
            .unwrap()
    ));
    let mut bad = order;
    bad["inserts"]["ordered-note"]["request"] = json!("not-a-digest");
    let bad_order: NotebookOrder = serde_json::from_value(bad).unwrap();
    assert_eq!(bad_order.view(), Err(OrderingError::InvalidAppend));
    assert!(ids(&state, &notebook).contains(&late));

    let base_update = state.notebooks[&notebook]
        .blocks
        .iter()
        .find(|block| block.id == "ordered-note")
        .unwrap()
        .update
        .clone();
    let pass = state.notebooks[&notebook].pass_id.clone();
    run(
        &mut state,
        json!({
            "op":"edit_block","notebook_id":notebook,"block_id":"ordered-note",
            "base_update":base_update,"text":"Exact café 日本語","at":7,
            "capture_context":{"pass_id":pass,"positions":[],"position_observations":[]}
        }),
    )
    .unwrap();
    set_ink(&mut state, &notebook, "ordered-note", "new-ink");
    let legacy_write = old_add(&mut state, &notebook, "text");
    let projection = ids(&state, &notebook);
    assert_eq!(projection.last(), Some(&legacy_write));
    let after = state.clone();
    assert_eq!(run(&mut state, request.clone()).unwrap()["replayed"], true);
    assert_eq!(state, after);
    let mut changed = request;
    changed["at"] = json!(55);
    assert!(run(&mut state, changed).is_err());
    assert_eq!(state, after);
    assert_eq!(state.schema, ORDER_ARCHIVE_SCHEMA);
    assert_eq!(round_trip(&state), state);

    let mut wrong = state.clone();
    wrong.ontology_version = NOTE_ONTOLOGY.into();
    assert!(core::validate(&wrong).is_err());
    wrong.schema = NOTE_ARCHIVE_SCHEMA.into();
    assert!(core::validate(&wrong).is_err());
    let blocks = &mut state.notebooks.get_mut(&notebook).unwrap().blocks;
    blocks.swap(1, 2);
    assert!(core::validate(&state).is_err());
}

#[test]
fn two_independent_legacy_appends_promote_and_converge_across_reciprocal_retry() {
    let (base, notebook) = desk();
    let mut left = base.clone();
    let mut right = base.clone();
    let a = old_add(&mut left, &notebook, "text");
    let b = old_add(&mut left, &notebook, "whiteboard");
    let c = old_add(&mut right, &notebook, "whiteboard");
    let d = old_add(&mut right, &notebook, "text");
    assert_eq!(left.schema, NOTE_ARCHIVE_SCHEMA);
    assert_eq!(right.schema, NOTE_ARCHIVE_SCHEMA);
    let lr = reconcile::reconcile(Some(&base), &left, &right)
        .unwrap()
        .state;
    let rl = reconcile::reconcile(Some(&base), &right, &left)
        .unwrap()
        .state;
    assert_eq!(lr.schema, ORDER_ARCHIVE_SCHEMA);
    let order = ids(&lr, &notebook);
    assert_eq!(order, ids(&rl, &notebook));
    assert_eq!(lr.notebooks[&notebook], rl.notebooks[&notebook]);
    assert!(order.iter().position(|id| id == &a) < order.iter().position(|id| id == &b));
    assert!(order.iter().position(|id| id == &c) < order.iter().position(|id| id == &d));
    for (local, incoming) in [(&lr, &right), (&left, &rl), (&lr, &rl), (&rl, &lr)] {
        let repeated = reconcile::reconcile(Some(&base), local, incoming)
            .unwrap()
            .state;
        assert_eq!(repeated.notebooks[&notebook], lr.notebooks[&notebook]);
    }
}

#[test]
fn old_and_new_replicas_merge_without_replaying_a_projection_as_authored() {
    let (base, notebook) = desk();
    let visible = ids(&base, &notebook);
    let mut modern = base.clone();
    let mut legacy = base.clone();
    run(&mut modern, order_request(&notebook, "modern", &visible)).unwrap();
    let old = old_add(&mut legacy, &notebook, "whiteboard");
    let lr = reconcile::reconcile(Some(&base), &modern, &legacy)
        .unwrap()
        .state;
    let rl = reconcile::reconcile(Some(&base), &legacy, &modern)
        .unwrap()
        .state;
    assert_eq!(lr.notebooks[&notebook], rl.notebooks[&notebook]);
    let merged = ids(&lr, &notebook);
    assert!(merged.contains(&old) && merged.contains(&"modern".to_string()));
    assert_eq!(round_trip(&lr), lr);
}

#[test]
fn reorder_only_legacy_conflict_retains_both_orders_and_review_is_explicit_atomic() {
    let (mut left, notebook) = desk();
    let text = old_add(&mut left, &notebook, "text");
    let board = old_add(&mut left, &notebook, "whiteboard");
    let mut right = left.clone();
    right
        .notebooks
        .get_mut(&notebook)
        .unwrap()
        .blocks
        .swap(1, 2);
    let mut merged = reconcile::reconcile(None, &left, &right).unwrap().state;
    let opposite = reconcile::reconcile(None, &right, &left).unwrap().state;
    assert_eq!(merged.notebooks[&notebook], opposite.notebooks[&notebook]);
    let order = merged.notebooks[&notebook].order.as_ref().unwrap();
    let issue = order.view().unwrap().issue.unwrap();
    assert_eq!(
        issue.legacy_orders,
        BTreeSet::from([ids(&left, &notebook), ids(&right, &notebook)])
    );
    assert!(ids(&merged, &notebook).contains(&text) && ids(&merged, &notebook).contains(&board));
    assert_eq!(round_trip(&merged), merged);
    let untouched = merged.clone();
    let wrong = json!({
        "op":"review_notebook_order","notebook_id":notebook,"review_id":"choice",
        "evidence":"stale","heads":[],"selected":ids(&right,&notebook)
    });
    assert!(run(&mut merged, wrong).is_err());
    assert_eq!(merged, untouched);
    let correct = json!({
        "op":"review_notebook_order","notebook_id":notebook,"review_id":"choice",
        "evidence":issue.evidence,"heads":issue.heads,"selected":ids(&right,&notebook)
    });
    run(&mut merged, correct.clone()).unwrap();
    let retained = merged.clone();
    assert_eq!(run(&mut merged, correct).unwrap()["replayed"], true);
    assert_eq!(merged, retained);
    assert_eq!(ids(&merged, &notebook), ids(&right, &notebook));
    assert!(
        merged.notebooks[&notebook]
            .order
            .as_ref()
            .unwrap()
            .view()
            .unwrap()
            .issue
            .is_none()
    );
    let repeated = reconcile::reconcile(None, &merged, &left).unwrap().state;
    assert_eq!(repeated.notebooks[&notebook], merged.notebooks[&notebook]);
}

#[test]
fn large_competing_reviews_can_settle_existing_choice_without_an_extra_full_order() {
    let ids: Vec<_> = (0..4095)
        .map(|index| format!("legacy-{index:05}"))
        .collect();
    let reverse: Vec<_> = ids.iter().cloned().rev().collect();
    let a = NotebookOrder::from_legacy(ids.clone()).unwrap();
    let b = NotebookOrder::from_legacy(reverse.clone()).unwrap();
    let conflict = a.join(&b).unwrap();
    let issue = conflict.view().unwrap().issue.unwrap();
    let one = conflict.review("one".into(), &issue, ids).unwrap();
    let two = conflict
        .review("two".into(), &issue, reverse.clone())
        .unwrap();
    let competing = one.join(&two).unwrap();
    let distinct = competing.reviewed_orders.len();
    let issue = competing.view().unwrap().issue.unwrap();
    let settled = competing
        .review("settlement".into(), &issue, reverse.clone())
        .unwrap();
    assert_eq!(settled.reviewed_orders.len(), distinct);
    assert_eq!(settled.view().unwrap().ids, reverse);
    assert_eq!(settled.join(&one).unwrap(), settled);
}

#[test]
fn historical_review_scope_and_duplicate_wire_evidence_are_validated() {
    let older = NotebookOrder::from_legacy(vec!["p".into(), "a".into(), "b".into()]).unwrap();
    let opposite = NotebookOrder::from_legacy(vec!["p".into(), "b".into(), "a".into()]).unwrap();
    let conflict = older.join(&opposite).unwrap();
    let previous = conflict.view().unwrap().issue.unwrap();
    let selection = vec!["p".into(), "a".into(), "b".into()];
    let resolved = conflict
        .review("review".into(), &previous, selection)
        .unwrap();
    let later = NotebookOrder::from_legacy(vec!["p".into(), "a".into(), "b".into(), "late".into()])
        .unwrap();
    let extended = resolved.join(&later).unwrap();
    assert_eq!(extended.review_scopes.len(), 1);
    let mut encoded = serde_json::to_value(&extended).unwrap();
    let fake = vec!["p".to_owned()];
    let fake_id = content_id(&fake).unwrap();
    let selected = extended.reviews["review"].selected.clone();
    encoded["reviewed_orders"]
        .as_object_mut()
        .unwrap()
        .remove(&selected);
    encoded["reviewed_orders"][&fake_id] = json!(fake);
    encoded["reviews"]["review"]["selected"] = json!(fake_id);
    let invalid: NotebookOrder = serde_json::from_value(encoded).unwrap();
    assert_eq!(invalid.view(), Err(OrderingError::InvalidReview));
    assert_eq!(invalid.join(&older), Err(OrderingError::InvalidReview));

    let mut duplicate = serde_json::to_value(&resolved).unwrap();
    let paths = duplicate["legacy"].as_array_mut().unwrap();
    paths.push(paths[0].clone());
    assert!(serde_json::from_value::<NotebookOrder>(duplicate).is_err());
    let mut duplicate_scope = serde_json::to_value(&resolved).unwrap();
    let paths = duplicate_scope["review_scopes"][&previous.evidence]
        .as_array_mut()
        .unwrap();
    paths.push(paths[0].clone());
    assert!(serde_json::from_value::<NotebookOrder>(duplicate_scope).is_err());
}

#[test]
fn over_budget_join_can_retry_an_unchanged_verified_packet_after_dominance() {
    fn build(indices: std::ops::Range<usize>) -> NotebookOrder {
        indices.fold(NotebookOrder::default(), |order, index| {
            order
                .join(
                    &NotebookOrder::from_legacy(vec!["base".into(), format!("note-{index:02}")])
                        .unwrap(),
                )
                .unwrap()
        })
    }
    let a = build(0..17);
    let b = build(17..34);
    let raw = serde_json::to_vec(&b).unwrap();
    assert_eq!(a.join(&b), Err(OrderingError::LegacyBudget));
    assert_eq!(serde_json::to_vec(&b).unwrap(), raw);
    let dominating = NotebookOrder::from_legacy(
        std::iter::once("base".to_owned())
            .chain((0..34).map(|index| format!("note-{index:02}")))
            .collect(),
    )
    .unwrap();
    let updated = a.join(&dominating).unwrap();
    let retry: NotebookOrder = serde_json::from_slice(&raw).unwrap();
    assert_eq!(updated.join(&retry).unwrap(), dominating);
    let oversized = (0..33)
        .map(|index| vec!["base".to_owned(), format!("owned-{index:02}")])
        .collect();
    let malformed = NotebookOrder {
        legacy: oversized,
        ..NotebookOrder::default()
    };
    assert_eq!(malformed.view(), Err(OrderingError::LegacyBudget));
}

#[test]
fn pending_review_choices_totalize_partial_legacy_orders_without_hiding_unresolved_history() {
    let (mut full, notebook) = desk();
    let root = ids(&full, &notebook)[0].clone();
    let a = old_add(&mut full, &notebook, "text");
    let b = old_add(&mut full, &notebook, "whiteboard");
    let c = old_add(&mut full, &notebook, "text");
    let earlier = |first: &str, second: &str| {
        let mut state = full.clone();
        let values = &mut state.notebooks.get_mut(&notebook).unwrap().blocks;
        let order = [&root[..], first, second];
        values.retain(|block| order.contains(&block.id.as_str()));
        values.sort_by_key(|block| order.iter().position(|id| *id == block.id).unwrap());
        state
    };
    let ab = earlier(&a, &b);
    let bc = earlier(&b, &c);
    let ca = earlier(&c, &a);
    let joined = reconcile::reconcile(None, &ab, &bc).unwrap().state;
    let joined = reconcile::reconcile(None, &joined, &ca).unwrap().state;
    let list = unresolved_reviews(&joined).unwrap();
    assert_eq!(list.len(), 1);
    let issue = &list[0];
    assert_eq!(issue.notebook_id, notebook);
    assert_eq!(issue.legacy_orders.len(), 3);
    assert!(issue.choices.len() >= 2);
    assert_eq!(issue.provisional_legacy_order, ids(&joined, &notebook));
    for selected in &issue.choices {
        assert_eq!(selected.len(), 4);
        let mut reviewing = joined.clone();
        let command = json!({
            "op":"review_notebook_order","notebook_id":notebook,"review_id":"review",
            "evidence":issue.evidence,"heads":issue.heads,"selected":selected
        });
        run(&mut reviewing, command).unwrap();
        assert_eq!(ids(&reviewing, &notebook), *selected);
        assert!(unresolved_reviews(&reviewing).unwrap().is_empty());
    }
    let encoded = serde_json::to_value(&list).unwrap();
    assert!(encoded[0]["evidence"].is_string());
    assert!(encoded[0]["choices"].is_array());
    assert!(encoded[0]["provisional_legacy_order"].is_array());
}
