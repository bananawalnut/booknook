// SPDX-License-Identifier: MIT OR Apache-2.0
//! Coordinate the portable restore's SQLite commit with its content-addressed files.
//! Only an empty library can be restored. Committed references therefore tell recovery
//! exactly which prepared additions to keep, without changing the public archive or DB.
use crate::{NativeError, PortableArchive, Result, content, social_bridge, store::Store};
use base64::{Engine, engine::general_purpose::STANDARD};
use booknook_core::model::{ContentAsset, MOBI6_MEDIA_TYPE};
use rusqlite::{Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{ErrorKind, Write},
    path::Path,
};

// Both names reside inside the asset directory so publication and retirement
// remain same-volume renames, including separately mounted asset storage.
const WORK: &str = ".booknook-restore-work";
const PENDING: &str = ".booknook-restore-pending";
const PLAN: &str = "undo.json";
const OWNER: &str = "booknook-restore-v1";

#[derive(Serialize, Deserialize)]
enum Change {
    NewAsset,
    NewMobiReaderDirectory,
    NewMobiReader,
    ReplaceMobiReader,
}
type Plan = BTreeMap<String, Change>;

fn error(message: &str) -> NativeError {
    NativeError::Failure {
        message: message.to_owned(),
    }
}

fn sync_dir(path: &Path) -> Result<()> {
    // POSIX permits opening directories for fsync. Other targets retain process
    // interruption recovery without relying on a directory handle API.
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn sync_tree(path: &Path) -> Result<()> {
    for item in fs::read_dir(path)? {
        let item = item?;
        if item.file_type()?.is_dir() {
            sync_tree(&item.path())?;
        } else {
            #[cfg(unix)]
            File::open(item.path())?.sync_all()?;
            #[cfg(not(unix))]
            fs::OpenOptions::new()
                .write(true)
                .open(item.path())?
                .sync_all()?;
        }
    }
    sync_dir(path)
}

// Existing EPUB/PDF files are reused, never replaced. MOBI alone allows
// regeneration of an absent/stale derivative; retain its exact before-image.
fn plan_asset(
    root: &Path,
    work: &Path,
    asset: &ContentAsset,
    bytes: &[u8],
) -> Result<Option<Change>> {
    let live = root.join("assets").join(&asset.id);
    match fs::symlink_metadata(&live) {
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Some(Change::NewAsset)),
        Err(err) => return Err(err.into()),
        Ok(meta) if !meta.is_dir() => return Err(error("Stored asset is not a directory")),
        _ => {}
    }
    let kind = content::ContentKind::from_media_type(&asset.media_type)?;
    if !content::file_matches(&live.join(kind.original_name()), bytes)? {
        return Err(error("Stored asset original differs from the backup"));
    }
    if asset.media_type != MOBI6_MEDIA_TYPE {
        if asset.spine.iter().any(|path| !live.join(path).is_file()) {
            return Err(error("Stored book files do not match this content format"));
        }
        return Ok(None);
    }
    let reader = live.join(content::MOBI6_SPINE);
    let parent = reader.parent().unwrap();
    match fs::symlink_metadata(parent) {
        Err(err) if err.kind() == ErrorKind::NotFound => {
            return Ok(Some(Change::NewMobiReaderDirectory));
        }
        Err(err) => return Err(err.into()),
        Ok(meta) if !meta.is_dir() => {
            return Err(error("Stored MOBI reader path is not a directory"));
        }
        _ => {}
    }
    let fresh = work
        .join("assets")
        .join(&asset.id)
        .join(content::MOBI6_SPINE);
    if content::file_matches(&reader, &fs::read(fresh)?)? {
        return Ok(None);
    }
    if !reader.try_exists()? {
        return Ok(Some(Change::NewMobiReader));
    }
    // A link preserves the old inode across rollback. Volumes without hard
    // links still retain the prior file contents through a private copy.
    let backup = work.join("previous").join(&asset.id);
    if fs::hard_link(&reader, &backup).is_err() {
        fs::copy(reader, backup)?;
    }
    Ok(Some(Change::ReplaceMobiReader))
}

