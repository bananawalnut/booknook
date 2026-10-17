use booknook_core::{model::State, settings::ConnectionSettings};
use booknook_native::Desk;
use serde_json::{Value, json};
use std::path::PathBuf;
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "booknook-connection-test-{}",
            booknook_core::new_id()
        )))
    }
    fn desk(&self) -> Desk {
        Desk::new(self.0.to_string_lossy().into()).unwrap()
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn profile() -> String {
    include_str!("fixtures/castalia-native-authority-profile-v1.json").into()
}
fn apply(
    desk: &Desk,
    preview: &Value,
    raw: String,
) -> Result<String, booknook_native::NativeError> {
    desk.install_castalia_profile(
        raw,
        preview["profile_digest_sha256"].as_str().unwrap().into(),
        preview["settings_digest_sha256"].as_str().unwrap().into(),
        "33".repeat(32),
    )
}
fn registry() -> String {
    json!([
    {"schema":"castalia.provider-connection.v1","connection_id":"companion.kindle.1","provider_id":"com.amazon.kindle","display_name":"Selected Kindle catalog","capabilities":["kindle.catalog"],"status":"available","credential":null},
    {"schema":"castalia.provider-connection.v1","connection_id":"companion.audible.1","provider_id":"com.audible","display_name":"Audible account","capabilities":["audible.positions"],"status":"available","credential":{"reference":"audible.1","storage":"secure_local","portability":"reauthorization_required"}}
]).to_string()
}
#[test]
fn public_profile_preview_installs_only_reviewed_authority_and_preserves_other_settings() {
    let t = Temp::new();
    let desk = t.desk();
    let old = ConnectionSettings {
        nostr_relays: vec!["wss://relay.example".into()],
        audible_marketplace: "ca".into(),
        ..Default::default()
    };
    desk.save_connection_settings(serde_json::to_string(&old).unwrap())
        .unwrap();
    desk.save_provider_connections(registry()).unwrap();
    let before: Value = serde_json::from_str(&desk.connection_settings().unwrap()).unwrap();
    let preview: Value =
        serde_json::from_str(&desk.preview_castalia_profile(profile()).unwrap()).unwrap();
    assert!(before["castalia"].is_null());
    assert_eq!(
        serde_json::from_str::<Value>(&desk.connection_settings().unwrap()).unwrap(),
        before
    );
    let after: Value = serde_json::from_str(&apply(&desk, &preview, profile()).unwrap()).unwrap();
    for key in [
        "provider_connections",
        "nostr_relays",
        "audible_marketplace",
    ] {
        assert_eq!(before[key], after[key]);
    }
    assert_eq!(after["castalia"]["holder_public_key"], "33".repeat(32));
    let exported: Value = serde_json::from_str(&desk.export_castalia_profile().unwrap()).unwrap();
    assert_eq!(exported, serde_json::from_str::<Value>(&profile()).unwrap());
    assert!(exported.get("holder_public_key").is_none());
    assert!(exported.get("membership").is_none());
    drop(desk);
    assert_eq!(
        serde_json::from_str::<Value>(&t.desk().connection_settings().unwrap()).unwrap(),
        after
    );
}
#[test]
fn stale_review_across_independent_handles_cannot_overwrite_new_settings() {
    let t = Temp::new();
    let first = t.desk();
    let other = t.desk();
    let preview: Value =
        serde_json::from_str(&first.preview_castalia_profile(profile()).unwrap()).unwrap();
    other.save_provider_connections(registry()).unwrap();
    let changed = other.connection_settings().unwrap();
    assert!(
        apply(&first, &preview, profile())
            .unwrap_err()
            .to_string()
            .contains("changed after review")
    );
    assert_eq!(first.connection_settings().unwrap(), changed);
    let fresh: Value =
        serde_json::from_str(&first.preview_castalia_profile(profile()).unwrap()).unwrap();
    let modified = profile().replace("independent-fixture", "different-issuer");
    assert!(apply(&first, &fresh, modified).is_err());
    assert_eq!(first.connection_settings().unwrap(), changed);
}
#[test]
fn registry_refresh_never_changes_authority_and_rejects_secrets_atomically() {
    let t = Temp::new();
    let desk = t.desk();
    let preview: Value =
        serde_json::from_str(&desk.preview_castalia_profile(profile()).unwrap()).unwrap();
    apply(&desk, &preview, profile()).unwrap();
    let exported = desk.export_castalia_profile().unwrap();
    desk.save_provider_connections(registry()).unwrap();
    assert_eq!(desk.export_castalia_profile().unwrap(), exported);
    let before = desk.connection_settings().unwrap();
    let mut bad: Value = serde_json::from_str(&registry()).unwrap();
    bad[1]["credential"]["access_token"] = json!("synthetic-secret");
    assert!(desk.save_provider_connections(bad.to_string()).is_err());
    assert_eq!(desk.connection_settings().unwrap(), before);
    desk.save_provider_connections("[]".into()).unwrap();
    assert_eq!(desk.export_castalia_profile().unwrap(), exported);
}
#[test]
fn connection_references_survive_duplicate_import_and_lossless_portable_export() {
    let t = Temp::new();
    let desk = t.desk();
    desk.save_provider_connections(registry()).unwrap();
    let catalog=json!({"schema":"booknook.kindle-catalog.v1","items":[{"asin":"B000000001","title":"Fixture","authors":["Author"],"position":"chapter:3","observed_at":10}]}).to_string();
    assert!(
        desk.import_kindle_from_connection(catalog.clone(), 20, "companion.audible.1".into())
            .is_err()
    );
    desk.import_kindle_from_connection(catalog.clone(), 20, "companion.kindle.1".into())
        .unwrap();
    desk.import_kindle_from_connection(catalog, 21, "companion.kindle.1".into())
        .unwrap();
    let initial: State = serde_json::from_str(&desk.snapshot().unwrap()).unwrap();
    assert_eq!(initial.books.len(), 1);
    let book = initial.books.values().next().unwrap();
    assert_eq!(book.provenance.len(), 1);
    assert_eq!(
        book.provenance[0].provider_connection_id.as_deref(),
        Some("companion.kindle.1")
    );
    assert_eq!(
        initial
            .positions
            .values()
            .next()
            .unwrap()
            .provider_connection_id
            .as_deref(),
        Some("companion.kindle.1")
    );
    let pass = initial.passes.keys().next().unwrap().clone();
    let observation=json!({"asin":"B000000002","position_milliseconds":500,"source_updated_at":10,"fetched_at":20,"duration_milliseconds":1000,"edition_identifier":null,"alignment":"unmatched"}).to_string();
    desk.attach_audio_observation_from_connection(
        observation,
        pass,
        None,
        "companion.audible.1".into(),
    )
    .unwrap();
    let before: State = serde_json::from_str(&desk.snapshot().unwrap()).unwrap();
    assert!(
        before
            .positions
            .values()
            .any(|p| p.provider_connection_id.as_deref() == Some("companion.audible.1"))
    );
    let path = t.0.join("portable.json");
    desk.export_archive(path.to_string_lossy().into()).unwrap();
    let imported = Temp::new();
    let restored = imported.desk();
    restored
        .restore_archive(path.to_string_lossy().into())
        .unwrap();
    let mut after: State = serde_json::from_str(&restored.snapshot().unwrap()).unwrap();
    after.revision = before.revision;
    assert_eq!(before, after);
    desk.save_provider_connections("[]".into()).unwrap();
    assert!(
        desk.import_kindle_from_connection("{}".into(), 22, "companion.kindle.1".into())
            .is_err()
    );
}

#[test]
fn stale_manual_form_cannot_restore_prior_authority_or_provider_registry() {
    let t = Temp::new();
    let first = t.desk();
    let second = t.desk();
    let baseline = second.connection_settings().unwrap();
    let mut edited: Value = serde_json::from_str(&baseline).unwrap();
    edited["audible_marketplace"] = json!("ca");
    let preview: Value =
        serde_json::from_str(&first.preview_castalia_profile(profile()).unwrap()).unwrap();
    apply(&first, &preview, profile()).unwrap();
    let reviewed = first.connection_settings().unwrap();
    assert!(
        second
            .save_connection_settings_if_current(edited.to_string(), baseline)
            .is_err()
    );
    assert_eq!(second.connection_settings().unwrap(), reviewed);
    let mut fresh: Value = serde_json::from_str(&reviewed).unwrap();
    fresh["audible_marketplace"] = json!("ca");
    second
        .save_connection_settings_if_current(fresh.to_string(), reviewed)
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&first.connection_settings().unwrap()).unwrap(),
        fresh
    );
}

