fn main() {
    let output = serde_json::json!({
        "portable-export": schemars::schema_for!(booknook_core::portable_archive::PortableArchive),
        "social-archive": schemars::schema_for!(booknook_core::portable_archive::SocialArchiveBundle),
        "provider-registry": schemars::schema_for!(booknook_core::provider_runtime::ProviderRegistry),
        "provider-operation": schemars::schema_for!(booknook_core::provider_runtime::ProviderOperation),
        "provider-request": schemars::schema_for!(booknook_core::provider_runtime::ProviderRequest),
        "provider-response": schemars::schema_for!(booknook_core::provider_runtime::ProviderResponse),
        "provider-execution": schemars::schema_for!(booknook_core::provider_runtime::ProviderExecution),
        "audio-observation": schemars::schema_for!(booknook_core::audio::AudioObservation),
        "audio-candidates": schemars::schema_for!(Vec<booknook_core::audio::AudioEditionCandidate>),
        "note": schemars::schema_for!(booknook_core::model::NoteMetadata),
        "captured-reading-context": schemars::schema_for!(booknook_core::model::CapturedReadingContext),
        "notebook-ink": schemars::schema_for!(booknook_core::model::NotebookInk),
        "state": schemars::schema_for!(booknook_core::model::State),
        "command": schemars::schema_for!(booknook_core::Command),
        "castalia-provider-connection": schemars::schema_for!(booknook_core::castalia_connections::ProviderConnection),
        "castalia-native-authority-profile": schemars::schema_for!(booknook_core::castalia_connections::NativeAuthorityProfile),
        "connections": schemars::schema_for!(booknook_core::settings::ConnectionSettings),
        "identity": schemars::schema_for!(booknook_core::identity::AttributionLink),
        "social": schemars::schema_for!(booknook_core::social_protocol::SocialState),
        "social-action": schemars::schema_for!(booknook_core::social_protocol::SocialAction),
        "social-local": schemars::schema_for!(booknook_core::social_protocol::LocalSocialCommand),
        "social-event": schemars::schema_for!(booknook_core::social_protocol::SocialEvent),
        "publication": schemars::schema_for!(booknook_core::social_protocol::ApprovedPublication),
        "authority-policy": schemars::schema_for!(booknook_core::authority::AuthorityPolicy)
    });
    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}
