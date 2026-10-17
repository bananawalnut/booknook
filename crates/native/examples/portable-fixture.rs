// SPDX-License-Identifier: MIT OR Apache-2.0
//! Produce a synthetic complete native archive for cross-SDK conformance. No user data.
use booknook_core::{
    canonical, model::State, portable_archive as archive, social_protocol as social,
};
use booknook_native::Desk;
use nostr_sdk::prelude::*;
use serde_json::{Value, json};
use std::{io::Write, path::Path};
#[path = "../tests/fixtures/social_receipts.rs"]
mod receipts;
fn command(desk: &Desk, command: Value) -> Value {
    serde_json::from_str::<Value>(&desk.execute(command.to_string()).unwrap()).unwrap()["result"]
        .clone()
}
fn main() {
    let directory = std::env::args()
        .nth(1)
        .expect("temporary fixture directory");
    let root = Path::new(&directory);
    std::fs::create_dir_all(root).unwrap();
    let source = Desk::new(root.join("source").to_string_lossy().into_owned()).unwrap();
    command(
        &source,
        json!({"op":"add_book","title":"Portable café 日本語 🌿","author":"Synthetic writer","at":100}),
    );
    let state: State = serde_json::from_str(&source.snapshot().unwrap()).unwrap();
    let book = state.books.values().next().unwrap();
    let notebook = state.notebooks.values().next().unwrap();
    let block = &notebook.blocks[0];
    command(
        &source,
        json!({"op":"edit_rich_block","notebook_id":notebook.id,"block_id":block.id,"text":"Ink stays. Café 日本語 🌿","base_update":block.update,"styles":[{"start_utf16":0,"length_utf16":3,"bold":true,"italic":false,"underline":false}],"at":101}),
    );
    for (id, parents, strokes, at) in [
        (
            "ink-first",
            vec![],
            json!([{"points":[{"x":1.5,"y":2.0,"pressure":0.5,"time":0.0},{"x":9.5,"y":5.0,"pressure":0.8,"time":0.2}],"color":"#315b48","width":2.0}]),
            102,
        ),
        ("ink-erased", vec!["ink-first"], json!([]), 103),
        ("ink-branch", vec!["ink-first"], json!([]), 104),
    ] {
        command(
            &source,
            json!({"op":"set_notebook_ink","notebook_id":notebook.id,"block_id":block.id,"version_id":id,"parents":parents,"canvas_width":768.0,"canvas_height":960.0,"strokes":strokes,"native_drawing":"U3ludGhldGljIG9wYXF1ZSBpbms=","at":at}),
        );
    }
    command(
        &source,
        json!({"op":"save_notebook_transcription","notebook_id":notebook.id,"block_id":block.id,"transcription_id":"transcript-first","ink_version_id":"ink-first","text":"Reviewed café 日本語 🌿","method":"manual","at":105}),
    );
    command(
        &source,
        json!({"op":"add_character","notebook_id":notebook.id,"block_id":block.id,"name":"Mara","at":106}),
    );
    let current: State = serde_json::from_str(&source.snapshot().unwrap()).unwrap();
    let card = current.cards.values().next().unwrap();
    let mut interpretation = serde_json::to_value(&card.versions[0].interpretation).unwrap();
    interpretation["description"] = "Chapter 12 reassessment".into();
    command(
        &source,
        json!({"op":"save_card","card_id":card.id,"parents":[card.versions[0].id],"interpretation":interpretation,"positions":[],"at":107}),
    );
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, content) in [
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
            r#"<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Fixture</title></head><body><p>Original café 日本語 🌿</p></body></html>"#,
        ),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(content.as_bytes()).unwrap();
    }
    let epub = zip.finish().unwrap().into_inner();
    let epub_path = root.join("original.epub");
    std::fs::write(&epub_path, &epub).unwrap();
    source
        .import_content(book.id.clone(), epub_path.to_string_lossy().into_owned())
        .unwrap();
    let pdf=b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Count 0 /Kids [] >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n";
    let pdf_path = root.join("original.pdf");
    std::fs::write(&pdf_path, pdf).unwrap();
    source
        .import_content(book.id.clone(), pdf_path.to_string_lossy().into_owned())
        .unwrap();
    let export = root.join("seed.json");
    source
        .export_archive(export.to_string_lossy().into_owned())
        .unwrap();
    let mut value = archive::parse(&std::fs::read(&export).unwrap()).unwrap();
    let mut social = social::SocialState::default();
    let event = social::prepare(
        &social,
        &receipts::actor(2),
        social::SocialAction::CreateList {
            id: format!("{}:fixture-list", receipts::actor(2)),
            name: "Portable reading list".into(),
            description: "Synthetic shared records".into(),
            books: vec![social::BookReference {
                id: "isbn:fixture".into(),
                title: book.title.clone(),
                author: "Synthetic writer".into(),
            }],
        },
        200,
    )
    .unwrap();
    let (request, presentation, policy) = receipts::proof(&event, 2);
    social::approve(&mut social, event, request, presentation, &policy, 200).unwrap();
    social::local(
        &mut social,
        social::LocalSocialCommand::Block {
            actor: receipts::actor(3),
            blocked: true,
        },
    )
    .unwrap();
    let keys = Keys::parse(&"03".repeat(32)).unwrap();
    let binding = archive::KeyBinding {
        schema: "booknook.nostr-binding.v1".into(),
        castalia_holder: receipts::actor(2),
        nostr_public_key: keys.public_key().to_hex(),
        valid_from: 100,
        valid_until: 1000,
        version: 1,
    };
    let (request_json, presentation_json, _) =
        receipts::receipt(&canonical(&binding).unwrap(), "nostr.bind", 1, 100, 2);
    let document = json!({"schema":"booknook.transport-identity.v1","binding":binding,"approved_at":100,"request_json":request_json,"presentation_json":presentation_json});
    let pending = EventBuilder::new(Kind::Custom(30078), "Synthetic unresolved relay data")
        .custom_created_at(Timestamp::from(201))
        .sign_with_keys(&keys)
        .unwrap();
    value.social = Some(
        json!({"schema":archive::SOCIAL_SCHEMA,"state":social,"bindings":[{"document":document,"minimum_version":1}],"binding_history":[{"document":document,"retired_at":null}],"pending":[{"id":pending.id.to_hex(),"event_json":format!("\n{}\n",pending.as_json()),"reason":"Waiting for a synthetic parent"}]}),
    );
    std::fs::write(&export, serde_json::to_vec(&value).unwrap()).unwrap();
    let restored = Desk::new(root.join("native-exporter").to_string_lossy().into_owned()).unwrap();
    restored
        .restore_archive(export.to_string_lossy().into_owned())
        .unwrap();
    let destination = root.join("native-export.json");
    restored
        .export_archive(destination.to_string_lossy().into_owned())
        .unwrap();
    println!("{}", destination.display());
}
