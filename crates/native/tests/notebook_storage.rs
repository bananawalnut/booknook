// SPDX-License-Identifier: MIT OR Apache-2.0
use booknook_core::{Command, model::State};
use booknook_native::{NativeError, NotebookEngine, NotebookStorageHost};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

type Result<T> = std::result::Result<T, NativeError>;
fn io(error: impl std::fmt::Display) -> NativeError {
    NativeError::Failure {
        message: error.to_string(),
    }
}
fn descriptor(id: &str, available: bool, capabilities: &[&str]) -> String {
    serde_json::json!({"id": id, "available": available, "capabilities": capabilities,
        "reason": if available { None } else { Some("Synthetic store offline") }})
    .to_string()
}
fn command(title: &str) -> String {
    serde_json::json!({"op":"add_book","title":title,"author":"Synthetic","at":1}).to_string()
}

#[derive(Default)]
struct Controls {
    descriptor: String,
    raw_override: Option<String>,
    descriptor_after_load: Option<String>,
    external_write: bool,
    fail_save: bool,
    fail_descriptor: bool,
    fail_load: bool,
    saves: usize,
}
// A real persistent alternate host: JSON file with atomic rename and a shared
// writer lock. This fixture's lock covers all of its in-process handles/writers.
// Production hosts with multiple processes need their own cross-process CAS.
#[derive(Clone)]
struct FileHost {
    directory: Arc<PathBuf>,
    controls: Arc<Mutex<Controls>>,
}
impl FileHost {
    fn new() -> Self {
        let directory =
            std::env::temp_dir().join(format!("booknook-state-host-{}", booknook_core::new_id()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("state.json"),
            serde_json::to_vec(&State::default()).unwrap(),
        )
        .unwrap();
        Self {
            directory: Arc::new(directory),
            controls: Arc::new(Mutex::new(Controls {
                descriptor: descriptor("example.file", true, &["notebook_storage"]),
                ..Default::default()
            })),
        }
    }
    fn stored(&self) -> State {
        serde_json::from_slice(&std::fs::read(self.directory.join("state.json")).unwrap()).unwrap()
    }
    fn write(&self, state: &State) -> Result<()> {
        std::fs::write(
            self.directory.join("pending.json"),
            serde_json::to_vec(state)?,
        )
        .map_err(io)?;
        std::fs::rename(
            self.directory.join("pending.json"),
            self.directory.join("state.json"),
        )
        .map_err(io)
    }
}
impl Drop for FileHost {
    fn drop(&mut self) {
        if Arc::strong_count(&self.directory) == 1 {
            let _ = std::fs::remove_dir_all(self.directory.as_ref());
        }
    }
}
impl NotebookStorageHost for FileHost {
    fn descriptor_json(&self) -> Result<String> {
        let control = self.controls.lock().unwrap();
        if control.fail_descriptor {
            return Err(io("Synthetic descriptor I/O failure"));
        }
        Ok(control.descriptor.clone())
    }
    fn load_json(&self) -> Result<String> {
        let mut control = self.controls.lock().unwrap();
        if control.fail_load {
            return Err(io("Synthetic read I/O failure"));
        }
        if let Some(descriptor) = control.descriptor_after_load.take() {
            control.descriptor = descriptor;
        }
        match &control.raw_override {
            Some(raw) => Ok(raw.clone()),
            None => std::fs::read_to_string(self.directory.join("state.json")).map_err(io),
        }
    }
    fn compare_and_save(&self, expected_revision: u64, state_json: String) -> Result<bool> {
        let mut control = self.controls.lock().unwrap();
        control.saves += 1;
        if control.fail_save {
            return Err(io("Synthetic disk full"));
        }
        if control.external_write {
            control.external_write = false;
            let mut other = self.stored();
            booknook_core::apply(
                &mut other,
                serde_json::from_str::<Command>(&command("External writer 🌿")).unwrap(),
            )?;
            self.write(&other)?;
        }
        if self.stored().revision != expected_revision {
            return Ok(false);
        }
        let state = booknook_core::strict_parse(state_json.as_bytes())?;
        self.write(&state)?;
        Ok(true)
    }
}

#[test]
fn alternate_persisted_host_and_local_sqlite_reopen() {
    let host = FileHost::new();
    let notebook = NotebookEngine::with_storage(Box::new(host.clone())).unwrap();
    let response: serde_json::Value = serde_json::from_str(
        &notebook
            .execute(command("Private Unicode 🌿 中文"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(response["state"]["revision"], 1);
    assert_eq!(
        host.stored().books.values().next().unwrap().title,
        "Private Unicode 🌿 中文"
    );
    drop(notebook);
    let reopened = NotebookEngine::with_storage(Box::new(host.clone())).unwrap();
    assert_eq!(
        reopened.snapshot().unwrap(),
        serde_json::to_string(&host.stored()).unwrap()
    );
    let local =
        NotebookEngine::local(host.directory.join("local").to_string_lossy().into()).unwrap();
    local.execute(command("Separate SQLite")).unwrap();
    let saved = local.snapshot().unwrap();
    drop(local);
    assert_eq!(
        NotebookEngine::local(host.directory.join("local").to_string_lossy().into())
            .unwrap()
            .snapshot()
            .unwrap(),
        saved
    );
    assert_eq!(host.stored().books.len(), 1);
}

#[test]
fn rejected_cas_retains_external_writer_and_requires_fresh_command() {
    let host = FileHost::new();
    let notebook = NotebookEngine::with_storage(Box::new(host.clone())).unwrap();
    host.controls.lock().unwrap().external_write = true;
    let error = notebook.execute(command("Must not win")).unwrap_err();
    assert!(error.to_string().contains("Conflict"));
    let external = host.stored();
    assert_eq!(external.revision, 1);
    assert_eq!(external.books.len(), 1);
    assert_eq!(
        external.books.values().next().unwrap().title,
        "External writer 🌿"
    );
    assert_eq!(
        notebook.snapshot().unwrap(),
        serde_json::to_string(&external).unwrap()
    );
    assert_eq!(host.controls.lock().unwrap().saves, 1, "No automatic retry");
    drop(notebook);
    let reopened = NotebookEngine::with_storage(Box::new(host.clone())).unwrap();
    reopened.execute(command("Explicit fresh command")).unwrap();
    assert_eq!(host.stored().revision, 2);
    assert_eq!(host.stored().books.len(), 2);
}

#[test]
fn storage_failure_and_changed_availability_cannot_report_success() {
    let host = FileHost::new();
    let notebook = NotebookEngine::with_storage(Box::new(host.clone())).unwrap();
    let original = notebook.snapshot().unwrap();
    host.controls.lock().unwrap().fail_save = true;
    assert!(
        notebook
            .execute(command("Fail"))
            .unwrap_err()
            .to_string()
            .contains("Synthetic disk full")
    );
    host.controls.lock().unwrap().fail_save = false;
    assert_eq!(notebook.snapshot().unwrap(), original);
    host.controls.lock().unwrap().fail_descriptor = true;
    assert!(
        notebook
            .snapshot()
            .unwrap_err()
            .to_string()
            .contains("descriptor I/O failure")
    );
    host.controls.lock().unwrap().fail_descriptor = false;
    host.controls.lock().unwrap().fail_load = true;
    assert!(
        notebook
            .execute(command("Read failed"))
            .unwrap_err()
            .to_string()
            .contains("read I/O failure")
    );
    host.controls.lock().unwrap().fail_load = false;
    assert_eq!(notebook.snapshot().unwrap(), original);
    for changed in [
        descriptor("example.file", false, &["notebook_storage"]),
        descriptor("example.file", true, &["catalog"]),
        descriptor("different.store", true, &["notebook_storage"]),
    ] {
        host.controls.lock().unwrap().descriptor_after_load = Some(changed);
        assert!(notebook.execute(command("Unavailable")).is_err());
        assert_eq!(host.stored().revision, 0);
        assert_eq!(
            host.controls.lock().unwrap().saves,
            1,
            "Unavailable host must not reach CAS"
        );
        host.controls.lock().unwrap().descriptor =
            descriptor("example.file", true, &["notebook_storage"]);
    }
    host.controls.lock().unwrap().descriptor =
        descriptor("example.file", false, &["notebook_storage"]);
    assert!(
        notebook.snapshot().is_err(),
        "Availability is reread even after opening"
    );
    assert!(NotebookEngine::with_storage(Box::new(host.clone())).is_err());
    host.controls.lock().unwrap().descriptor = descriptor("example.file", true, &["catalog"]);
    assert!(NotebookEngine::with_storage(Box::new(host.clone())).is_err());
}

#[test]
fn foreign_json_is_strictly_parsed_and_semantically_validated() {
    let host = FileHost::new();
    let notebook = NotebookEngine::with_storage(Box::new(host.clone())).unwrap();
    for raw in [
        r#"{"id":"example.file","id":"other","capabilities":["notebook_storage"],"available":true}"#.to_string(),
        r#"{"id":"example.file","capabilities":["notebook_storage"],"available":true,"extra":1}"#.to_string(),
        descriptor("", true, &["notebook_storage"]),
        descriptor("example.file", true, &["notebook_storage","notebook_storage"]),
    ] {
        host.controls.lock().unwrap().descriptor = raw;
        assert!(notebook.snapshot().is_err());
    }
    host.controls.lock().unwrap().descriptor =
        " ".repeat(16 * 1024) + &descriptor("example.file", true, &["notebook_storage"]);
    assert!(
        notebook
            .snapshot()
            .unwrap_err()
            .to_string()
            .contains("exceeds 16 KiB")
    );
    host.controls.lock().unwrap().descriptor =
        descriptor("example.file", true, &["notebook_storage"]);
    let mut invalid_state = serde_json::to_value(host.stored()).unwrap();
    invalid_state["schema"] = "booknook.archive.invalid".into();
    for raw in [
        serde_json::to_string(&host.stored())
            .unwrap()
            .replacen("{", "{\"revision\":0,", 1),
        invalid_state.to_string(),
    ] {
        host.controls.lock().unwrap().raw_override = Some(raw);
        assert!(notebook.snapshot().is_err());
        assert!(notebook.execute(command("Rejected state")).is_err());
    }
    host.controls.lock().unwrap().raw_override = None;
    assert!(notebook.execute(r#"{"op":"add_book","op":"add_book","title":"Duplicate","author":"Fixture","at":1}"#.into()).is_err());
    assert_eq!(host.controls.lock().unwrap().saves, 0);
    assert_eq!(host.stored().revision, 0);
}
