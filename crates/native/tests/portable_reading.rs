//! The real native portable envelope must preserve rich reading records and original assets,
//! including unresolved branches and their eventual explicit reconciliation.
use booknook_core::model::*;
use booknook_native::Desk;
use serde_json::{Value, json};
use std::{io::Write, path::PathBuf};

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "booknook-reading-archive-{}",
            booknook_core::new_id()
        )))
    }
    fn open(&self) -> Desk {
        Desk::new(self.0.to_string_lossy().into_owned()).unwrap()
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn command(desk: &Desk, value: Value) -> Value {
    serde_json::from_str::<Value>(&desk.execute(value.to_string()).unwrap()).unwrap()["result"]
        .clone()
}
fn snapshot(desk: &Desk) -> State {
    serde_json::from_str(&desk.snapshot().unwrap()).unwrap()
}
fn epub() -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in [
        ("mimetype", "application/epub+zip"),
        (
            "META-INF/container.xml",
            r#"<container><rootfiles><rootfile full-path="OPS/package.opf"/></rootfiles></container>"#,
        ),
        (
            "OPS/package.opf",
            r#"<package><manifest><item id="one" href="chapter.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="one"/></spine></package>"#,
        ),
        (
            "OPS/chapter.xhtml",
            r#"<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Fixture</title></head><body><p>Before. A café 🦊 日本語. After.</p></body></html>"#,
        ),
    ] {
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(bytes.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}
fn pdf() -> Vec<u8> {
    let content = "BT /F1 18 Tf 30 100 Td (Archive fixture) Tj ET";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
    ];
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        write!(bytes, "{} 0 obj\n{object}\nendobj\n", index + 1).unwrap();
    }
    let xref = bytes.len();
    write!(bytes, "xref\n0 {}\n0000000000 65535 f \n", offsets.len()).unwrap();
    for offset in offsets.iter().skip(1) {
        writeln!(bytes, "{offset:010} 00000 n ").unwrap();
    }
    write!(
        bytes,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        offsets.len()
    )
    .unwrap();
    bytes
}
fn assert_archive_roundtrip(desk: &Desk, expected: &State, assets: &[(String, Vec<u8>)]) {
    let first = Root::new();
    let restored = first.open();
    let archive = first.0.join("source.json");
    desk.export_archive(archive.to_string_lossy().into_owned())
        .unwrap();
    restored
        .restore_archive(archive.to_string_lossy().into_owned())
        .unwrap();
    let mut state = snapshot(&restored);
    state.revision = expected.revision;
    assert_eq!(
        &state, expected,
        "every portable field survives the native envelope"
    );
    for (asset, original) in assets {
        let directory = PathBuf::from(restored.asset_directory(asset.clone()).unwrap());
        let filename = if original.starts_with(b"%PDF") {
            "book.pdf"
        } else {
            "original.epub"
        };
        assert_eq!(std::fs::read(directory.join(filename)).unwrap(), *original);
    }
    // Re-export and reimport proves the destination retains the original asset bytes,
    // not merely transient references into the exporting desk.
    let repeated = first.0.join("reexport.json");
    restored
        .export_archive(repeated.to_string_lossy().into_owned())
        .unwrap();
    let second = Root::new();
    let reopened = second.open();
    reopened
        .restore_archive(repeated.to_string_lossy().into_owned())
        .unwrap();
    let mut twice = snapshot(&reopened);
    twice.revision = expected.revision;
    assert_eq!(&twice, expected);
    let mut first_json: Value = serde_json::from_slice(&std::fs::read(archive).unwrap()).unwrap();
    let mut next_json: Value = serde_json::from_slice(&std::fs::read(repeated).unwrap()).unwrap();
    first_json["state"]["revision"] = Value::Null;
    next_json["state"]["revision"] = Value::Null;
    assert_eq!(
        first_json, next_json,
        "only the destination store revision changes"
    );
}

