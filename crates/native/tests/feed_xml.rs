use booknook_native::normalize_feed_xml_v1;
use serde_json::Value;
#[test]
fn native_normalization_matches_core_and_all_synthetic_adversarial_cases() {
    let corpus: Value =
        serde_json::from_slice(include_bytes!("fixtures/feed-xml-v1/cases.json")).unwrap();
    let context = serde_json::to_vec(&corpus["context"]).unwrap();
    for case in corpus["cases"].as_array().unwrap() {
        let raw = case["xml"].as_str().unwrap().as_bytes().to_vec();
        let result = normalize_feed_xml_v1(context.clone(), raw.clone());
        assert_eq!(
            result.is_ok(),
            case["valid"].as_bool().unwrap(),
            "{}",
            case["id"]
        );
        if let Ok(result) = result {
            let expected = booknook_core::feed_xml::normalize(&context, &raw).unwrap();
            assert_eq!(result, serde_json::to_string(&expected).unwrap());
            let value: Value = serde_json::from_str(&result).unwrap();
            for (path, expected) in case["checks"].as_object().unwrap() {
                assert_eq!(
                    value.pointer(path).unwrap(),
                    expected,
                    "{} {path}",
                    case["id"]
                );
            }
        }
    }
    assert!(normalize_feed_xml_v1(context, vec![0xff, 0xfe, b'<', 0]).is_err());
}
