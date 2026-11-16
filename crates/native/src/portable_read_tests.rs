// SPDX-License-Identifier: MIT OR Apache-2.0
use super::read_bounded;
use crate::Desk;
use booknook_core::settings::{CastaliaSettings, ConnectionSettings};
use std::{
    ffi::CString,
    fs,
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, symlink},
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("booknook-portable-{}", booknook_core::new_id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn regular_error<T: std::fmt::Debug>(result: crate::Result<T>) {
    assert_eq!(
        result.unwrap_err().to_string(),
        "Portable input must be a regular file"
    );
}

#[test]
fn regular_file_and_symlink_preserve_exact_bytes() {
    let root = Directory::new();
    let file = root.0.join("original.json");
    let link = root.0.join("selected.json");
    let bytes = "Café — 読書 📚\0\r\nexact original bytes".as_bytes();
    fs::write(&file, bytes).unwrap();
    symlink(&file, &link).unwrap();
    assert_eq!(read_bounded(&file).unwrap(), bytes);
    assert_eq!(read_bounded(&link).unwrap(), bytes);
    fs::write(&file, []).unwrap();
    assert!(read_bounded(&link).unwrap().is_empty());
}

#[test]
fn directory_is_rejected_as_nonregular() {
    let root = Directory::new();
    regular_error(read_bounded(&root.0));
}

#[test]
fn sparse_oversized_file_is_rejected_before_reading() {
    let root = Directory::new();
    let path = root.0.join("oversized.json");
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    // Extend only the logical length; never allocate a 128 MiB content buffer or write it.
    file.set_len(booknook_core::MAX_JSON_BYTES as u64 + 1)
        .unwrap();
    assert!(file.metadata().unwrap().blocks() * 512 <= 64 * 1024);
    assert_eq!(
        read_bounded(&path).unwrap_err().to_string(),
        "Portable file exceeds the development build's 128 MiB limit"
    );
}

// A regression to blocking File::open must fail the parent, not hang the whole test binary.
struct OwnedChild(Option<Child>);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn fifo_and_fifo_symlink_are_rejected_without_a_writer() {
    const CHILD_ROOT: &str = "BOOKNOOK_TEST_PORTABLE_FIFO_CHILD_ROOT";
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let root = PathBuf::from(root);
        fs::write(root.join("child-entered"), b"entered file boundary").unwrap();
        let fifo = root.join("input.json");
        let link = root.join("linked.json");
        regular_error(read_bounded(&fifo));
        regular_error(read_bounded(&link));
        rejected_inputs_preserve_native_state(&root, [&fifo, &link]);
        return;
    }
    let root = Directory::new();
    let fifo = root.0.join("input.json");
    let cpath = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    // SAFETY: cpath is a live, NUL-terminated path; mkfifo does not retain its pointer.
    assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
    symlink(&fifo, root.0.join("linked.json")).unwrap();
    let test_name = format!(
        "{}::fifo_and_fifo_symlink_are_rejected_without_a_writer",
        module_path!().split_once("::").unwrap().1
    );
    let mut child = OwnedChild(Some(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &test_name, "--nocapture"])
            .env(CHILD_ROOT, &root.0)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    ));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if child.0.as_mut().unwrap().try_wait().unwrap().is_some() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Portable FIFO reader exceeded watchdog; owned child is killed/reaped, never detached"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.0.take().unwrap().wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "FIFO child failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(root.0.join("child-entered").is_file());
}

fn rejected_inputs_preserve_native_state(root: &Path, inputs: [&Path; 2]) {
    let desk_path = root.join("desk");
    let open = || Desk::new(desk_path.to_string_lossy().into_owned()).unwrap();
    let desk = open();
    desk.execute(
        serde_json::json!({"op":"add_book","title":"Keep Café 読書 📚","author":"Reader","at":10})
            .to_string(),
    )
    .unwrap();
    // Synthetic public configuration reaches sync's file boundary. No receipt, grant,
    // signing key, network request, pairing, or accepted sync operation is created.
    let settings = ConnectionSettings {
        castalia: Some(CastaliaSettings {
            issuer: "portable-test".into(),
            key_id: "test-key".into(),
            issuer_public_key: "11".repeat(32),
            audience: "booknook".into(),
            terminal_caller: "portable-test".into(),
            holder_public_key: "22".repeat(32),
            policy_digest: "33".repeat(32),
            credential_endpoint: String::new(),
        }),
        ..Default::default()
    };
    desk.save_connection_settings(serde_json::to_string(&settings).unwrap())
        .unwrap();
    let snapshot = desk.snapshot().unwrap();
    let state: booknook_core::model::State = serde_json::from_str(&snapshot).unwrap();
    let book_id = state.books.keys().next().unwrap().clone();
    let before = (
        snapshot,
        desk.connection_settings().unwrap(),
        desk.social_snapshot(false).unwrap(),
        desk.sync_status().unwrap(),
    );
    for path in inputs {
        let path = path.to_string_lossy().into_owned();
        regular_error(desk.restore_archive(path.clone()));
        regular_error(desk.social_restore(path.clone()));
        regular_error(desk.sync_receive(path.clone(), "synthetic-peer".into(), "44".repeat(32)));
        regular_error(desk.import_content(book_id.clone(), path));
    }
    assert_eq!(desk.snapshot().unwrap(), before.0);
    assert_eq!(desk.connection_settings().unwrap(), before.1);
    assert_eq!(desk.social_snapshot(false).unwrap(), before.2);
    assert_eq!(desk.sync_status().unwrap(), before.3);
    drop(desk);
    let reopened = open();
    assert_eq!(reopened.snapshot().unwrap(), before.0);
    assert_eq!(reopened.connection_settings().unwrap(), before.1);
    assert_eq!(reopened.social_snapshot(false).unwrap(), before.2);
    assert_eq!(reopened.sync_status().unwrap(), before.3);
}
