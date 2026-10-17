// SPDX-License-Identifier: MIT OR Apache-2.0
use booknook_core::{Command, apply, model::*, reconcile, strict_parse, validate};
use serde_json::{Value, json};

fn setup() -> (State, String, String) {
    let mut state = State::default();
    apply(
        &mut state,
        Command::AddBook {
            title: "Ink test".into(),
            author: "Synthetic".into(),
            at: 1,
        },
    )
    .unwrap();
    let notebook = state.notebooks.values().next().unwrap();
    let ids = (notebook.id.clone(), notebook.blocks[0].id.clone());
    (state, ids.0, ids.1)
}
fn ink_command(n: &str, b: &str, id: &str, parents: &[&str], erased: bool) -> Value {
    json!({"op":"set_notebook_ink", "notebook_id":n, "block_id":b, "version_id":id,
        "parents":parents, "canvas_width":768, "canvas_height":960, "at":10,
        "native_drawing": if erased { "" } else { "c3ludGhldGljLW5hdGl2ZS1kYXRh" },
        "strokes": if erased { json!([]) } else { json!([{"color":"#123456", "width":2.5,
            "points":[{"x":10,"y":12,"pressure":0.5,"time":0},{"x":12,"y":14,"pressure":0.8,"time":0.1}]}]) }})
}
fn run(state: &mut State, value: Value) -> booknook_core::Result<Value> {
    apply(
        state,
        strict_parse(serde_json::to_string(&value).unwrap().as_bytes())?,
    )
}
fn ink<'a>(s: &'a State, n: &str) -> &'a NotebookInk {
    s.notebooks[n].blocks[0].ink.as_ref().unwrap()
}

#[test]
fn ink_erasure_transcriptions_and_typed_edits_preserve_originals_and_context() {
    let (mut state, n, b) = setup();
    let pass = state.notebooks[&n].pass_id.clone();
    run(&mut state, json!({"op":"observe_position", "observation":{
        "id":"observed", "provider_connection_id":"audible.account.one", "pass_id":pass,
        "location":{"kind":"audio","provider":"audible","source_id":"book-one","milliseconds":12000,"chapter":"1"},
        "observed_at":4,"received_at":5,"alignment":{"status":"unmatched"}}})).unwrap();
    let before_text = state.notebooks[&n].blocks[0].text.clone();
    let first = run(&mut state, ink_command(&n, &b, "first", &[], false)).unwrap();
    assert_eq!(first["heads"], json!(["first"]));
    assert_eq!(
        (state.schema.as_str(), state.ontology_version.as_str()),
        (INK_ARCHIVE_SCHEMA, INK_ONTOLOGY)
    );
    let original = ink(&state, &n).versions[0].clone();
    assert_eq!(
        original.position_observations[0]
            .provider_connection_id
            .as_deref(),
        Some("audible.account.one")
    );
    assert_eq!(
        original.positions,
        vec![original.position_observations[0].location.clone()]
    );
    run(&mut state, ink_command(&n, &b, "erased", &["first"], true)).unwrap();
    for (id, text, method) in [
        ("t-one", "A first interpretation 🌿", "on_device"),
        ("t-two", "A corrected interpretation", "manual"),
    ] {
        run(
            &mut state,
            json!({"op":"save_notebook_transcription","notebook_id":n,"block_id":b,
            "transcription_id":id,"ink_version_id":"first","text":text,"method":method,"at":20}),
        )
        .unwrap();
    }
    assert_eq!(ink(&state, &n).versions[0], original);
    assert!(ink(&state, &n).versions[1].strokes.is_empty());
    assert_eq!(ink(&state, &n).transcriptions.len(), 2);
    assert_eq!(state.notebooks[&n].blocks[0].text, before_text);
    let all_ink = ink(&state, &n).clone();
    let base = state.notebooks[&n].blocks[0].update.clone();
    run(&mut state, json!({"op":"edit_block","notebook_id":n,"block_id":b,"text":"Separate prose café","base_update":base,"at":21})).unwrap();
    assert_eq!(ink(&state, &n), &all_ink);
    let exported = serde_json::to_vec(&state).unwrap();
    let reimported: State = strict_parse(&exported).unwrap();
    validate(&reimported).unwrap();
    assert_eq!(reimported, state);
}