#[test]
fn equal_positions_from_independent_connections_keep_both_source_references() {
    let t = Temp::new();
    let desk = t.desk();
    let mut connections: Value = serde_json::from_str(&registry()).unwrap();
    let mut kindle = connections[0].clone();
    kindle["connection_id"] = json!("companion.kindle.2");
    let mut audible = connections[1].clone();
    audible["connection_id"] = json!("companion.audible.2");
    connections
        .as_array_mut()
        .unwrap()
        .extend([kindle, audible]);
    desk.save_provider_connections(connections.to_string())
        .unwrap();
    let catalog=json!({"schema":"booknook.kindle-catalog.v1","items":[{"asin":"B000000001","title":"Fixture","authors":["Author"],"position":"chapter:3","observed_at":10}]}).to_string();
    for id in [
        "companion.kindle.1",
        "companion.kindle.2",
        "companion.kindle.1",
    ] {
        desk.import_kindle_from_connection(catalog.clone(), 20, id.into())
            .unwrap();
    }
    let state: State = serde_json::from_str(&desk.snapshot().unwrap()).unwrap();
    assert_eq!(state.books.len(), 1);
    assert_eq!(state.positions.len(), 2);
    assert_eq!(state.books.values().next().unwrap().provenance.len(), 2);
    let pass = state.passes.keys().next().unwrap().clone();
    let source=json!({"asin":"B000000002","position_milliseconds":500,"source_updated_at":10,"fetched_at":20,"duration_milliseconds":1000,"edition_identifier":null,"alignment":"unmatched"}).to_string();
    for id in [
        "companion.audible.1",
        "companion.audible.2",
        "companion.audible.1",
    ] {
        desk.attach_audio_observation_from_connection(
            source.clone(),
            pass.clone(),
            None,
            id.into(),
        )
        .unwrap();
    }
    let state: State = serde_json::from_str(&desk.snapshot().unwrap()).unwrap();
    assert_eq!(state.positions.len(), 4);
    let refs: std::collections::BTreeSet<_> = state
        .positions
        .values()
        .filter_map(|p| p.provider_connection_id.as_deref())
        .collect();
    assert_eq!(refs.len(), 4);
}