fn prepare(root: &Path, archive: &PortableArchive) -> Result<Plan> {
    let work = root.join("assets").join(WORK);
    fs::create_dir(&work)?;
    fs::create_dir(work.join(OWNER))?;
    fs::create_dir(work.join("previous"))?;
    fs::create_dir(work.join("assets"))?;
    let mut plan = Plan::new();
    let mut prepared = BTreeMap::new();
    for expected in archive
        .state
        .editions
        .values()
        .flat_map(|edition| &edition.assets)
    {
        if let Some(actual) = prepared.get(&expected.id) {
            if actual != expected {
                return Err(error("Export asset metadata mismatch"));
            }
            continue;
        }
        let bytes = STANDARD
            .decode(
                archive
                    .assets
                    .get(&expected.id)
                    .ok_or_else(|| error("Export is missing an asset"))?,
            )
            .map_err(|_| error("Invalid export asset encoding"))?;
        let kind = content::ContentKind::from_media_type(&expected.media_type)?;
        let actual = content::import(&work, &bytes, kind)?;
        if actual != *expected {
            return Err(error("Export asset metadata mismatch"));
        }
        if let Some(change) = plan_asset(root, &work, &actual, &bytes)? {
            plan.insert(actual.id.clone(), change);
        }
        prepared.insert(actual.id.clone(), actual);
    }
    let mut file = File::create(work.join(PLAN))?;
    file.write_all(&serde_json::to_vec(&plan)?)?;
    sync_tree(&work)?;
    fs::rename(&work, root.join("assets").join(PENDING))?;
    sync_dir(&root.join("assets"))?;
    Ok(plan)
}

fn publish(root: &Path, plan: &Plan) -> Result<()> {
    let staging = root.join("assets").join(PENDING).join("assets");
    let assets = root.join("assets");
    for (id, change) in plan {
        let live = assets.join(id);
        let src = staging.join(id);
        match change {
            Change::NewAsset => fs::rename(src, live)?,
            _ => {
                let reader = live.join(content::MOBI6_SPINE);
                let parent = reader.parent().unwrap();
                if matches!(change, Change::NewMobiReaderDirectory) {
                    fs::create_dir(parent)?;
                    sync_dir(&live)?;
                }
                fs::rename(src.join(content::MOBI6_SPINE), &reader)?;
                sync_dir(parent)?;
            }
        }
    }
    sync_dir(&assets)
}

fn remove_if_present(path: &Path, remove: impl FnOnce(&Path) -> std::io::Result<()>) -> Result<()> {
    match remove(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

// Keep the ownership marker until last. A kill during cleanup leaves either a
// recognizable scratch directory or an empty directory. Never remove unrelated
// files if an existing desk already uses one of these names.
fn discard_work(work: &Path) -> Result<()> {
    match fs::symlink_metadata(work) {
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err.into()),
        Ok(meta) if !meta.is_dir() => return Err(error("Restore scratch path is not a directory")),
        _ => {}
    }
    let mut entries = fs::read_dir(work)?;
    if entries.next().transpose()?.is_none() {
        fs::remove_dir(work)?;
        return Ok(());
    }
    require_owned(work, false)?;
    remove_if_present(&work.join("assets"), |p| fs::remove_dir_all(p))?;
    remove_if_present(&work.join("previous"), |p| fs::remove_dir_all(p))?;
    remove_if_present(&work.join(PLAN), |p| fs::remove_file(p))?;
    fs::remove_dir(work.join(OWNER))?;
    fs::remove_dir(work)?;
    Ok(())
}

fn check_type(path: &Path, directory: bool, allow_missing: bool) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(meta) if (directory && meta.is_dir()) || (!directory && meta.is_file()) => Ok(true),
        Err(err) if allow_missing && err.kind() == ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err.into()),
        Ok(_) => Err(error("Unexpected file type in restore journal")),
    }
}

fn require_owned(path: &Path, complete: bool) -> Result<()> {
    check_type(path, true, false)?;
    check_type(&path.join(OWNER), true, false)?;
    check_type(&path.join("assets"), true, !complete)?;
    check_type(&path.join("previous"), true, !complete)?;
    check_type(&path.join(PLAN), false, !complete)?;
    for entry in fs::read_dir(path)? {
        if !matches!(
            entry?.file_name().to_str(),
            Some("assets" | "previous" | PLAN | OWNER)
        ) {
            return Err(error("Unrecognized restore scratch directory"));
        }
    }
    Ok(())
}

fn validate_undo(root: &Path, pending: &Path, plan: &Plan) -> Result<()> {
    for (id, change) in plan {
        if id.len() != 64
            || !id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(error("Invalid restore journal asset ID"));
        }
        let staged = pending.join("assets").join(id);
        let live = root.join("assets").join(id);
        if matches!(change, Change::NewAsset) {
            if !check_type(&staged, true, true)? {
                check_type(&live, true, true)?;
            }
        } else {
            check_type(&staged, true, false)?;
            check_type(&live, true, false)?;
            let reader = live.join(content::MOBI6_SPINE);
            let directory_can_be_absent = matches!(change, Change::NewMobiReaderDirectory);
            if check_type(reader.parent().unwrap(), true, directory_can_be_absent)? {
                check_type(&reader, false, true)?;
            }
            if matches!(change, Change::ReplaceMobiReader) {
                check_type(&pending.join("previous").join(id), false, true)?;
            }
        }
    }
    Ok(())
}

