use booknook_core::settings::*;
#[test]
fn settings_validate_public_configuration_without_embedding_secrets() {
    let mut s = ConnectionSettings::default();
    validate(&s).unwrap();
    s.nostr_relays = vec!["wss://relay.example".into()];
    validate(&s).unwrap();
    s.nostr_relays = vec!["wss://user:secret@relay.example".into()];
    assert!(validate(&s).is_err());
    s.nostr_relays.clear();
    s.castalia = Some(CastaliaSettings {
        issuer: "issuer".into(),
        key_id: "key".into(),
        issuer_public_key: "11".repeat(32),
        audience: "booknook".into(),
        terminal_caller: "booknook-native".into(),
        policy_digest: String::new(),
        credential_endpoint: String::new(),
        holder_public_key: "22".repeat(32),
    });
    validate(&s).unwrap();
    s.castalia.as_mut().unwrap().issuer_public_key = "invalid".into();
    assert!(validate(&s).is_err());
    assert!(booknook_core::strict_parse::<ConnectionSettings>(br#"{"schema":"booknook.connections.v1","castalia":null,"nostr_relays":[],"nostr_public_key":null,"audible_marketplace":"us","audible_test_asin":"","access_token":"secret"}"#).is_err());
}
