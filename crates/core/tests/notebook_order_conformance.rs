// SPDX-License-Identifier: MIT OR Apache-2.0
use booknook_core::{
    Result,
    model::{ORDER_ARCHIVE_SCHEMA, ORDER_ONTOLOGY, State},
    notebook_command::NotebookCommand,
    portable_archive, strict_parse, validate,
};
use serde_json::Value;

#[test]
fn published_notebook_order_vectors_match_the_actual_contracts() {
    let bundle: Value = serde_json::from_str(include_str!("fixtures/notebook-order.json")).unwrap();
    assert_eq!(bundle["schema"], "booknook.notebook-order-conformance.v1");
    assert_eq!(bundle["ontology_version"], ORDER_ONTOLOGY);

    let cases = bundle["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 15);
    assert_eq!(
        cases
            .iter()
            .filter(|case| case["valid"].as_bool() == Some(true))
            .count(),
        6
    );
    let mut seen = std::collections::BTreeSet::new();
    for case in cases {
        let id = case["id"].as_str().unwrap();
        assert!(seen.insert(id), "Duplicate fixture ID {id}");
        let raw = case["raw_json"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| serde_json::to_string(&case["input"]).unwrap());
        let result = check_case(case, &raw, cases);
        assert_eq!(
            result.is_ok(),
            case["valid"].as_bool().unwrap(),
            "{id}: {result:?}"
        );
    }
}

fn check_case(case: &Value, raw: &str, cases: &[Value]) -> Result<()> {
    let id = case["id"].as_str().unwrap();
    match case["contract"].as_str().unwrap() {
        "state" => {
            let state: State = strict_parse(raw.as_bytes())?;
            validate(&state)?;
            assert_eq!(serde_json::to_value(state)?, case["input"], "{id}");
        }
        "portable" => {
            let output = portable_archive::roundtrip(raw.as_bytes())?;
            assert_eq!(
                serde_json::from_str::<Value>(&output)?,
                case["input"],
                "{id}"
            );
        }
        "command" => check_command(case, raw, cases)?,
        contract => panic!("{id}: unsupported fixture contract {contract}"),
    }
    Ok(())
}

fn check_command(case: &Value, raw: &str, cases: &[Value]) -> Result<()> {
    let id = case["id"].as_str().unwrap();
    let original: State = serde_json::from_value(case["state"].clone()).unwrap();
    validate(&original).expect("The fixture must begin with a valid state");
    let command = NotebookCommand::parse(raw.as_bytes()).expect("Expected a well-formed command");

    if id == "atomic-review" {
        let mut stale = case["input"].clone();
        stale["evidence"] = "0".repeat(64).into();
        let stale = NotebookCommand::parse(&serde_json::to_vec(&stale)?).unwrap();
        let mut unchanged = original.clone();
        assert!(stale.apply(&mut unchanged).is_err());
        assert_eq!(unchanged, original, "A rejected review must preserve state");
    }

    let mut updated = original.clone();
    if let Err(error) = command.apply(&mut updated) {
        assert_eq!(updated, original, "{id}: rejected command changed state");
        return Err(error);
    }

    validate(&updated)?;
    assert_eq!(updated.schema, ORDER_ARCHIVE_SCHEMA, "{id}");
    let (expected_id, field) = match id {
        "atomic-review" => ("exact-visible-append", "state"),
        "exact-visible-append" => ("v4-reviewed-then-authored", "input"),
        _ => panic!("{id}: unexpected accepted command"),
    };
    let expected = cases
        .iter()
        .find(|entry| entry["id"] == expected_id)
        .unwrap();
    assert_eq!(serde_json::to_value(updated)?, expected[field], "{id}");
    Ok(())
}
