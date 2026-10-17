// SPDX-License-Identifier: MIT OR Apache-2.0
use booknook_native::Desk;
use serde_json::{Value, json};

#[test]
fn reviewed_audio_association_survives_native_reopen_and_portable_export() {
    let root = std::env::temp_dir().join(format!("booknook-audio-{}", booknook_core::new_id()));
    let desk = Desk::new(root.join("desk").to_string_lossy().into_owned()).unwrap();
    let catalog = json!({"schema":"booknook.kindle-catalog.v1","items":[{"asin":"B098765432","title":"A Quiet Book","authors":["Writer"],"position":null,"observed_at":null}]});
    desk.execute(json!({"op":"import_kindle","json":catalog.to_string(),"at":100}).to_string())
        .unwrap();
    let state: Value = serde_json::from_str(&desk.snapshot().unwrap()).unwrap();
    let book_id = state["books"].as_object().unwrap().keys().next().unwrap();
    desk.execute(json!({"op":"open_notebook","book_id":book_id,"at":100}).to_string())
        .unwrap();
    let state: Value = serde_json::from_str(&desk.snapshot().unwrap()).unwrap();
    let pass = state["passes"].as_object().unwrap().keys().next().unwrap();
    let source = json!({"asin":"B012345678","position_milliseconds":12345,"source_updated_at":100,"fetched_at":200,"duration_milliseconds":50000,"edition_identifier":null,"alignment":"unmatched","metadata":{"title":"A Quiet Book","authors":["Writer"],"ebook_identifiers":[{"provider":"kindle","id":"B098765432"}]}});
    let candidates: Value =
        serde_json::from_str(&desk.audio_edition_candidates(source.to_string()).unwrap()).unwrap();
    assert_eq!(candidates.as_array().unwrap().len(), 1);
    let edition = candidates[0]["edition_id"].as_str().unwrap();
    let response: Value = serde_json::from_str(
        &desk
            .attach_audio_observation(source.to_string(), pass.clone(), Some(edition.into()))
            .unwrap(),
    )
    .unwrap();
    let position = response["state"]["positions"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    assert_eq!(position["location"]["milliseconds"], 12345);
    assert_eq!(position["audio_metadata"], source["metadata"]);
    assert_eq!(position["alignment"]["status"], "confirmed");
    assert!(
        position["alignment"]["evidence"]
            .as_str()
            .unwrap()
            .contains("not mapped")
    );
    let saved = desk.snapshot().unwrap();
    assert!(
        desk.attach_audio_observation(source.to_string(), pass.clone(), Some("missing".into()))
            .is_err()
    );
    assert_eq!(desk.snapshot().unwrap(), saved);
    let archive = root.join("portable.json");
    desk.export_archive(archive.to_string_lossy().into_owned())
        .unwrap();
    drop(desk);
    let reopened = Desk::new(root.join("desk").to_string_lossy().into_owned()).unwrap();
    assert_eq!(reopened.snapshot().unwrap(), saved);
    let restored = Desk::new(root.join("restored").to_string_lossy().into_owned()).unwrap();
    restored
        .restore_archive(archive.to_string_lossy().into_owned())
        .unwrap();
    let restored: Value = serde_json::from_str(&restored.snapshot().unwrap()).unwrap();
    assert_eq!(restored["positions"], response["state"]["positions"]);
    drop(reopened);
    std::fs::remove_dir_all(root).unwrap();
}
