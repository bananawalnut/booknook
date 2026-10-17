// SPDX-License-Identifier: MIT OR Apache-2.0
use crate::{Desk, Result};
use booknook_core::{
    Error,
    castalia_connections::*,
    settings::{CastaliaSettings, ConnectionSettings},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

pub(crate) fn load(connection: &Connection) -> Result<ConnectionSettings> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS settings (id INTEGER PRIMARY KEY CHECK(id=1),json TEXT NOT NULL);")?;
    let raw: Option<String> = connection
        .query_row("SELECT json FROM settings WHERE id=1", [], |r| r.get(0))
        .optional()?;
    let settings = match raw {
        Some(raw) => booknook_core::strict_parse(raw.as_bytes())?,
        None => ConnectionSettings::default(),
    };
    booknook_core::settings::validate(&settings)?;
    Ok(settings)
}
fn persist(connection: &Connection, settings: &ConnectionSettings) -> Result<String> {
    booknook_core::settings::validate(settings)?;
    let json = serde_json::to_string(settings)?;
    connection.execute("INSERT INTO settings(id,json) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json", [&json])?;
    Ok(json)
}
fn digest(settings: &ConnectionSettings) -> Result<String> {
    Ok(booknook_core::digest(&booknook_core::canonical(settings)?))
}
#[uniffi::export]
impl Desk {
    /// Compare the persisted baseline before saving an edited settings form. Independent
    /// windows may never replace a newly reviewed authority with an old cached form.
    pub fn save_connection_settings_if_current(
        &self,
        settings_json: String,
        expected_settings_json: String,
    ) -> Result<String> {
        let settings: ConnectionSettings = booknook_core::strict_parse(settings_json.as_bytes())?;
        let expected: ConnectionSettings =
            booknook_core::strict_parse(expected_settings_json.as_bytes())?;
        booknook_core::settings::validate(&settings)?;
        booknook_core::settings::validate(&expected)?;
        let mut store = self.lock()?;
        let tx = store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = load(&tx)?;
        if digest(&current)? != digest(&expected)? {
            return Err(Error::Conflict("Connection settings changed in another operation or window. Reload saved settings and review your edits before saving.".into()).into());
        }
        let result = persist(&tx, &settings)?;
        tx.commit()?;
        Ok(result)
    }
    /// Preview public metadata only. Trust is installed exclusively through an explicit apply action.
    pub fn preview_castalia_profile(&self, profile_json: String) -> Result<String> {
        let profile = NativeAuthorityProfile::parse(profile_json.as_bytes())?;
        let settings = load(&self.lock()?.connection)?;
        Ok(serde_json::to_string(&serde_json::json!({
            "schema":"booknook.castalia-profile-preview.v1",
            "profile_digest_sha256":booknook_core::digest(&booknook_core::canonical(&profile)?),
            "settings_digest_sha256":digest(&settings)?,
            "profile":profile
        }))?)
    }
    pub fn install_castalia_profile(
        &self,
        profile_json: String,
        expected_profile_digest: String,
        expected_settings_digest: String,
        holder_public_key: String,
    ) -> Result<String> {
        let profile = NativeAuthorityProfile::parse(profile_json.as_bytes())?;
        if !valid_hex(&holder_public_key) {
            return Err(
                Error::Invalid("Select an existing Castalia public identity".into()).into(),
            );
        }
        if expected_profile_digest != booknook_core::digest(&booknook_core::canonical(&profile)?) {
            return Err(
                Error::Conflict("Profile changed after review; preview it again".into()).into(),
            );
        }
        let mut store = self.lock()?;
        let tx = store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut settings = load(&tx)?;
        if digest(&settings)? != expected_settings_digest {
            return Err(Error::Conflict(
                "Connection settings changed after review; preview the profile again".into(),
            )
            .into());
        }
        settings.castalia_profile_name = Some(profile.display_name);
        settings.castalia = Some(CastaliaSettings {
            issuer: profile.issuer,
            key_id: profile.key_id,
            issuer_public_key: profile.issuer_public_key,
            audience: profile.audience,
            terminal_caller: profile.caller.id,
            holder_public_key,
            policy_digest: profile.policy_digest_sha256,
            credential_endpoint: profile.credential_endpoint,
        });
        let result = persist(&tx, &settings)?;
        tx.commit()?;
        Ok(result)
    }
    /// Export public authority configuration; never includes holder custody or provider secrets.
    pub fn export_castalia_profile(&self) -> Result<String> {
        let settings = load(&self.lock()?.connection)?;
        let c = settings
            .castalia
            .ok_or_else(|| Error::Unavailable("Configure a Castalia authority first".into()))?;
        let profile = NativeAuthorityProfile {
            schema: NativeAuthorityProfileSchema::V1,
            display_name: settings
                .castalia_profile_name
                .unwrap_or_else(|| "Castalia authority".into()),
            issuer: c.issuer,
            key_id: c.key_id,
            issuer_public_key: c.issuer_public_key,
            audience: c.audience,
            caller: NativeCaller {
                kind: NativeCallerKind::Terminal,
                id: c.terminal_caller,
            },
            policy_digest_sha256: c.policy_digest,
            credential_endpoint: c.credential_endpoint,
        };
        profile.validate()?;
        Ok(serde_json::to_string(&profile)?)
    }
    pub fn validate_provider_connection(&self, connection_json: String) -> Result<String> {
        Ok(serde_json::to_string(&ProviderConnection::parse(
            connection_json.as_bytes(),
        )?)?)
    }
    /// Replace the selected companion's public registry only; authority and unrelated settings survive.
    pub fn save_provider_connections(&self, connections_json: String) -> Result<String> {
        let connections = parse_connections(connections_json.as_bytes())?;
        let mut store = self.lock()?;
        let tx = store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut settings = load(&tx)?;
        settings.provider_connections = connections;
        let result = persist(&tx, &settings)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn import_kindle_from_connection(
        &self,
        catalog_json: String,
        at: u64,
        connection_id: String,
    ) -> Result<String> {
        let mut store = self.lock()?;
        require_connection(
            &load(&store.connection)?,
            &connection_id,
            "com.amazon.kindle",
            "kindle.catalog",
        )?;
        store.execute(booknook_core::Command::ImportKindleFromConnection {
            json: catalog_json,
            at,
            provider_connection_id: connection_id,
        })
    }
}
pub(crate) fn require_connection(
    settings: &ConnectionSettings,
    id: &str,
    provider: &str,
    capability: &str,
) -> Result<()> {
    if !settings.provider_connections.iter().any(|c| {
        c.connection_id == id
            && c.provider_id == provider
            && c.status == ConnectionStatus::Available
            && c.capabilities.iter().any(|s| s == capability)
    }) {
        return Err(Error::Unavailable("Provider connection is unavailable or does not provide this capability; refresh connections".into()).into());
    }
    Ok(())
}

#[uniffi::export]
pub fn validate_provider_connections(connections_json: String) -> Result<String> {
    Ok(serde_json::to_string(&parse_connections(
        connections_json.as_bytes(),
    )?)?)
}

#[uniffi::export]
pub fn validate_native_authority_profile(profile_json: String) -> Result<String> {
    Ok(serde_json::to_string(&NativeAuthorityProfile::parse(
        profile_json.as_bytes(),
    )?)?)
}
