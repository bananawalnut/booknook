mod audio_bridge;
mod castalia_custody;
pub use castalia_custody::CastaliaCustody;
mod provider_bridge;
pub use provider_bridge::{ProviderHost, ProviderReply};
mod connection_bridge;
pub use connection_bridge::{validate_native_authority_profile, validate_provider_connections};
mod content;
mod custody;
mod edit_journal;
mod notebook_history;
mod notebook_storage;
pub use notebook_storage::{NotebookEngine, NotebookStorageHost};
mod portable;
mod social_bridge;
mod store;
pub use store::Store as SqliteNotebookStorage;
mod sync_bridge;
#[cfg(test)]
mod sync_tests;
use base64::{Engine, engine::general_purpose::STANDARD};
use booknook_core::Command;
pub use booknook_core::portable_archive::PortableArchive;
use std::{path::Path, sync::Mutex};
uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum NativeError {
    #[error("{message}")]
    Failure { message: String },
}
type Result<T> = std::result::Result<T, NativeError>;
macro_rules! convert{($($ty:ty),*)=>{$(impl From<$ty> for NativeError{fn from(e:$ty)->Self{Self::Failure{message:e.to_string()}}})*};}
convert!(
    std::io::Error,
    serde_json::Error,
    rusqlite::Error,
    booknook_core::Error
);

/// Validate and roundtrip the whole native portable envelope without persisting it.
#[uniffi::export]
pub fn validate_portable_archive(archive_json: String) -> Result<String> {
    booknook_core::portable_archive::roundtrip(archive_json.as_bytes()).map_err(Into::into)
}

#[derive(uniffi::Object)]
pub struct Desk {
    store: Mutex<store::Store>,
    custody: Mutex<custody::Custody>,
    providers: Mutex<provider_bridge::ProviderRuntime>,
}