#[test]
fn legacy_mobi_import_and_restore_keep_original_bytes_and_stable_reader_resource() {
    let root = Root::new();
    let desk = root.open();
    let book = command(
        &desk,
        json!({"op":"add_book", "title":"The Quiet Margin", "author":"Fixture", "at":1}),
    )["book_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let catalog_editions = snapshot(&desk).books[&book].edition_ids.clone();
    let original = include_bytes!("fixtures/mobi6-reader.mobi");
    let asset_id = booknook_core::digest(original);
    for extension in ["mobi", "AZW"] {
        let source = root.0.join(format!("reader.{extension}"));
        std::fs::write(&source, original).unwrap();
        desk.import_content(book.clone(), source.to_string_lossy().into_owned())
            .unwrap();
        assert_eq!(std::fs::read(source).unwrap(), original.as_slice());
    }
    let state = snapshot(&desk);
    let editions = &state.books[&book].edition_ids;
    let (edition_id, earlier_editions) = editions.split_last().unwrap();
    assert_eq!(earlier_editions, catalog_editions.as_slice());
    let asset = &state.editions[edition_id].assets[0];
    assert_eq!(asset.id, asset_id);
    assert_eq!(asset.storage_key, asset_id);
    assert_eq!(asset.sha256, asset_id);
    assert_eq!(asset.media_type, MOBI6_MEDIA_TYPE);
    assert_eq!(asset.spine, ["mobi6-v1/book.xhtml"]);
    let folder = PathBuf::from(desk.asset_directory(asset_id.clone()).unwrap());
    assert_eq!(
        std::fs::read(folder.join("original.mobi")).unwrap(),
        original.as_slice()
    );
    let xhtml = std::fs::read_to_string(folder.join(&asset.spine[0])).unwrap();
    let parsed = roxmltree::Document::parse(&xhtml).unwrap();
    assert!(
        parsed
            .descendants()
            .any(|n| n.is_text() && n.text().is_some_and(|s| s.contains("Café, 日本語, 🌿.")))
    );

    let rendered = folder.join(&asset.spine[0]);
    let reimport = || {
        desk.import_content(
            book.clone(),
            root.0.join("reader.mobi").to_string_lossy().into_owned(),
        )
    };
    std::fs::write(&rendered, vec![b'X'; xhtml.len()]).unwrap();
    reimport().unwrap();
    assert_eq!(std::fs::read_to_string(&rendered).unwrap(), xhtml);
    std::fs::remove_file(&rendered).unwrap();
    reimport().unwrap();
    assert_eq!(std::fs::read_to_string(&rendered).unwrap(), xhtml);
    let repaired_state = snapshot(&desk);
    let mut expected_state = state.clone();
    expected_state.revision = repaired_state.revision;
    assert_eq!(repaired_state, expected_state);

    let original_path = folder.join("original.mobi");
    std::fs::write(&original_path, vec![0; original.len()]).unwrap();
    assert!(
        reimport()
            .unwrap_err()
            .to_string()
            .contains("Stored MOBI original does not match")
    );
    assert_eq!(
        std::fs::read(&original_path).unwrap(),
        vec![0; original.len()]
    );
    assert_eq!(std::fs::read_to_string(&rendered).unwrap(), xhtml);
    assert_eq!(snapshot(&desk), repaired_state);
    std::fs::write(original_path, original).unwrap();

    let receiver = Root::new();
    let restored = receiver.open();
    let archive = root.0.join("mobi-backup.json");
    desk.export_archive(archive.to_string_lossy().into_owned())
        .unwrap();

    // A failed previous restore can leave valid originals with stale caches.
    let previous = receiver.0.join("assets").join(&asset_id);
    let previous_rendered = previous.join(&asset.spine[0]);
    std::fs::create_dir_all(previous_rendered.parent().unwrap()).unwrap();
    std::fs::write(previous.join("original.mobi"), original).unwrap();
    std::fs::write(&previous_rendered, b"stale derived data").unwrap();
    restored
        .restore_archive(archive.to_string_lossy().into_owned())
        .unwrap();
    let received = snapshot(&restored);
    assert_eq!(
        received.editions[edition_id].assets.as_slice(),
        std::slice::from_ref(asset)
    );
    let received_folder = PathBuf::from(restored.asset_directory(asset_id).unwrap());
    assert_eq!(
        std::fs::read(received_folder.join("original.mobi")).unwrap(),
        original.as_slice()
    );
    assert_eq!(
        std::fs::read_to_string(received_folder.join(&asset.spine[0])).unwrap(),
        xhtml
    );

    // Merely marking the synthetic fixture as encrypted is enough to reject the
    // selection; no secret, protected book or byte conversion is attempted.
    let mut protected = original.to_vec();
    let header = u32::from_be_bytes(protected[78..82].try_into().unwrap()) as usize;
    protected[header + 12..header + 14].copy_from_slice(&1u16.to_be_bytes());
    for (name, bytes) in [
        ("protected.azw", protected.as_slice()),
        ("renamed.azw", b"not a book".as_slice()),
        ("unsupported.azw3", original.as_slice()),
    ] {
        let source = root.0.join(name);
        std::fs::write(&source, bytes).unwrap();
        assert!(
            desk.import_content(book.clone(), source.to_string_lossy().into_owned())
                .is_err(),
            "{name}"
        );
        assert_eq!(std::fs::read(source).unwrap(), bytes);
        assert_eq!(snapshot(&desk), repaired_state);
    }
    assert_eq!(std::fs::read_dir(root.0.join("assets")).unwrap().count(), 1);
}

#[test]
fn native_archive_preserves_branched_character_history_rich_notes_positions_and_ink() {
    let root = Root::new();
    let desk = root.open();
    let book = command(
        &desk,
        json!({"op":"add_book", "title":"Quiet library", "author":"Fixture", "at":1}),
    )["book_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let initial = snapshot(&desk);
    let notebook = initial.notebooks.values().next().unwrap().clone();
    let session = command(
        &desk,
        json!({"op":"start_session","pass_id":notebook.pass_id,"at":2}),
    )["session_id"]
        .clone();
    command(
        &desk,
        json!({"op":"end_session","session_id":session,"at":9}),
    );
    command(
        &desk,
        json!({"op":"start_session","pass_id":notebook.pass_id,"at":10}),
    );
    for (kind, name) in [
        ("place", "The library"),
        ("theme", "Memory"),
        ("event", "The return"),
        ("concept", "Trust"),
    ] {
        command(
            &desk,
            json!({"op":"create_literary_entity","book_id":book,"kind":kind,"name":name}),
        );
    }
    let mut originals = Vec::new();
    for (extension, bytes) in [("epub", epub()), ("pdf", pdf())] {
        let path = root.0.join(format!("source.{extension}"));
        std::fs::write(&path, &bytes).unwrap();
        desk.import_content(book.clone(), path.to_string_lossy().into_owned())
            .unwrap();
        originals.push((booknook_core::digest(&bytes), bytes));
    }
    let imported = snapshot(&desk);
    let edition = |mime: &str| {
        imported
            .editions
            .values()
            .find(|e| e.assets.iter().any(|a| a.media_type == mime))
            .unwrap()
    };
    let ebook = edition("application/epub+zip");
    let pdf = edition("application/pdf");
    let text = json!({"kind":"text", "edition_id":ebook.id, "resource":"OPS/chapter.xhtml", "quote":"A café 🦊 日本語", "prefix":"Before. ", "suffix":". After."});
    let page = json!({"kind":"page", "edition_id":pdf.id, "page":0});
    let audio = json!({"kind":"audio", "provider":"audible", "source_id":"fixture-audio", "milliseconds":123456, "chapter":"Chapter 3"});
    let provider = json!({"kind":"provider", "provider":"kindle", "source_id":"fixture-kindle", "value":"loc:200"});
    for (index, location) in [text.clone(), page.clone(), audio.clone(), provider]
        .into_iter()
        .enumerate()
    {
        command(
            &desk,
            json!({"op":"observe_position", "observation":{
                "id":format!("position-{index}"), "pass_id":notebook.pass_id, "location":location,
                "observed_at":10+index, "received_at":40+index,
                "alignment":{"status":"candidate", "edition_id":ebook.id, "reason":"Unconfirmed edition evidence; never equal percentages"}
            }}),
        );
    }
    let rich = "A café 🦊 日本語\nAn interpretation in context.";
    command(
        &desk,
        json!({"op":"edit_rich_block", "notebook_id":notebook.id,
        "block_id":notebook.blocks[0].id, "base_update":notebook.blocks[0].update,
        "text":rich, "styles":[{"start_utf16":7,"length_utf16":2,"bold":true,"italic":false,"underline":true}], "at":50}),
    );
    let card = command(&desk, json!({"op":"add_character", "notebook_id":notebook.id, "block_id":notebook.blocks[0].id, "name":"Mara", "at":51}))["card_id"].as_str().unwrap().to_owned();
    let other = command(&desk, json!({"op":"add_character", "notebook_id":notebook.id, "block_id":notebook.blocks[0].id, "name":"The archivist", "at":52}))["card_id"].as_str().unwrap().to_owned();
    let characters = snapshot(&desk);
    let original = characters.cards[&card].versions[0].id.clone();
    let interpretation = |description: &str, passage: Value| {
        json!({
            "description":description, "traits":["uncertain", "observant"],
            "relationships":[{"target":characters.cards[&other].character_id,"description":"Questions the archivist’s account"}],
            "observations":["Pauses before answering — 日本語"], "questions":["Is this account trustworthy?"], "passages":[passage]
        })
    };
    for (description, passage, at) in [
        ("Trustworthy in chapter 3", text.clone(), 60),
        ("Unreliable by chapter 12", audio.clone(), 70),
    ] {
        command(
            &desk,
            json!({"op":"save_card", "card_id":card, "parents":[original],
            "interpretation":interpretation(description,passage), "positions":[], "at":at}),
        );
    }
    for (index, anchor, space, resource, page_number, fingerprint, quote) in [
        (
            0,
            text.clone(),
            "epub_document",
            "OPS/chapter.xhtml",
            None,
            &ebook.assets[0].sha256,
            Some("A café 🦊 日本語"),
        ),
        (
            1,
            page.clone(),
            "pdf_page",
            "book.pdf",
            Some(0),
            &pdf.assets[0].sha256,
            None,
        ),
    ] {
        command(
            &desk,
            json!({"op":"save_annotation", "annotation":{
                "id":format!("ink-{index}"), "pass_id":notebook.pass_id, "anchor":anchor,
                "original_context":"A café 🦊 日本語", "layout_fingerprint":"fixture-layout",
                "strokes":[{"points":[{"x":24.25,"y":42.75,"pressure":0.8,"time":0.125},{"x":27.5,"y":43.75,"pressure":0.4,"time":0.25}],"color":"#806f39","width":1.75}],
                "native_drawing":"b3BhcXVlLW5hdGl2ZS1kcmF3aW5nLWZpeHR1cmU=",
                "ink_geometry":{"space":space,"resource":resource,"page":page_number,"content_fingerprint":fingerprint,
                    "width":300.0,"height":500.0,"origin_x":0.0,"origin_y":0.0,
                    "text_quote":quote,"text_prefix":null,"text_suffix":null,"text_rects":[{"x":20.0,"y":40.0,"width":90.0,"height":16.0}]}
            }}),
        );
    }
    let next = command(&desk, json!({"op":"start_pass", "book_id":book, "at":100}));
    command(
        &desk,
        json!({"op":"attach_character", "notebook_id":next["id"], "block_id":next["blocks"][0]["id"],
        "character_id":characters.cards[&card].character_id,"at":101}),
    );
    let branched = snapshot(&desk);
    assert_eq!(branched.cards[&card].heads().len(), 2);
    assert_eq!(
        branched.cards[&card].versions[1]
            .position_observations
            .len(),
        4
    );
    assert_eq!(branched.entities.len(), 6);
    assert_eq!(
        branched
            .entities
            .values()
            .filter(|e| e.kind == LiteraryKind::Character)
            .count(),
        2,
        "second pass reuses the same character identity"
    );
    assert_archive_roundtrip(&desk, &branched, &originals);

    let parents: Vec<_> = branched.cards[&card]
        .heads()
        .iter()
        .map(|v| &v.id)
        .collect();
    let mut reconciled = interpretation("Both earlier interpretations remain useful", text);
    reconciled["passages"].as_array_mut().unwrap().push(audio);
    command(
        &desk,
        json!({"op":"save_card", "card_id":card,"parents":parents,"interpretation":reconciled,"positions":[],"at":120}),
    );
    let final_state = snapshot(&desk);
    assert_eq!(final_state.cards[&card].heads().len(), 1);
    assert_eq!(
        &final_state.cards[&card].versions[..3],
        &branched.cards[&card].versions[..]
    );
    assert_archive_roundtrip(&desk, &final_state, &originals);
}
