// SPDX-License-Identifier: MIT OR Apache-2.0
use booknook_core::{Command, Result, identity::AttributionLink, model::State};
use serde_json::Value;

#[test]
fn published_offline_conformance_vectors_match_the_actual_contracts() {
    let bundle: Value = serde_json::from_str(include_str!("fixtures/conformance.json")).unwrap();
    assert_eq!(bundle["schema"], "booknook.conformance.v1");
    assert_eq!(bundle["ontology_version"], booknook_core::model::ONTOLOGY);
    let cases = bundle["cases"].as_array().unwrap();
    assert!(cases.len() >= 20);
    let mut seen = std::collections::BTreeSet::new();
    for case in cases {
        let id = case["id"].as_str().unwrap();
        assert!(seen.insert(id));
        let raw = case["raw_json"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| serde_json::to_string(&case["input"]).unwrap());
        let result: Result<Value> = (|| match case["contract"].as_str().unwrap() {
            "state" => {
                let state: State = booknook_core::strict_parse(raw.as_bytes())?;
                booknook_core::validate(&state)?;
                Ok(serde_json::to_value(state)?)
            }
            "identity" => {
                let identity: AttributionLink = booknook_core::strict_parse(raw.as_bytes())?;
                identity.directory.validate()?;
                Ok(serde_json::to_value(identity)?)
            }
            "command" => {
                let mut state: State = serde_json::from_value(case["state"].clone())?;
                let command: Command = booknook_core::strict_parse(raw.as_bytes())?;
                booknook_core::validate(&state)?;
                let result = booknook_core::apply(&mut state, command)?;
                Ok(serde_json::json!({"state":state,"result":result}))
            }
            contract => panic!("Unsupported fixture contract {contract}"),
        })();
        assert_eq!(
            result.is_ok(),
            case["valid"].as_bool().unwrap(),
            "{id}: {result:?}"
        );
        if let expected = &case["normalized"]
            && !expected.is_null()
        {
            assert_eq!(result.unwrap(), *expected, "{id} canonical output");
        }
    }
}
