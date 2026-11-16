// SPDX-License-Identifier: MIT OR Apache-2.0
use super::{Store, ensure_private_permissions};
use booknook_core::model::State;
use std::{
    fs::{self, OpenOptions, Permissions},
    io,
    os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink},
    path::{Path, PathBuf},
};

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "booknook-store-permissions-{}",
            booknook_core::new_id()
        )))
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o7777
}
fn chmod(path: &Path, mode: u32) {
    fs::set_permissions(path, Permissions::from_mode(mode)).unwrap();
}
fn note_fixture(root: &Path) -> State {
    let mut store = Store::open(root).unwrap();
    store
        .execute(
            serde_json::from_value(serde_json::json!({
                "op":"add_book", "title":"Permission fixture", "author":"Synthetic", "at":1
            }))
            .unwrap(),
        )
        .unwrap();
    for (at, text) in [(2, "Earlier thought"), (3, "Café — 読書 📚")] {
        let state = store.load().unwrap();
        let notebook = state.notebooks.values().next().unwrap();
        let block = &notebook.blocks[0];
        store
            .execute(
                serde_json::from_value(serde_json::json!({
                    "op":"edit_block", "notebook_id":notebook.id, "block_id":block.id,
                    "base_update":block.update, "text":text, "at":at
                }))
                .unwrap(),
            )
            .unwrap();
    }
    store.load().unwrap()
}

#[test]
fn private_reopen_skips_root_repair_and_preserves_note_history() {
    let root = Root::new();
    let before = note_fixture(&root.0);
    assert_eq!(mode(&root.0), 0o700);
    assert_eq!(mode(&root.0.join("booknook.sqlite")), 0o600);
    // Exercise the actual Store startup with a deterministic syscall boundary:
    // any attempted root permission write is an error, regardless of its speed.
    let reopened = Store::open_with_root_permissions(&root.0, |_, _| {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "redundant root permission mutation",
        ))
    })
    .unwrap();
    assert_eq!(reopened.load().unwrap(), before);
    let notebook = before.notebooks.values().next().unwrap();
    assert_eq!(notebook.blocks[0].text, "Café — 読書 📚");
    assert_eq!(notebook.revisions.len(), 2);
}

#[test]
fn missing_parent_directories_and_database_start_private() {
    let root = Root::new();
    let nested = root.0.join("parent/desk");
    let store = Store::open_with_root_permissions(&nested, |_, _| {
        Err(io::Error::other("new root must already be private"))
    })
    .unwrap();
    assert_eq!(mode(&root.0), 0o700);
    assert_eq!(mode(&root.0.join("parent")), 0o700);
    assert_eq!(mode(&nested), 0o700);
    assert_eq!(mode(&nested.join("assets")), 0o700);
    assert_eq!(mode(&nested.join("booknook.sqlite")), 0o600);
    assert!(store.load().unwrap().books.is_empty());
}

#[test]
fn loose_existing_modes_are_repaired_without_changing_saved_state() {
    let root = Root::new();
    let before = note_fixture(&root.0);
    chmod(&root.0, 0o755);
    chmod(&root.0.join("booknook.sqlite"), 0o644);
    let mut repairs = 0;
    let reopened = Store::open_with_root_permissions(&root.0, |path, permissions| {
        repairs += 1;
        fs::set_permissions(path, permissions)
    })
    .unwrap();
    assert_eq!(repairs, 1);
    assert_eq!(mode(&root.0), 0o700);
    assert_eq!(mode(&root.0.join("booknook.sqlite")), 0o600);
    assert_eq!(reopened.load().unwrap(), before);
}

#[test]
fn denied_root_repair_fails_before_database_creation() {
    let root = Root::new();
    fs::create_dir_all(&root.0).unwrap();
    chmod(&root.0, 0o755);
    let mut repairs = 0;
    let error = Store::open_with_root_permissions(&root.0, |_, _| {
        repairs += 1;
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "synthetic denied permission repair",
        ))
    })
    .err()
    .expect("a denied permission change must not be ignored");
    assert!(
        error
            .to_string()
            .contains("synthetic denied permission repair")
    );
    assert_eq!(repairs, 1);
    assert_eq!(mode(&root.0), 0o755);
    assert!(!root.0.join("booknook.sqlite").exists());
}

#[test]
fn ineffective_root_repair_fails_before_database_creation() {
    let root = Root::new();
    fs::create_dir_all(&root.0).unwrap();
    chmod(&root.0, 0o755);
    let error = Store::open_with_root_permissions(&root.0, |_, _| Ok(()))
        .err()
        .expect("a reported success must be verified against actual metadata");
    assert!(error.to_string().contains("could not be verified"));
    assert!(!root.0.join("booknook.sqlite").exists());
}

#[test]
fn descriptor_guard_skips_exact_mode_and_rejects_denied_or_ineffective_repair() {
    let root = Root::new();
    fs::create_dir_all(&root.0).unwrap();
    let path = root.0.join("fixture");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .unwrap();
    ensure_private_permissions(
        0o600,
        || file.metadata().map(|metadata| metadata.permissions()),
        |_| panic!("an exact private file mode must not call fchmod"),
    )
    .unwrap();
    for loose in [0o644, 0o1600] {
        file.set_permissions(Permissions::from_mode(loose)).unwrap();
        assert_eq!(mode(&path), loose);
        let mut attempted = false;
        let error = ensure_private_permissions(
            0o600,
            || file.metadata().map(|metadata| metadata.permissions()),
            |_| {
                attempted = true;
                Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"))
            },
        )
        .unwrap_err();
        assert!(attempted, "special mode bits must not be masked away");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(mode(&path), loose);
        assert!(
            ensure_private_permissions(
                0o600,
                || file.metadata().map(|metadata| metadata.permissions()),
                |_| Ok(()),
            )
            .is_err()
        );
        ensure_private_permissions(
            0o600,
            || file.metadata().map(|metadata| metadata.permissions()),
            |permissions| file.set_permissions(permissions),
        )
        .unwrap();
        assert_eq!(mode(&path), 0o600);
    }
}

#[test]
fn existing_directory_and_regular_database_symlinks_remain_supported() {
    let root = Root::new();
    let actual = root.0.join("actual");
    let before = note_fixture(&actual);
    let moved = root.0.join("retained.sqlite");
    fs::rename(actual.join("booknook.sqlite"), &moved).unwrap();
    symlink(&moved, actual.join("booknook.sqlite")).unwrap();
    let alias = root.0.join("alias");
    symlink(&actual, &alias).unwrap();
    chmod(&actual, 0o755);
    chmod(&moved, 0o644);
    let reopened = Store::open(&alias).unwrap();
    assert_eq!(reopened.load().unwrap(), before);
    assert!(
        fs::symlink_metadata(&alias)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        fs::symlink_metadata(actual.join("booknook.sqlite"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(mode(&actual), 0o700);
    assert_eq!(mode(&moved), 0o600);
}

#[test]
fn directory_repair_preserves_owner_write_search_only_compatibility() {
    let root = Root::new();
    fs::create_dir_all(root.0.join("assets")).unwrap();
    chmod(&root.0, 0o300);
    let store = Store::open(&root.0).unwrap();
    assert_eq!(mode(&root.0), 0o700);
    assert!(store.load().unwrap().books.is_empty());
}