#[uniffi::export]
impl Desk {
    #[uniffi::constructor]
    pub fn new(directory: String) -> Result<Self> {
        Ok(Self {
            store: Mutex::new(store::Store::open(Path::new(&directory))?),
            custody: Mutex::new(custody::Custody::default()),
            providers: Mutex::new(provider_bridge::ProviderRuntime::default()),
        })
    }
    pub fn snapshot(&self) -> Result<String> {
        let s = self.lock()?;
        Ok(serde_json::to_string(&s.load()?)?)
    }
    pub fn execute(&self, command_json: String) -> Result<String> {
        let command: Command = booknook_core::strict_parse(command_json.as_bytes())?;
        self.lock()?.execute(command)
    }
    pub fn canonical_payload(&self, payload_json: String) -> Result<String> {
        String::from_utf8(canonical_payload_bytes(&payload_json)?).map_err(|_| {
            NativeError::Failure {
                message: "Invalid payload encoding".into(),
            }
        })
    }
    pub fn authorized_action(
        &self,
        payload_json: String,
        action: String,
        resource_version: u64,
        idempotency_key: String,
    ) -> Result<String> {
        let policy = self.policy(&action, None)?;
        if idempotency_key.is_empty() || idempotency_key.len() > 200 {
            return Err(
                booknook_core::Error::Invalid("Invalid action idempotency key".into()).into(),
            );
        }
        let request = booknook_core::authority::ActionRequest {
            schema: "booknook.authorized-action.v1".into(),
            action,
            actor: policy.actor,
            payload_digest: booknook_core::digest(&canonical_payload_bytes(&payload_json)?),
            resource_version,
            idempotency_key,
        };
        Ok(String::from_utf8(booknook_core::canonical(&request)?).unwrap())
    }
    pub fn custody_unlock(&self, recovery_json: String, passphrase: String) -> Result<String> {
        Ok(self
            .custody_lock_guard()?
            .unlock(recovery_json.as_bytes(), passphrase)?)
    }
    /// Validate an import candidate in isolated custody without replacing the live identity.
    /// The temporary decrypted key is zeroized when this call returns.
    pub fn custody_validate_recovery(
        &self,
        recovery_json: String,
        passphrase: String,
    ) -> Result<String> {
        Ok(custody::Custody::default().unlock(recovery_json.as_bytes(), passphrase)?)
    }
    pub fn custody_lock(&self) -> Result<()> {
        self.custody_lock_guard()?.lock();
        Ok(())
    }
    pub fn custody_public_key(&self) -> Result<Option<String>> {
        Ok(self.custody_lock_guard()?.public_key())
    }
    pub fn custody_prepare(
        &self,
        request_json: String,
        payload_json: String,
        action: String,
        resource_version: u64,
    ) -> Result<String> {
        let policy = self.policy(&action, None)?;
        let payload = canonical_payload_bytes(&payload_json)?;
        let prepared = self.custody_lock_guard()?.prepare(
            request_json.as_bytes(),
            &payload,
            policy,
            resource_version,
            now()?,
        )?;
        Ok(serde_json::to_string(&prepared)?)
    }
    pub fn custody_approve(&self, token: String) -> Result<String> {
        let action = self
            .custody_lock_guard()?
            .pending_action(&token)
            .map(str::to_owned)
            .ok_or_else(|| booknook_core::Error::Denied("Approval expired or cancelled".into()))?;
        let policy = self.policy(&action, None)?;
        Ok(self
            .custody_lock_guard()?
            .approve(&token, &policy, now()?)?)
    }
    pub fn custody_cancel(&self, token: String) -> Result<()> {
        self.custody_lock_guard()?.cancel(&token);
        Ok(())
    }
    pub fn release_gates(&self) -> Result<String> {
        Ok(serde_json::to_string(
            &booknook_core::providers::release_gates(),
        )?)
    }
    pub fn connection_settings(&self) -> Result<String> {
        Ok(serde_json::to_string(&connection_bridge::load(
            &self.lock()?.connection,
        )?)?)
    }
    pub fn save_connection_settings(&self, settings_json: String) -> Result<()> {
        let settings: booknook_core::settings::ConnectionSettings =
            booknook_core::strict_parse(settings_json.as_bytes())?;
        booknook_core::settings::validate(&settings)?;
        let store = self.lock()?;
        store.connection.execute_batch("CREATE TABLE IF NOT EXISTS settings (id INTEGER PRIMARY KEY CHECK(id=1),json TEXT NOT NULL);")?;
        store.connection.execute("INSERT INTO settings(id,json) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json",[serde_json::to_string(&settings)?])?;
        Ok(())
    }
    pub fn import_content(&self, book_id: String, path: String) -> Result<String> {
        let source = Path::new(&path);
        let meta = std::fs::metadata(source)?;
        if meta.len() > 128 * 1024 * 1024 {
            return Err(NativeError::Failure {
                message: "Content exceeds 128 MiB".into(),
            });
        }
        let bytes = portable::read_bounded(source)?;
        let kind = source
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or("")
            .to_lowercase();
        let mut store = self.lock()?;
        let mut state = store.load()?;
        let revision = state.revision;
        if !state.books.contains_key(&book_id) {
            return Err(NativeError::Failure {
                message: "Book/edition missing".into(),
            });
        }
        let asset = content::import(&store.root, &bytes, &kind)?;
        if !state
            .editions
            .values()
            .any(|e| e.book_id == book_id && e.assets.iter().any(|a| a.id == asset.id))
        {
            // No edition equivalence is inferred from attaching a file to a literary work.
            let edition_id = format!("{}:content:{}", book_id, asset.sha256);
            state.editions.insert(
                edition_id.clone(),
                booknook_core::model::Edition {
                    id: edition_id.clone(),
                    book_id: book_id.clone(),
                    identifiers: vec![],
                    language: None,
                    assets: vec![asset],
                },
            );
            state
                .books
                .get_mut(&book_id)
                .unwrap()
                .edition_ids
                .push(edition_id);
        }
        state.revision += 1;
        store.save(revision, &state)?;
        Ok(serde_json::to_string(&state)?)
    }
    pub fn asset_directory(&self, asset_id: String) -> Result<String> {
        let store = self.lock()?;
        let state = store.load()?;
        let asset = state
            .editions
            .values()
            .flat_map(|e| &e.assets)
            .find(|a| a.id == asset_id)
            .ok_or_else(|| NativeError::Failure {
                message: "Asset missing".into(),
            })?;
        Ok(store
            .root
            .join("assets")
            .join(&asset.storage_key)
            .to_string_lossy()
            .into_owned())
    }
    pub fn export_archive(&self, path: String) -> Result<()> {
        let mut store = self.lock()?;
        edit_journal::recover(&mut store)?;
        let state = store.load()?;
        let mut assets = std::collections::BTreeMap::new();
        let mut encoded_total = 0usize;
        for asset in state.editions.values().flat_map(|e| &e.assets) {
            if assets.contains_key(&asset.id) {
                continue;
            }
            let name = if asset.media_type == "application/pdf" {
                "book.pdf"
            } else {
                "original.epub"
            };
            let source = store
                .root
                .join("assets")
                .join(&asset.storage_key)
                .join(name);
            let size = std::fs::metadata(&source)?.len();
            if size > booknook_core::MAX_JSON_BYTES as u64 {
                return Err(NativeError::Failure {
                    message: "Content exceeds 128 MiB".into(),
                });
            }
            encoded_total = encoded_total.saturating_add((size as usize).div_ceil(3) * 4);
            if encoded_total > booknook_core::MAX_JSON_BYTES {
                return Err(NativeError::Failure {
                    message:
                        "Portable export exceeds 128 MiB. Existing export and desk are unchanged."
                            .into(),
                });
            }
            let bytes = portable::read_bounded(&source)?;
            if booknook_core::digest(&bytes) != asset.sha256 {
                return Err(NativeError::Failure {
                    message: "Asset checksum mismatch".into(),
                });
            }
            assets.insert(asset.id.clone(), STANDARD.encode(bytes));
        }
        let archive = PortableArchive {
            schema: "booknook.portable-export.v1".into(),
            state,
            assets,
            social: Some(social_bridge::export_bundle(&store.connection)?),
        };
        portable::write_atomic(Path::new(&path), &archive, booknook_core::MAX_JSON_BYTES)
    }
    /// Restores only into an empty desk. Never silently replaces an existing reader's library.
    pub fn restore_archive(&self, path: String) -> Result<String> {
        let archive =
            booknook_core::portable_archive::parse(&portable::read_bounded(Path::new(&path))?)?;
        let mut store = self.lock()?;
        let current = store.load()?;
        if current.revision != 0 || !current.books.is_empty() {
            return Err(NativeError::Failure {
                message: "Restore requires an empty desk to protect existing work".into(),
            });
        }
        for asset in archive.state.editions.values().flat_map(|e| &e.assets) {
            let encoded = archive
                .assets
                .get(&asset.id)
                .ok_or_else(|| NativeError::Failure {
                    message: "Export is missing an asset".into(),
                })?;
            let bytes = STANDARD.decode(encoded).map_err(|e| NativeError::Failure {
                message: e.to_string(),
            })?;
            if booknook_core::digest(&bytes) != asset.sha256 {
                return Err(NativeError::Failure {
                    message: "Export asset checksum mismatch".into(),
                });
            }
            let kind = if asset.media_type == "application/pdf" {
                "pdf"
            } else {
                "epub"
            };
            let actual = content::import(&store.root, &bytes, kind)?;
            if &actual != asset {
                return Err(NativeError::Failure {
                    message: "Export asset metadata mismatch".into(),
                });
            }
        }
        let mut state = archive.state;
        state.revision = current.revision + 1;
        let json = serde_json::to_string(&state)?;
        if json.len() > booknook_core::MAX_JSON_BYTES {
            return Err(
                booknook_core::Error::Invalid("Restored state exceeds 128 MiB".into()).into(),
            );
        }
        let tx = store.connection.transaction()?;
        if let Some(social) = &archive.social {
            social_bridge::write_archive_bundle(&tx, social)?;
        }
        let changed = tx.execute(
            "UPDATE desk SET revision=?1,json=?2 WHERE singleton=1 AND revision=0",
            rusqlite::params![state.revision as i64, json],
        )?;
        if changed != 1 {
            return Err(
                booknook_core::Error::Conflict("Desk changed during restore".into()).into(),
            );
        }
        tx.commit()?;
        Ok(serde_json::to_string(&state)?)
    }
}
impl Desk {
    fn policy(
        &self,
        action: &str,
        actor: Option<&str>,
    ) -> Result<booknook_core::authority::AuthorityPolicy> {
        use booknook_core::authority::{AuthorityPolicy, Caller, TrustPin};
        if ![
            "sync.send",
            "device.enroll",
            "social.execute",
            "social.publish",
            "nostr.bind",
            "group.membership",
        ]
        .contains(&action)
        {
            return Err(booknook_core::Error::Denied("Unsupported Book Nook action".into()).into());
        }
        let settings: booknook_core::settings::ConnectionSettings =
            booknook_core::strict_parse(self.connection_settings()?.as_bytes())?;
        let c = settings.castalia.ok_or_else(|| {
            booknook_core::Error::Unavailable("Configure Castalia in Connections".into())
        })?;
        if c.policy_digest.len() != 64 {
            return Err(booknook_core::Error::Unavailable(
                "Configure the issuer's policy digest in Connections".into(),
            )
            .into());
        }
        let caller = Caller {
            kind: "terminal".into(),
            id: c.terminal_caller,
        };
        Ok(AuthorityPolicy {
            pins: vec![TrustPin {
                issuer: c.issuer,
                key_id: c.key_id,
                public_key: c.issuer_public_key,
                audience: c.audience.clone(),
                callers: vec![caller.clone()],
            }],
            expected_caller: caller,
            audience: c.audience,
            actor: actor.unwrap_or(&c.holder_public_key).into(),
            action: action.into(),
            policy_digest: c.policy_digest,
        })
    }
    fn custody_lock_guard(&self) -> Result<std::sync::MutexGuard<'_, custody::Custody>> {
        self.custody.lock().map_err(|_| NativeError::Failure {
            message: "Custody lock unavailable; reopen Book Nook".into(),
        })
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, store::Store>> {
        self.store.lock().map_err(|_| NativeError::Failure {
            message: "Storage lock poisoned; reopen for recovery".into(),
        })
    }
}
fn canonical_payload_bytes(json: &str) -> Result<Vec<u8>> {
    let value: serde_json::Value = booknook_core::strict_parse(json.as_bytes())?;
    Ok(booknook_core::canonical(&value)?)
}
fn now() -> Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| NativeError::Failure {
            message: "System clock precedes Unix epoch".into(),
        })?
        .as_secs())
}
