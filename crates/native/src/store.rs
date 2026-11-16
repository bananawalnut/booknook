use crate::Result;
use booknook_core::{
    Command,
    model::State,
    providers::{Capability, NotebookStorage, Provider, ProviderDescriptor},
};
use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};

pub struct Store {
    pub connection: Connection,
    pub root: PathBuf,
}
impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        Self::open_with_root_permissions(root, |path, permissions| {
            std::fs::set_permissions(path, permissions)
        })
    }
    fn open_with_root_permissions(
        root: &Path,
        mut repair_root: impl FnMut(&Path, std::fs::Permissions) -> std::io::Result<()>,
    ) -> Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(root.join("assets"))?;
            // Path-based directory repair preserves existing symlink following and
            // owner write/search-only directories that cannot be opened for reading.
            ensure_private_permissions(
                0o700,
                || std::fs::metadata(root).map(|metadata| metadata.permissions()),
                |permissions| repair_root(root, permissions),
            )?;
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
                .open(root.join("booknook.sqlite"))?;
            if !file.metadata()?.is_file() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Notebook database is not a regular file",
                )
                .into());
            }
            // New files are private before SQLite can write; existing regular-file
            // symlinks remain supported. Repair through this owned descriptor.
            ensure_private_permissions(
                0o600,
                || file.metadata().map(|metadata| metadata.permissions()),
                |permissions| file.set_permissions(permissions),
            )?;
        }
        #[cfg(not(unix))]
        {
            let _ = &mut repair_root;
            std::fs::create_dir_all(root.join("assets"))?;
        }
        let connection = Connection::open(root.join("booknook.sqlite"))?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; CREATE TABLE IF NOT EXISTS desk (singleton INTEGER PRIMARY KEY CHECK(singleton=1), revision INTEGER NOT NULL, json TEXT NOT NULL); CREATE TABLE IF NOT EXISTS consumed_receipts (nonce TEXT PRIMARY KEY, consumed_at INTEGER NOT NULL);")?;
        let initial = serde_json::to_string(&State::default())?;
        connection.execute("INSERT OR IGNORE INTO desk VALUES(1,0,?1)", [initial])?;
        let mut store = Self {
            connection,
            root: root.to_owned(),
        };
        store.load()?;
        crate::edit_journal::recover(&mut store)?;
        Ok(store)
    }
    pub fn load(&self) -> Result<State> {
        Ok(NotebookStorage::load(self)?)
    }
    pub fn save(&mut self, expected: u64, state: &State) -> Result<()> {
        Ok(self.compare_and_save(expected, state)?)
    }
    /// Keep source selection and the notebook CAS in one cross-handle transaction.
    pub(crate) fn save_for_provider(
        &mut self,
        expected: u64,
        state: &State,
        settings_digest: &str,
    ) -> Result<()> {
        booknook_core::validate(state)?;
        let json = serde_json::to_string(state)?;
        if json.len() > booknook_core::MAX_JSON_BYTES {
            return Err(booknook_core::Error::Invalid("Desk exceeds storage limit".into()).into());
        }
        let revision = i64::try_from(state.revision)
            .map_err(|_| booknook_core::Error::Invalid("Revision exceeds SQLite range".into()))?;
        let expected = i64::try_from(expected)
            .map_err(|_| booknook_core::Error::Invalid("Revision exceeds SQLite range".into()))?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let settings = crate::connection_bridge::load(&tx)?;
        if booknook_core::digest(serde_json::to_string(&settings)?.as_bytes()) != settings_digest {
            return Err(booknook_core::Error::Conflict(
                "Connection settings changed before provider commit".into(),
            )
            .into());
        }
        if tx.execute(
            "UPDATE desk SET revision=?1,json=?2 WHERE singleton=1 AND revision=?3",
            params![revision, json, expected],
        )? != 1
        {
            return Err(booknook_core::Error::Conflict(
                "Local store changed. Reload and retry.".into(),
            )
            .into());
        }
        tx.commit()?;
        Ok(())
    }
    pub fn execute(&mut self, command: Command) -> Result<String> {
        Ok(serde_json::to_string(&booknook_core::providers::execute(
            self, command,
        )?)?)
    }
}

impl Provider for Store {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: "booknook.sqlite".into(),
            capabilities: vec![Capability::NotebookStorage],
            available: true,
            reason: None,
        }
    }
}
impl NotebookStorage for Store {
    fn load(&self) -> booknook_core::Result<State> {
        let raw: String = self
            .connection
            .query_row("SELECT json FROM desk WHERE singleton=1", [], |r| r.get(0))
            .map_err(storage_error)?;
        let state: State = booknook_core::strict_parse(raw.as_bytes())?;
        booknook_core::validate(&state)?;
        Ok(state)
    }
    fn compare_and_save(&self, expected: u64, state: &State) -> booknook_core::Result<()> {
        booknook_core::validate(state)?;
        let json = serde_json::to_string(state)?;
        if json.len() > booknook_core::MAX_JSON_BYTES {
            return Err(booknook_core::Error::Invalid(
                "Desk exceeds the development build's 128 MiB state limit; nothing was committed"
                    .into(),
            ));
        }
        let revision = i64::try_from(state.revision)
            .map_err(|_| booknook_core::Error::Invalid("Revision exceeds SQLite range".into()))?;
        let expected = i64::try_from(expected)
            .map_err(|_| booknook_core::Error::Invalid("Revision exceeds SQLite range".into()))?;
        // rusqlite's shared-reference transaction still rejects nested transactions. The
        // revision predicate is the atomic concurrency boundary for independent handles.
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(storage_error)?;
        let count = tx
            .execute(
                "UPDATE desk SET revision=?1,json=?2 WHERE singleton=1 AND revision=?3",
                params![revision, json, expected],
            )
            .map_err(storage_error)?;
        if count != 1 {
            return Err(booknook_core::Error::Conflict(
                "Local store changed. Reload and retry.".into(),
            ));
        }
        tx.commit().map_err(storage_error)?;
        Ok(())
    }
}

fn storage_error(error: rusqlite::Error) -> booknook_core::Error {
    booknook_core::Error::Unavailable(format!("SQLite: {error}"))
}

/// Preserve exact private modes without repeating permission-changing syscalls on
/// every reopen. Errors and an ineffective repair fail closed; metadata reads and
/// necessary repairs are still synchronous filesystem I/O, not deadline-bounded.
#[cfg(unix)]
pub(crate) fn ensure_private_permissions(
    mode: u32,
    mut read: impl FnMut() -> std::io::Result<std::fs::Permissions>,
    mut repair: impl FnMut(std::fs::Permissions) -> std::io::Result<()>,
) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if read()?.mode() & 0o7777 == mode {
        return Ok(());
    }
    repair(std::fs::Permissions::from_mode(mode))?;
    if read()?.mode() & 0o7777 != mode {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "Private storage permissions could not be verified",
        ));
    }
    Ok(())
}

#[cfg(all(test, unix))]
#[path = "store_permissions_tests.rs"]
mod permissions_tests;
