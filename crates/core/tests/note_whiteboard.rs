// SPDX-License-Identifier: MIT OR Apache-2.0
use booknook_core::{apply, model::*, reconcile, strict_parse, validate};
use serde_json::{Value, json};
fn run(state: &mut State, value: Value) -> booknook_core::Result<Value> {
    apply(
        state,
        strict_parse(serde_json::to_string(&value)?.as_bytes())?,
    )
}
fn setup() -> (State, String, String) {
    let mut s = State::default();
    run(
        &mut s,
        json!({"op":"add_book","title":"Notes","author":"Synthetic","at":1}),
    )
    .unwrap();
    let n = s.notebooks.values().next().unwrap();
    let ids = (n.id.clone(), n.blocks[0].id.clone());
    (s, ids.0, ids.1)
}
fn context(s: &State, n: &str, position: &str) -> Value {
    json!({"pass_id":s.notebooks[n].pass_id,"positions":[{"kind":"provider","provider":"example.reader","source_id":"book","value":position}],"position_observations":[]})
}
fn edit(s: &State, n: &str, b: &str, text: &str, at: u64) -> Value {
    json!({"op":"edit_block","notebook_id":n,"block_id":b,"base_update":s.notebooks[n].blocks.iter().find(|x| x.id==b).unwrap().update,"text":text,"at":at})
}
fn ink(n: &str, b: &str, id: &str, parent: Vec<&str>) -> Value {
    json!({"op":"set_notebook_ink","notebook_id":n,"block_id":b,"version_id":id,"parents":parent,"canvas_width":32000,"canvas_height":40000,"canvas_space":{"kind":"whiteboard","origin_x":-10000,"origin_y":-20000},"native_drawing":"b3JpZ2luYWw=","strokes":[{"color":"#123456","width":2,"points":[{"x":-50,"y":30000,"pressure":0.7,"time":0}]}],"at":50})
}
#[test]
fn legacy_commands_stay_legacy_and_default_blank_has_no_invented_origin() {
    let (mut s, n, b) = setup();
    assert!(s.notebooks[&n].blocks[0].note.is_none());
    let command = edit(&s, &n, &b, "Legacy café", 2);
    run(&mut s, command).unwrap();
    assert_eq!(s.schema, SCHEMA);
    assert!(s.notebooks[&n].blocks[0].note.is_none());
    let mut modern = edit(&s, &n, &b, "Modern edit", 10);
    modern["capture_context"] = context(&s, &n, "later");
    run(&mut s, modern).unwrap();
    let origin = &s.notebooks[&n].blocks[0].note.as_ref().unwrap().created;
    assert_eq!(origin.source, NoteOriginSource::HistoryFallback);
    assert_eq!(origin.at, Some(2));
    assert!(origin.positions.is_empty());
}
#[test]
fn first_input_is_immutable_after_flips_positions_and_whitespace_coalescing() {
    let (mut s, n, b) = setup();
    run(
        &mut s,
        json!({"op":"set_note_face","notebook_id":n,"block_id":b,"face":"whiteboard","at":10}),
    )
    .unwrap();
    assert_eq!(
        s.notebooks[&n].blocks[0]
            .note
            .as_ref()
            .unwrap()
            .created
            .source,
        NoteOriginSource::Pending
    );
    let first = context(&s, &n, "chapter3");
    let mut command = edit(&s, &n, &b, "", 30);
    command["capture_context"] = context(&s, &n, "chapter12");
    command["note_creation"] = json!({"at":20,"capture_context":first});
    run(&mut s, command).unwrap();
    let origin = s.notebooks[&n].blocks[0]
        .note
        .as_ref()
        .unwrap()
        .created
        .clone();
    assert_eq!(origin.at, Some(20));
    assert_eq!(
        serde_json::to_value(&origin.positions).unwrap(),
        first["positions"]
    );
    let mut command = edit(&s, &n, &b, "Later 日本語", 40);
    command["capture_context"] = context(&s, &n, "chapter15");
    run(&mut s, command).unwrap();
    run(
        &mut s,
        json!({"op":"set_note_face","notebook_id":n,"block_id":b,"face":"text","at":41}),
    )
    .unwrap();
    assert_eq!(
        s.notebooks[&n].blocks[0].note.as_ref().unwrap().created,
        origin
    );
    assert_eq!(
        (s.schema.as_str(), s.ontology_version.as_str()),
        (NOTE_ARCHIVE_SCHEMA, NOTE_ONTOLOGY)
    );
}
#[test]
fn world_growth_and_export_preserve_original_ink_context_and_order() {
    let (mut s, n, b) = setup();
    let c = context(&s, &n, "captured");
    run(
        &mut s,
        json!({"op":"add_block","notebook_id":n,"at":20,"capture_context":c,"face":"text"}),
    )
    .unwrap();
    let order: Vec<_> = s.notebooks[&n]
        .blocks
        .iter()
        .map(|b| b.id.clone())
        .collect();
    let mut first = ink(&n, &b, "one", vec![]);
    first["capture_context"] = context(&s, &n, "drawing-start");
    first["note_creation"] = json!({"at":25,"capture_context":c});
    run(&mut s, first.clone()).unwrap();
    let old = s.notebooks[&n].blocks[0].ink.as_ref().unwrap().versions[0].clone();
    let origin = s.notebooks[&n].blocks[0]
        .note
        .as_ref()
        .unwrap()
        .created
        .clone();
    let mut grown = ink(&n, &b, "two", vec!["one"]);
    grown["canvas_width"] = json!(1_200_000);
    grown["canvas_space"]["origin_x"] = json!(-600000);
    grown["capture_context"] = context(&s, &n, "later");
    grown["at"] = json!(80);
    run(&mut s, grown).unwrap();
    assert_eq!(
        s.notebooks[&n].blocks[0].ink.as_ref().unwrap().versions[0],
        old
    );
    assert_eq!(
        s.notebooks[&n].blocks[0].note.as_ref().unwrap().created,
        origin
    );
    let prior = s.clone();
    assert_eq!(run(&mut s, first.clone()).unwrap()["replayed"], true);
    assert_eq!(s, prior);
    first["capture_context"] = context(&s, &n, "wrong-retry");
    assert!(run(&mut s, first).is_err());
    assert_eq!(s, prior);
    let exported = booknook_core::portable_archive::PortableArchive {
        schema: "booknook.portable-export.v1".into(),
        state: s.clone(),
        assets: Default::default(),
        social: None,
    };
    let raw = serde_json::to_string(&exported).unwrap();
    let restored = booknook_core::portable_archive::parse(raw.as_bytes()).unwrap();
    assert_eq!(restored.state, s);
    assert_eq!(
        s.notebooks[&n]
            .blocks
            .iter()
            .map(|b| b.id.clone())
            .collect::<Vec<_>>(),
        order
    );
}
#[test]
fn whiteboard_limits_and_schema_downgrade_are_atomic() {
    let (mut s, n, b) = setup();
    for change in [
        json!({"canvas_width":2_000_001}),
        json!({"canvas_space":{"kind":"whiteboard","origin_x":999999,"origin_y":0}}),
        json!({"canvas_space":null}),
    ] {
        let mut cmd = ink(&n, &b, "one", vec![]);
        for (k, v) in change.as_object().unwrap() {
            cmd[k] = v.clone();
        }
        let old = s.clone();
        assert!(run(&mut s, cmd).is_err());
        assert_eq!(s, old);
    }
    run(&mut s, ink(&n, &b, "one", vec![])).unwrap();
    s.schema = INK_ARCHIVE_SCHEMA.into();
    s.ontology_version = INK_ONTOLOGY.into();
    assert!(validate(&s).is_err());
}
#[test]
fn first_card_context_and_explicit_blank_add_are_recorded() {
    let (mut s, n, b) = setup();
    let c = context(&s, &n, "card-time");
    run(&mut s,json!({"op":"add_character","notebook_id":n,"block_id":b,"name":"Mara","at":12,"capture_context":c})).unwrap();
    let origin = &s.notebooks[&n].blocks[0].note.as_ref().unwrap().created;
    assert_eq!(origin.source, NoteOriginSource::Recorded);
    assert_eq!(origin.at, Some(12));
    let added = run(
        &mut s,
        json!({"op":"add_block","notebook_id":n,"at":14,"capture_context":c,"face":"whiteboard"}),
    )
    .unwrap();
    let last = s.notebooks[&n].blocks.last().unwrap();
    assert_eq!(last.id, added["block_id"]);
    assert_eq!(last.note.as_ref().unwrap().created.at, Some(14));
    assert!(last.ink.is_none());
}
#[test]
fn conflicting_creation_is_explicit_and_three_way_face_preserves_block_order() {
    let (mut base, n, b) = setup();
    let c = context(&base, &n, "first");
    let mut cmd = edit(&base, &n, &b, "First", 2);
    cmd["capture_context"] = c;
    run(&mut base, cmd).unwrap();
    let left = base.clone();
    let mut right = base.clone();
    run(
        &mut right,
        json!({"op":"set_note_face","notebook_id":n,"block_id":b,"face":"whiteboard","at":5}),
    )
    .unwrap();
    let merged = reconcile::reconcile(Some(&base), &left, &right)
        .unwrap()
        .state;
    assert_eq!(
        merged.notebooks[&n].blocks[0].note.as_ref().unwrap().face,
        NoteFace::Whiteboard
    );
    assert_eq!(merged.notebooks[&n].blocks[0].text, "First");
    right.notebooks.get_mut(&n).unwrap().blocks[0]
        .note
        .as_mut()
        .unwrap()
        .created
        .at = Some(3);
    assert!(
        reconcile::reconcile(Some(&base), &left, &right)
            .unwrap_err()
            .to_string()
            .contains("creation")
    );
    assert_eq!(left, base);
}
#[test]
fn captured_context_from_another_pass_is_rejected_before_any_change() {
    let (mut s, n, b) = setup();
    let mut cmd = edit(&s, &n, &b, "No", 10);
    cmd["capture_context"] = context(&s, &n, "position");
    cmd["capture_context"]["pass_id"] = json!("wrong");
    let old = s.clone();
    assert!(run(&mut s, cmd).is_err());
    assert_eq!(s, old);
}

#[test]
fn published_note_whiteboard_vectors_use_the_real_strict_contract() {
    let bundle: Value =
        serde_json::from_str(include_str!("fixtures/note-whiteboard.json")).unwrap();
    assert_eq!(bundle["ontology_version"], NOTE_ONTOLOGY);
    for case in bundle["cases"].as_array().unwrap() {
        let input = serde_json::to_vec(&case["input"]).unwrap();
        let result: booknook_core::Result<()> = (|| {
            if case["contract"] == "state" {
                let state: State = strict_parse(&input)?;
                validate(&state)?;
            } else {
                let mut state: State = serde_json::from_value(case["state"].clone())?;
                apply(&mut state, strict_parse(&input)?)?;
            }
            Ok(())
        })();
        assert_eq!(
            result.is_ok(),
            case["valid"].as_bool().unwrap(),
            "{}: {:?}",
            case["id"],
            result
        );
    }
}
