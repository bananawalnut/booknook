// SPDX-License-Identifier: MIT OR Apache-2.0
use booknook_core::model::State;
use booknook_native::Desk;
use serde_json::json;
use std::path::PathBuf;
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("booknook-ink-{}", booknook_core::new_id())))
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
fn state(d: &Desk) -> State {
    serde_json::from_str(&d.snapshot().unwrap()).unwrap()
}
#[test]
fn ink_and_transcripts_survive_sqlite_reopen_prose_restore_and_portable_reimport() {
    let a = Temp::new();
    let d = a.desk();
    d.execute(
        json!({"op":"add_book","title":"Ink fixture","author":"Synthetic","at":1}).to_string(),
    )
    .unwrap();
    let initial = state(&d);
    let n = initial.notebooks.values().next().unwrap();
    let block = &n.blocks[0];
    d.execute(json!({"op":"edit_block","notebook_id":n.id,"block_id":block.id,"base_update":block.update,"text":"Original prose","at":2}).to_string()).unwrap();
    let prose = state(&d);
    assert_eq!(prose.schema, booknook_core::model::SCHEMA);
    // Open the already persisted legacy desk before its first ink promotion.
    drop(d);
    let d = a.desk();
    assert_eq!(state(&d), prose);
    let revision = prose.notebooks[&n.id].revisions[0].id.clone();
    let ink = json!({"op":"set_notebook_ink","notebook_id":n.id,"block_id":block.id,
        "version_id":"durable-ink","parents":[],"canvas_width":768,"canvas_height":960,
        "native_drawing":"b3JpZ2luYWw=","strokes":[{"color":"#000000","width":2,
            "points":[{"x":5,"y":10,"pressure":0.7,"time":0}]}],"at":3});
    d.execute(ink.to_string()).unwrap();
    d.execute(json!({"op":"save_notebook_transcription","notebook_id":n.id,"block_id":block.id,
        "transcription_id":"durable-transcript","ink_version_id":"durable-ink","text":"Handwritten thought 🌿",
        "method":"on_device","at":4}).to_string()).unwrap();
    let saved = state(&d);
    drop(d);
    let reopened = a.desk();
    assert_eq!(state(&reopened), saved);
    let replay: serde_json::Value =
        serde_json::from_str(&reopened.execute(ink.to_string()).unwrap()).unwrap();
    assert_eq!(replay["result"]["replayed"], true);
    assert_eq!(state(&reopened), saved);
    reopened
        .execute(
            json!({"op":"edit_block","notebook_id":n.id,"block_id":block.id,
        "base_update":saved.notebooks[&n.id].blocks[0].update,"text":"Revised prose","at":5})
            .to_string(),
        )
        .unwrap();
    reopened
        .restore_notebook_revision(
            n.id.clone(),
            revision,
            state(&reopened).notebooks[&n.id].blocks[0].update.clone(),
        )
        .unwrap();
    let restored_prose = state(&reopened);
    assert_eq!(
        restored_prose.notebooks[&n.id].blocks[0].ink,
        saved.notebooks[&n.id].blocks[0].ink
    );
    assert_eq!(
        restored_prose.notebooks[&n.id].blocks[0].text,
        "Original prose"
    );
    let export = a.0.join("ink-export.json");
    reopened
        .export_archive(export.to_string_lossy().into())
        .unwrap();
    let b = Temp::new();
    let restored = b.desk();
    restored
        .restore_archive(export.to_string_lossy().into())
        .unwrap();
    let mut imported = state(&restored);
    imported.revision = restored_prose.revision;
    assert_eq!(imported, restored_prose);
}