// Hold SQLite's writer lock while interpreting the journal and changing files.
// Moving PENDING back to WORK makes interruption of journal deletion harmless.
pub(crate) fn recover_locked(root: &Path, tx: &Transaction<'_>) -> Result<()> {
    let pending = root.join("assets").join(PENDING);
    let work = root.join("assets").join(WORK);
    if pending.try_exists()? {
        require_owned(&pending, true)?;
        let state = Store::load_from(tx)?;
        let keep: BTreeSet<_> = state
            .editions
            .values()
            .flat_map(|e| &e.assets)
            .map(|a| &a.id)
            .collect();
        let plan: Plan =
            serde_json::from_slice(&crate::portable::read_bounded(&pending.join(PLAN))?)?;
        validate_undo(root, &pending, &plan)?;
        for (id, change) in plan {
            if keep.contains(&id) {
                continue;
            }
            let assets = root.join("assets");
            let live = assets.join(&id);
            let reader = live.join(content::MOBI6_SPINE);
            let parent = reader.parent().unwrap();
            match change {
                Change::NewAsset => {
                    if !pending.join("assets").join(&id).try_exists()? {
                        remove_if_present(&live, |p| fs::remove_dir_all(p))?;
                    }
                    sync_dir(&assets)?;
                }
                Change::ReplaceMobiReader => {
                    let backup = pending.join("previous").join(&id);
                    if !pending
                        .join("assets")
                        .join(&id)
                        .join(content::MOBI6_SPINE)
                        .try_exists()?
                        && backup.try_exists()?
                    {
                        fs::rename(backup, &reader)?;
                    }
                    sync_dir(parent)?;
                }
                Change::NewMobiReader | Change::NewMobiReaderDirectory => {
                    remove_if_present(&reader, |p| fs::remove_file(p))?;
                    if matches!(change, Change::NewMobiReaderDirectory) {
                        remove_if_present(parent, |p| fs::remove_dir(p))?;
                        sync_dir(&live)?;
                    } else {
                        sync_dir(parent)?;
                    }
                }
            }
        }
        fs::rename(pending, &work)?;
        sync_dir(&root.join("assets"))?;
    }
    discard_work(&work)
}

pub(crate) fn recover(store: &mut Store) -> Result<()> {
    if !store.root.join("assets").join(PENDING).try_exists()?
        && !store.root.join("assets").join(WORK).try_exists()?
    {
        return Ok(());
    }
    let tx = store
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    recover_locked(&store.root, &tx)?;
    tx.commit()?;
    Ok(())
}

pub(crate) fn archive(store: &mut Store, mut archive: PortableArchive) -> Result<String> {
    let root = &store.root;
    let tx = store
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    recover_locked(root, &tx)?;
    let current = Store::load_from(&tx)?;
    if current.revision != 0 || !current.books.is_empty() {
        return Err(error(
            "Restore requires an empty desk to protect existing work",
        ));
    }
    if let Some(social) = &archive.social {
        social_bridge::write_archive_bundle(&tx, social)?;
    }
    archive.state.revision = 1;
    let json = serde_json::to_string(&archive.state)?;
    if json.len() > booknook_core::MAX_JSON_BYTES {
        return Err(error("Restored state exceeds 128 MiB"));
    }
    let changed = tx.execute(
        "UPDATE desk SET revision=1,json=?1 WHERE singleton=1 AND revision=0",
        params![json],
    )?;
    if changed != 1 {
        return Err(error("Desk changed during restore"));
    }
    // No live file is changed until all domain writes and decoded content pass.
    let has_assets = archive
        .state
        .editions
        .values()
        .any(|e| !e.assets.is_empty());
    if !has_assets {
        tx.commit()?;
        return Ok(json);
    }
    let result = (|| {
        let plan = prepare(root, &archive)?;
        publish(root, &plan)?;
        #[cfg(all(test, unix))]
        tests::interrupt_at("before-commit");
        tx.commit()?;
        #[cfg(all(test, unix))]
        tests::interrupt_at("after-commit");
        Ok(())
    })();
    // On success a cleanup failure is deferred: SQLite has already committed.
    // On error recovery runs against the rolled-back SQLite state.
    let cleanup = recover(store);
    if let Err(failure) = result {
        cleanup?;
        return Err(failure);
    }
    Ok(json)
}

#[cfg(all(test, unix))]
#[path = "restore_tests.rs"]
mod tests;
