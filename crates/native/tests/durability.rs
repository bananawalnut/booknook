use booknook_core::{Command, model::*};
use booknook_native::Desk;
use std::path::PathBuf;
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("booknook-test-{}", booknook_core::new_id()));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
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
fn execute(d: &Desk, c: Command) {
    d.execute(serde_json::to_string(&c).unwrap()).unwrap();
}
fn state(d: &Desk) -> State {
    serde_json::from_str(&d.snapshot().unwrap()).unwrap()
}
fn add(d: &Desk) {
    execute(
        d,
        Command::AddBook {
            title: "The Quiet Library".into(),
            author: "Book Nook fixtures".into(),
            at: 1,
        },
    );
}

#[test]
fn committed_edits_survive_reopen_and_portable_roundtrip() {
    let a = Temp::new();
    let d = a.desk();
    add(&d);
    let n = state(&d).notebooks.values().next().unwrap().clone();
    execute(
        &d,
        Command::EditBlock {
            capture_context: None,
            note_creation: None,
            notebook_id: n.id.clone(),
            block_id: n.blocks[0].id.clone(),
            base_update: n.blocks[0].update.clone(),
            text: "Saved 🌿\nA second line.".into(),
            at: 2,
        },
    );
    execute(
        &d,
        Command::AddCharacter {
            capture_context: None,
            note_creation: None,
            notebook_id: n.id,
            block_id: n.blocks[0].id.clone(),
            name: "Mara".into(),
            at: 3,
        },
    );
    let before = state(&d);
    drop(d);
    let reopened = a.desk();
    assert_eq!(state(&reopened), before);
    let export = a.0.join("export.json");
    reopened
        .export_archive(export.to_string_lossy().into())
        .unwrap();
    let b = Temp::new();
    let restored = b.desk();
    restored
        .restore_archive(export.to_string_lossy().into())
        .unwrap();
    let mut after = state(&restored);
    after.revision = before.revision;
    assert_eq!(before, after);
    assert!(
        reopened
            .restore_archive(export.to_string_lossy().into())
            .is_err()
    );
}
#[test]
fn independent_handles_do_not_lose_serial_commits() {
    let t = Temp::new();
    let a = t.desk();
    let b = t.desk();
    add(&a);
    add(&b);
    assert_eq!(state(&a).books.len(), 2);
    assert_eq!(state(&a), state(&b));
}
#[test]
fn invalid_command_leaves_database_unchanged() {
    let t = Temp::new();
    let d = t.desk();
    add(&d);
    let before = d.snapshot().unwrap();
    assert!(d.execute(r#"{"op":"save_list","id":null,"title":"Bad","book_ids":["missing"],"expected_version":null}"#.into()).is_err());
    assert_eq!(d.snapshot().unwrap(), before);
}
#[test]
fn pdf_assets_export_and_restore_losslessly() {
    let t = Temp::new();
    let d = t.desk();
    add(&d);
    let book = state(&d).books.keys().next().unwrap().clone();
    let pdf = t.0.join("source.pdf");
    let bytes = b"%PDF-1.7\nfixture for byte preservation\n%%EOF";
    std::fs::write(&pdf, bytes).unwrap();
    d.import_content(book, pdf.to_string_lossy().into())
        .unwrap();
    let export = t.0.join("export.json");
    d.export_archive(export.to_string_lossy().into()).unwrap();
    let destination = Temp::new();
    let restored = destination.desk();
    restored
        .restore_archive(export.to_string_lossy().into())
        .unwrap();
    let asset = state(&restored)
        .editions
        .values()
        .find(|e| !e.assets.is_empty())
        .unwrap()
        .assets[0]
        .clone();
    let root = restored.asset_directory(asset.id).unwrap();
    assert_eq!(
        std::fs::read(PathBuf::from(root).join("book.pdf")).unwrap(),
        bytes
    );
}

#[test]
fn connection_configuration_persists_without_entering_portable_exports() {
    let t = Temp::new();
    let d = t.desk();
    let mut settings = booknook_core::settings::ConnectionSettings::default();
    settings
        .nostr_relays
        .push("wss://relay.example.test".into());
    d.save_connection_settings(serde_json::to_string(&settings).unwrap())
        .unwrap();
    assert!(
        d.save_connection_settings(
            r#"{"schema":"booknook.connections.v1","password":"must never be accepted"}"#.into()
        )
        .is_err()
    );
    drop(d);
    let reopened = t.desk();
    let loaded: booknook_core::settings::ConnectionSettings =
        serde_json::from_str(&reopened.connection_settings().unwrap()).unwrap();
    assert_eq!(loaded.nostr_relays, settings.nostr_relays);
    let export = t.0.join("export.json");
    reopened
        .export_archive(export.to_string_lossy().into())
        .unwrap();
    assert!(
        !std::fs::read_to_string(export)
            .unwrap()
            .contains("relay.example.test")
    );
}

#[test]
fn attaching_files_preserves_distinct_editions_and_deduplicates_identical_bytes() {
    let t = Temp::new();
    let d = t.desk();
    add(&d);
    let book = state(&d).books.keys().next().unwrap().clone();
    let before = state(&d).books[&book].edition_ids.len();
    let pdf = t.0.join("source.pdf");
    std::fs::write(&pdf, b"%PDF-1.7\nedition one").unwrap();
    d.import_content(book.clone(), pdf.to_string_lossy().into())
        .unwrap();
    d.import_content(book.clone(), pdf.to_string_lossy().into())
        .unwrap();
    assert_eq!(state(&d).books[&book].edition_ids.len(), before + 1);
    std::fs::write(&pdf, b"%PDF-1.7\nedition two").unwrap();
    d.import_content(book.clone(), pdf.to_string_lossy().into())
        .unwrap();
    assert_eq!(state(&d).books[&book].edition_ids.len(), before + 2);
}
#[test]
fn epub_path_traversal_is_rejected_without_importing_metadata() {
    use std::io::Write;
    let t = Temp::new();
    let d = t.desk();
    add(&d);
    let book = state(&d).books.keys().next().unwrap().clone();
    let path = t.0.join("bad.epub");
    let file = std::fs::File::create(&path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file("../escaped.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"bad").unwrap();
    zip.finish().unwrap();
    let before = d.snapshot().unwrap();
    assert!(
        d.import_content(book, path.to_string_lossy().into())
            .is_err()
    );
    assert_eq!(d.snapshot().unwrap(), before);
    assert!(!t.0.join("escaped.txt").exists());
}
#[test]
fn interrupted_command_input_preserves_previous_commit() {
    use std::io::Write;
    use std::process::{Command as Process, Stdio};
    let t = Temp::new();
    let executable = env!("CARGO_BIN_EXE_booknook");
    let mut child = Process::new(executable)
        .arg(&t.0)
        .arg("command")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(br#"{"op":"add_book","title":"Crash recovery","author":"Fixture","at":1}"#)
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(!output.stdout.is_empty());
    let d = t.desk();
    assert_eq!(
        state(&d).books.values().next().unwrap().title,
        "Crash recovery"
    );
    // A writer killed before stdin EOF has not acknowledged a commit; prior work must remain.
    let mut interrupted = Process::new(executable)
        .arg(&t.0)
        .arg("command")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    interrupted
        .stdin
        .as_mut()
        .unwrap()
        .write_all(br#"{"op":"add_book","title":"Incomplete""#)
        .unwrap();
    interrupted.kill().unwrap();
    interrupted.wait().unwrap();
    assert_eq!(state(&d).books.len(), 1);
}

fn epub_backup() -> (Temp, String, String) {
    let source = Temp::new();
    let desk = source.desk();
    add(&desk);
    let book = state(&desk).books.keys().next().unwrap().clone();
    let epub = source.0.join("fixture.epub");
    std::fs::write(&epub, include_bytes!("fixtures/restore-sample.epub")).unwrap();
    desk.import_content(book, epub.to_str().unwrap().into())
        .unwrap();
    let id = state(&desk)
        .editions
        .values()
        .flat_map(|edition| &edition.assets)
        .next()
        .unwrap()
        .id
        .clone();
    let backup = source.0.join("backup.json").to_str().unwrap().to_owned();
    desk.export_archive(backup.clone()).unwrap();
    (source, backup, id)
}

fn asset_tree(root: &std::path::Path) -> std::collections::BTreeMap<PathBuf, Option<String>> {
    fn visit(
        base: &std::path::Path,
        dir: &std::path::Path,
        tree: &mut std::collections::BTreeMap<PathBuf, Option<String>>,
    ) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let rel = path.strip_prefix(base).unwrap().to_owned();
            if entry.file_type().unwrap().is_dir() {
                tree.insert(rel, None);
                visit(base, &path, tree);
            } else {
                tree.insert(
                    rel,
                    Some(booknook_core::digest(&std::fs::read(path).unwrap())),
                );
            }
        }
    }
    let base = root.join("assets");
    let mut tree = std::collections::BTreeMap::new();
    visit(&base, &base, &mut tree);
    tree
}

#[test]
fn refused_socially_occupied_restore_keeps_asset_tree_and_both_domains() {
    let (_source, backup, _) = epub_backup();
    let target = Temp::new();
    let desk = target.desk();
    let event = serde_json::json!({"schema":"booknook.social-event.v1","id":"private-draft",
        "actor":"ab".repeat(32),"at":1,"resource_version":0,"delivery":{"kind":"public"},
        "action":{"op":"publish","group_id":null,"post":{"kind":"note","book":null,"body":"Private draft",
        "stars":null,"reading_status":null,"spoiler":false,"reply_to":null}}});
    desk.social_local(serde_json::json!({"op":"save_draft","event":event}).to_string())
        .unwrap();
    let before = (
        desk.snapshot().unwrap(),
        desk.social_snapshot(false).unwrap(),
        asset_tree(&target.0),
    );
    let error = desk.restore_archive(backup).unwrap_err();
    assert!(error.to_string().contains("empty social desk"), "{error}");
    drop(desk);
    let reopened = target.desk();
    assert_eq!(reopened.snapshot().unwrap(), before.0);
    assert_eq!(reopened.social_snapshot(false).unwrap(), before.1);
    assert_eq!(asset_tree(&target.0), before.2);
}

#[test]
fn deferred_commit_failure_undoes_new_assets_and_retains_existing_shared_files() {
    let (source, backup, epub_id) = epub_backup();
    let source_desk = source.desk();
    let book = state(&source_desk).books.keys().next().unwrap().clone();
    // An existing complete digest is shared by two editions. The destination
    // also has a file unrelated to this backup; neither may be rolled back.
    add(&source_desk);
    let second = state(&source_desk)
        .books
        .keys()
        .find(|key| **key != book)
        .unwrap()
        .clone();
    let pdf = source.0.join("shared.pdf");
    std::fs::write(&pdf, b"%PDF-1.7\nshared fixture\n%%EOF").unwrap();
    for id in [book, second] {
        source_desk
            .import_content(id, pdf.to_str().unwrap().into())
            .unwrap();
    }
    let pdf_id = booknook_core::digest(&std::fs::read(&pdf).unwrap());
    source_desk.export_archive(backup.clone()).unwrap();

    let target = Temp::new();
    let desk = target.desk();
    let shared = target.0.join("assets").join(&pdf_id);
    std::fs::create_dir(&shared).unwrap();
    std::fs::copy(pdf, shared.join("book.pdf")).unwrap();
    std::fs::write(shared.join("unrelated.txt"), b"retain this file").unwrap();
    let before = (
        desk.snapshot().unwrap(),
        desk.social_snapshot(false).unwrap(),
        asset_tree(&target.0),
    );
    let db = rusqlite::Connection::open(target.0.join("booknook.sqlite")).unwrap();
    // SQLite accepts the row update and all file promotions, then rejects COMMIT.
    db.execute_batch("CREATE TABLE test_parent(id INTEGER PRIMARY KEY);
        CREATE TABLE test_child(parent REFERENCES test_parent(id) DEFERRABLE INITIALLY DEFERRED);
        CREATE TRIGGER test_deferred AFTER UPDATE ON desk BEGIN INSERT INTO test_child VALUES(1); END;").unwrap();
    let failure = desk.restore_archive(backup.clone()).unwrap_err();
    assert!(
        failure
            .to_string()
            .contains("FOREIGN KEY constraint failed"),
        "{failure}"
    );
    assert_eq!(before.0, desk.snapshot().unwrap());
    assert_eq!(before.1, desk.social_snapshot(false).unwrap());
    assert_eq!(before.2, asset_tree(&target.0));
    assert!(!target.0.join("assets").join(&epub_id).exists());
    db.execute_batch("DROP TRIGGER test_deferred").unwrap();
    desk.restore_archive(backup).unwrap();
    let restored = state(&target.desk());
    assert_eq!(
        restored
            .editions
            .values()
            .flat_map(|e| &e.assets)
            .filter(|a| a.id == pdf_id)
            .count(),
        2
    );
    assert_eq!(
        std::fs::read(shared.join("unrelated.txt")).unwrap(),
        b"retain this file"
    );
    assert_eq!(
        std::fs::read(target.0.join("assets").join(epub_id).join("original.epub")).unwrap(),
        include_bytes!("fixtures/restore-sample.epub")
    );
}
