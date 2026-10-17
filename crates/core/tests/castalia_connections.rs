use booknook_core::{castalia_connections::*, settings::ConnectionSettings};
use serde_json::{Value, json};

#[test]
fn shared_castalia_fixtures_have_identical_rust_acceptance() {
    for (raw, profile) in [
        (
            include_str!("fixtures/castalia/provider-connections-v1-cases.json"),
            false,
        ),
        (
            include_str!("fixtures/castalia/native-authority-profile-v1-cases.json"),
            true,
        ),
    ] {
        let fixtures: Value = serde_json::from_str(raw).unwrap();
        for case in fixtures["cases"].as_array().unwrap() {
            let encoded = serde_json::to_vec(&case["value"]).unwrap();
            let accepted = if profile {
                NativeAuthorityProfile::parse(&encoded).is_ok()
            } else {
                ProviderConnection::parse(&encoded).is_ok()
            };
            assert_eq!(
                accepted,
                case["valid"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
        }
    }
}
#[test]
fn legacy_settings_load_without_connection_or_profile_metadata() {
    let raw = br#"{"schema":"booknook.connections.v1","castalia":null,"nostr_relays":[],"nostr_public_key":null,"audible_marketplace":"us","audible_test_asin":""}"#;
    let settings: ConnectionSettings = booknook_core::strict_parse(raw).unwrap();
    assert_eq!(settings.castalia_profile_name, None);
    assert!(settings.provider_connections.is_empty());
    booknook_core::settings::validate(&settings).unwrap();
}
#[test]
fn duplicate_keys_and_secret_fields_are_rejected_before_deserialization() {
    let raw = include_str!("fixtures/castalia/provider-connections-v1.json");
    let duplicate = raw.replacen('{', "{\"schema\":\"castalia.provider-connection.v1\",", 1);
    assert!(ProviderConnection::parse(duplicate.as_bytes()).is_err());
    let mut connection: Value = serde_json::from_str(raw).unwrap();
    connection["access_token"] = json!("not-a-real-token");
    assert!(ProviderConnection::parse(&serde_json::to_vec(&connection).unwrap()).is_err());
    connection.as_object_mut().unwrap().remove("access_token");
    connection.as_object_mut().unwrap().remove("credential");
    assert!(ProviderConnection::parse(&serde_json::to_vec(&connection).unwrap()).is_err());
}
#[test]
fn registry_rejects_duplicates_and_limits() {
    let connection = ProviderConnection::parse(include_bytes!(
        "fixtures/castalia/provider-connections-v1.json"
    ))
    .unwrap();
    assert!(validate_connections(&[connection.clone(), connection]).is_err());
    assert!(parse_connections(&vec![b' '; 1024 * 1024 + 1]).is_err());
}

#[test]
fn published_connection_schema_requires_an_explicit_nullable_credential() {
    let schema = serde_json::to_value(schemars::schema_for!(ProviderConnection)).unwrap();
    assert!(
        schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("credential"))
    );
    assert!(
        schema["properties"]["credential"]["anyOf"]
            .as_array()
            .unwrap()
            .iter()
            .any(|variant| variant["type"] == "null")
    );
    assert_eq!(schema["additionalProperties"], false);
}
