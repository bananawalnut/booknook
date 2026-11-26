// SPDX-License-Identifier: MIT OR Apache-2.0
use super::*;
use crate::Desk;
use std::{os::unix::process::ExitStatusExt, process::Command};

const CHILD: &str = "BOOKNOOK_RESTORE_CRASH_TEST";
pub(super) fn interrupt_at(stage: &str) {
    if std::env::var(CHILD).as_deref() == Ok(stage) {
        // Test subprocess only: skip Rust destructors and SQLite connection close.
        unsafe { libc::kill(libc::getpid(), libc::SIGKILL) };
        unreachable!();
    }
}

fn desk(root: &Path) -> Desk {
    Desk::new(root.to_str().unwrap().into()).unwrap()
}

#[test]
fn process_death_at_restore_commit_boundary() {
    if std::env::var_os(CHILD).is_some() {
        let root =
            std::path::PathBuf::from(std::env::var_os("BOOKNOOK_RESTORE_TEST_ROOT").unwrap());
        desk(&root.join("target"))
            .restore_archive(root.join("backup.json").to_str().unwrap().into())
            .unwrap();
        panic!("expected test process interruption");
    }
    let root = std::env::temp_dir().join(format!(
        "booknook-restore-crash-{}",
        booknook_core::new_id()
    ));
    let source = desk(&root.join("source"));
    source
        .execute(r#"{"op":"add_book","title":"A book","author":"Fixture","at":1}"#.into())
        .unwrap();
    let state: booknook_core::model::State =
        serde_json::from_str(&source.snapshot().unwrap()).unwrap();
    let epub = root.join("fixture.epub");
    let bytes = include_bytes!("../tests/fixtures/restore-sample.epub");
    fs::write(&epub, bytes).unwrap();
    source
        .import_content(
            state.books.keys().next().unwrap().clone(),
            epub.to_str().unwrap().into(),
        )
        .unwrap();
    let backup = root.join("backup.json").to_str().unwrap().to_owned();
    source.export_archive(backup).unwrap();
    let target = root.join("target");
    let asset = target.join("assets").join(booknook_core::digest(bytes));
    for stage in ["before-commit", "after-commit"] {
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "restore::tests::process_death_at_restore_commit_boundary",
            ])
            .env(CHILD, stage)
            .env("BOOKNOOK_RESTORE_TEST_ROOT", &root)
            .status()
            .unwrap();
        assert_eq!(status.signal(), Some(libc::SIGKILL));
        assert!(target.join("assets").join(PENDING).join(PLAN).is_file());
        assert_eq!(fs::read(asset.join("original.epub")).unwrap(), bytes);
        let reopened = desk(&target);
        let state: booknook_core::model::State =
            serde_json::from_str(&reopened.snapshot().unwrap()).unwrap();
        let committed = stage == "after-commit";
        assert_eq!(state.books.len(), usize::from(committed));
        assert_eq!(asset.exists(), committed);
        assert!(
            !target.join("assets").join(PENDING).exists()
                && !target.join("assets").join(WORK).exists()
        );
    }
    drop(source);
    fs::remove_dir_all(root).unwrap();
}

// An ordinary rename error during a multi-asset promotion must remove the
// already-published directory without deleting the obstruction at the next ID.
#[test]
fn partial_promotion_error_reconciles_only_owned_assets() {
    let root = std::env::temp_dir().join(format!(
        "booknook-restore-publish-{}",
        booknook_core::new_id()
    ));
    let mut store = Store::open(&root).unwrap();
    let pending = root.join("assets").join(PENDING);
    let first = "a".repeat(64);
    let second = "b".repeat(64);
    let plan = Plan::from([
        (first.clone(), Change::NewAsset),
        (second.clone(), Change::NewAsset),
    ]);
    for id in [&first, &second] {
        fs::create_dir_all(pending.join("assets").join(id)).unwrap();
        fs::write(
            pending.join("assets").join(id).join("original.epub"),
            b"prepared",
        )
        .unwrap();
    }
    fs::create_dir(pending.join(OWNER)).unwrap();
    fs::create_dir(pending.join("previous")).unwrap();
    fs::write(pending.join(PLAN), serde_json::to_vec(&plan).unwrap()).unwrap();
    fs::write(
        root.join("assets").join(&second),
        b"pre-existing obstruction",
    )
    .unwrap();
    assert!(publish(&root, &plan).is_err());
    assert!(root.join("assets").join(&first).is_dir());
    recover(&mut store).unwrap();
    assert!(!root.join("assets").join(first).exists());
    assert_eq!(
        fs::read(root.join("assets").join(second)).unwrap(),
        b"pre-existing obstruction"
    );
    drop(store);
    fs::remove_dir_all(root).unwrap();
}