#[test]
fn concurrent_branches_union_without_erasing_history_or_legacy_work() {
    let (legacy, n, b) = setup();
    let mut base = legacy.clone();
    run(&mut base, ink_command(&n, &b, "root", &[], false)).unwrap();
    let mut left = base.clone();
    let mut right = base.clone();
    run(&mut left, ink_command(&n, &b, "left", &["root"], true)).unwrap();
    run(&mut right, ink_command(&n, &b, "right", &["root"], false)).unwrap();
    let merged = reconcile::reconcile(Some(&base), &left, &right)
        .unwrap()
        .state;
    let reverse = reconcile::reconcile(Some(&base), &right, &left)
        .unwrap()
        .state;
    assert_eq!(ink(&merged, &n), ink(&reverse, &n));
    assert_eq!(
        ink(&merged, &n)
            .heads()
            .iter()
            .map(|v| v.id.as_str())
            .collect::<Vec<_>>(),
        ["left", "right"]
    );
    assert_eq!(ink(&merged, &n).versions.len(), 3);
    for result in [
        reconcile::reconcile(None, &legacy, &merged),
        reconcile::reconcile(None, &merged, &legacy),
    ] {
        let state = result.unwrap().state;
        assert_eq!(ink(&state, &n), ink(&merged, &n));
        assert_eq!(state.schema, INK_ARCHIVE_SCHEMA);
    }
    let mut reused = left.clone();
    reused.notebooks.get_mut(&n).unwrap().blocks[0]
        .ink
        .as_mut()
        .unwrap()
        .versions[1]
        .canvas_width = 800.0;
    assert!(reconcile::reconcile(None, &left, &reused).is_err());
}

#[test]
fn exact_retry_is_noop_but_reused_ids_and_invalid_inputs_are_atomic() {
    let (mut state, n, b) = setup();
    let command = ink_command(&n, &b, "retry", &[], false);
    run(&mut state, command.clone()).unwrap();
    let committed = state.clone();
    assert_eq!(run(&mut state, command.clone()).unwrap()["replayed"], true);
    assert_eq!(state, committed);
    let transcript = json!({"op":"save_notebook_transcription","notebook_id":n,"block_id":b,
        "transcription_id":"transcript","ink_version_id":"retry","text":"Unchanged handwriting","method":"manual","at":20});
    run(&mut state, transcript.clone()).unwrap();
    let committed = state.clone();
    assert_eq!(
        run(&mut state, transcript.clone()).unwrap()["replayed"],
        true
    );
    assert_eq!(state, committed);
    let mut bad = command.clone();
    bad["canvas_width"] = json!(800);
    let mut bad_transcript = transcript.clone();
    bad_transcript["text"] = json!("Different");
    for bad in [bad, bad_transcript] {
        assert!(run(&mut state, bad).is_err());
        assert_eq!(state, committed);
    }
    let mut variants = vec![];
    for (key, value) in [
        ("parents", json!(["missing"])),
        ("parents", json!(["retry", "retry"])),
        ("canvas_width", json!(0)),
        ("native_drawing", json!("not base64!")),
    ] {
        let mut bad = command.clone();
        bad["version_id"] = json!("new");
        bad[key] = value;
        variants.push(bad);
    }
    let mut bad = command.clone();
    bad["version_id"] = json!("new");
    bad["strokes"][0]["points"][0]["pressure"] = json!(2);
    variants.push(bad);
    let mut bad = command.clone();
    bad["version_id"] = json!("new");
    bad["strokes"][0]["color"] = json!("CanvasText");
    variants.push(bad);
    let mut bad = transcript.clone();
    bad["transcription_id"] = json!("new");
    bad["ink_version_id"] = json!("missing");
    variants.push(bad);
    for bad in variants {
        assert!(run(&mut state, bad).is_err());
        assert_eq!(state, committed);
    }
}

#[test]
fn legacy_json_stays_legacy_and_ink_cannot_be_disguised_as_v1() {
    let (mut state, n, b) = setup();
    let original = serde_json::to_value(&state).unwrap();
    assert!(original["notebooks"][&n]["blocks"][0].get("ink").is_none());
    assert_eq!(
        strict_parse::<State>(&serde_json::to_vec(&state).unwrap()).unwrap(),
        state
    );
    run(&mut state, ink_command(&n, &b, "new", &[], false)).unwrap();
    state.schema = SCHEMA.into();
    state.ontology_version = ONTOLOGY.into();
    assert!(
        validate(&state)
            .unwrap_err()
            .to_string()
            .contains("archive.v2")
    );
}

#[test]
fn published_ink_conformance_vectors_use_strict_portable_contracts() {
    let bundle: Value = serde_json::from_str(include_str!("fixtures/notebook-ink.json")).unwrap();
    assert_eq!(bundle["ontology_version"], INK_ONTOLOGY);
    for case in bundle["cases"].as_array().unwrap() {
        let raw = serde_json::to_vec(&case["input"]).unwrap();
        let result: booknook_core::Result<()> = (|| match case["contract"].as_str().unwrap() {
            "state" => validate(&strict_parse::<State>(&raw)?),
            "command" => {
                let mut state: State = serde_json::from_value(case["state"].clone())?;
                let original = state.clone();
                let result = apply(&mut state, strict_parse(&raw)?);
                if result.is_err() {
                    assert_eq!(state, original);
                }
                result.map(|_| ())
            }
            _ => unreachable!(),
        })();
        assert_eq!(
            result.is_ok(),
            case["valid"].as_bool().unwrap(),
            "{}: {result:?}",
            case["id"]
        );
    }
}
