// SPDX-License-Identifier: MIT OR Apache-2.0
//! Latest local intent is durable before expensive CRDT/history work. Each mounted editor
//! keeps its own sequential branch, independent from newly received replica state.
use crate::{Desk, Result, store::Store};
use booknook_core::{
    Command,
    model::{CapturedReadingContext, InlineStyle, NoteCreation},
    notebook,
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Edit {
    notebook_id: String,
    block_id: String,
    session_id: String,
    sequence: u64,
    base_update: String,
    text: String,
    styles: Vec<InlineStyle>,
    at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    capture_context: Option<CapturedReadingContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    note_creation: Option<NoteCreation>,
}
fn edit_key(edit: &Edit) -> String {
    serde_json::to_string(&(&edit.notebook_id, &edit.block_id, &edit.session_id))
        .expect("Three strings always serialize")
}
fn sql(n: u64) -> Result<i64> {
    Ok(i64::try_from(n)
        .map_err(|_| booknook_core::Error::Invalid("Edit sequence exhausted".into()))?)
}
fn schema(s: &Store) -> Result<()> {
    s.connection.execute_batch("CREATE TABLE IF NOT EXISTS edit_branches(block TEXT PRIMARY KEY,session TEXT NOT NULL,sequence INTEGER NOT NULL,json_update TEXT NOT NULL); CREATE TABLE IF NOT EXISTS pending_edits(block TEXT PRIMARY KEY,json TEXT NOT NULL);")?;
    Ok(())
}
pub fn recover(s: &mut Store) -> Result<()> {
    schema(s)?;
    let edits = {
        let mut stmt = s
            .connection
            .prepare("SELECT json FROM pending_edits ORDER BY rowid")?;
        stmt.query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    for raw in edits {
        let edit: Edit = booknook_core::strict_parse(raw.as_bytes())?;
        let key = edit_key(&edit);
        let (session, base): (String, String) = s.connection.query_row(
            "SELECT session,json_update FROM edit_branches WHERE block=?1",
            [&key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if session != edit.session_id {
            return Err(booknook_core::Error::Conflict(
                "Pending edit session mismatch; journal retained for recovery".into(),
            )
            .into());
        }
        let branch = notebook::rich_edit(&base, &edit.text, &edit.styles)?;
        let mut state = s.load()?;
        let previous = state.revision;
        booknook_core::apply(
            &mut state,
            Command::MergeBlockUpdate {
                notebook_id: edit.notebook_id,
                block_id: edit.block_id.clone(),
                update: branch.clone(),
                at: edit.at,
                capture_context: edit.capture_context,
                note_creation: edit.note_creation,
            },
        )?;
        let json = serde_json::to_string(&state)?;
        if json.len() > booknook_core::MAX_JSON_BYTES {
            return Err(booknook_core::Error::Invalid(
                "Notebook exceeds the 128 MiB limit; pending edit retained".into(),
            )
            .into());
        }
        let tx = s
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let still_pending: Option<String> = tx
            .query_row(
                "SELECT json FROM pending_edits WHERE block=?1",
                [&key],
                |r| r.get(0),
            )
            .optional()?;
        let current_branch: (String, String) = tx.query_row(
            "SELECT session,json_update FROM edit_branches WHERE block=?1",
            [&key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if still_pending.as_deref() != Some(raw.as_str()) || current_branch != (session, base) {
            return Err(booknook_core::Error::Conflict(
                "A newer pending edit arrived during recovery; retry".into(),
            )
            .into());
        }
        let changed = tx.execute(
            "UPDATE desk SET revision=?1,json=?2 WHERE singleton=1 AND revision=?3",
            params![sql(state.revision)?, json, sql(previous)?],
        )?;
        if changed != 1 {
            return Err(booknook_core::Error::Conflict(
                "Notebook changed during journal recovery".into(),
            )
            .into());
        }
        tx.execute(
            "UPDATE edit_branches SET json_update=?1,sequence=?2 WHERE block=?3 AND session=?4",
            params![branch, sql(edit.sequence)?, key, edit.session_id],
        )?;
        // A competing process may stage another intent while CRDT work runs. Never erase it.
        tx.execute(
            "DELETE FROM pending_edits WHERE block=?1 AND json=?2",
            params![key, raw],
        )?;
        tx.commit()?;
    }
    Ok(())
}
#[uniffi::export]
impl Desk {
    pub fn stage_text_edit(&self, edit_json: String) -> Result<()> {
        let mut edit: Edit = booknook_core::strict_parse(edit_json.as_bytes())?;
        if edit.session_id.is_empty()
            || edit.session_id.len() > 200
            || edit.notebook_id.is_empty()
            || edit.block_id.is_empty()
            || edit.text.len() > 8 * 1024 * 1024
            || edit.base_update.len() > 32 * 1024 * 1024
            || edit.styles.len() > 100_000
        {
            return Err(booknook_core::Error::Invalid("Invalid pending edit".into()).into());
        }
        let creation_supplied = edit.note_creation.is_some();
        let sequence = sql(edit.sequence)?;
        notebook::validate_styles(&edit.text, &edit.styles)?;
        let key = edit_key(&edit);
        let mut s = self.lock()?;
        schema(&s)?;
        if edit.capture_context.is_some() || edit.note_creation.is_some() {
            // First modern edit must not overwrite an older, undated pending legacy note.
            let pending: Option<String> = s
                .connection
                .query_row(
                    "SELECT json FROM pending_edits WHERE block=?1",
                    [&key],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(raw) = pending {
                let old: Edit = booknook_core::strict_parse(raw.as_bytes())?;
                if old.capture_context.is_none()
                    && old.note_creation.is_none()
                    && !old.text.trim().is_empty()
                {
                    recover(&mut s)?;
                }
            }
            let state = s.load()?;
            let document = state
                .notebooks
                .get(&edit.notebook_id)
                .ok_or_else(|| booknook_core::Error::NotFound(edit.notebook_id.clone()))?;
            if !document.blocks.iter().any(|b| b.id == edit.block_id) {
                return Err(booknook_core::Error::NotFound(edit.block_id.clone()).into());
            }
            if let Some(context) = &edit.capture_context {
                booknook_core::notes::validate_context(&state, document, context)?;
            }
            if let Some(creation) = &edit.note_creation {
                booknook_core::notes::validate_context(
                    &state,
                    document,
                    &creation.capture_context,
                )?;
                if creation.at > edit.at {
                    return Err(booknook_core::Error::Invalid(
                        "Note creation occurs after its edit".into(),
                    )
                    .into());
                }
            } else if !edit.text.trim().is_empty() {
                edit.note_creation =
                    edit.capture_context
                        .clone()
                        .map(|capture_context| NoteCreation {
                            at: edit.at,
                            capture_context,
                        });
            }
        }
        let read_branch = |s: &Store| -> Result<Option<(String, i64)>> {
            Ok(s.connection
                .query_row(
                    "SELECT session,sequence FROM edit_branches WHERE block=?1",
                    [&key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?)
        };
        let mut known = read_branch(&s)?;
        if known
            .as_ref()
            .is_some_and(|(session, _)| session != &edit.session_id)
        {
            recover(&mut s)?;
            known = read_branch(&s)?;
        }
        let switching = known
            .as_ref()
            .is_none_or(|(session, _)| session != &edit.session_id);
        // Validate initial branch once; subsequent intents keep the last committed local
        // branch even when remote state arrives. Invalid formatting is rejected before ack.
        if switching {
            notebook::text(&edit.base_update)?;
            let state = s.load()?;
            if !state
                .notebooks
                .get(&edit.notebook_id)
                .is_some_and(|n| n.blocks.iter().any(|b| b.id == edit.block_id))
            {
                return Err(booknook_core::Error::NotFound("Notebook block".into()).into());
            }
        } else if known.as_ref().is_some_and(|(_, old)| *old >= sequence) {
            return Ok(());
        }
        // IMMEDIATE serializes independent processes during the compare-and-stage step.
        let tx = s
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: Option<(String, i64)> = tx
            .query_row(
                "SELECT session,sequence FROM edit_branches WHERE block=?1",
                [&key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if current.as_ref().map(|(session, _)| session)
            != known.as_ref().map(|(session, _)| session)
            && current.as_ref().map(|(session, _)| session) != Some(&edit.session_id)
        {
            return Err(booknook_core::Error::Conflict(
                "Another editor session changed this block; reload before retrying".into(),
            )
            .into());
        }
        let switching = current
            .as_ref()
            .is_none_or(|(session, _)| session != &edit.session_id);
        if !switching && current.as_ref().is_some_and(|(_, old)| *old >= sequence) {
            return Ok(());
        }
        if switching {
            let pending: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM pending_edits WHERE block=?1)",
                [&key],
                |r| r.get(0),
            )?;
            if pending {
                return Err(booknook_core::Error::Conflict(
                    "Another editor staged new work during session handoff; retry".into(),
                )
                .into());
            }
        }
        let previous: Option<String> = tx
            .query_row(
                "SELECT json FROM pending_edits WHERE block=?1",
                [&key],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(previous) = previous {
            let previous: Edit = booknook_core::strict_parse(previous.as_bytes())?;
            if let Some(creation) = previous.note_creation {
                if creation_supplied
                    && edit
                        .note_creation
                        .as_ref()
                        .is_some_and(|incoming| incoming != &creation)
                {
                    return Err(booknook_core::Error::Conflict(
                        "First note creation changed while text was pending".into(),
                    )
                    .into());
                }
                edit.note_creation = Some(creation);
            }
        }
        let edit_json = serde_json::to_string(&edit)?;
        tx.execute("INSERT INTO edit_branches VALUES(?1,?2,?3,?4) ON CONFLICT(block) DO UPDATE SET session=excluded.session,sequence=excluded.sequence,json_update=CASE WHEN edit_branches.session=excluded.session THEN edit_branches.json_update ELSE excluded.json_update END",params![key,edit.session_id,sequence,edit.base_update])?;
        tx.execute("INSERT INTO pending_edits VALUES(?1,?2) ON CONFLICT(block) DO UPDATE SET json=excluded.json",params![key,edit_json])?;
        tx.commit()?;
        Ok(())
    }
    pub fn flush_text_edits(&self) -> Result<String> {
        let mut s = self.lock()?;
        recover(&mut s)?;
        Ok(serde_json::to_string(&s.load()?)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use booknook_core::model::State;
    use serde_json::json;
    use std::path::PathBuf;
    struct Root(PathBuf);
    impl Root {
        fn new() -> Self {
            let p =
                std::env::temp_dir().join(format!("booknook-journal-{}", booknook_core::new_id()));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn open(&self) -> Desk {
            Desk::new(self.0.to_string_lossy().into()).unwrap()
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn add(d: &Desk) {
        d.execute(
            json!({"op":"add_book","title":"Journal book","author":"Writer","at":1}).to_string(),
        )
        .unwrap();
    }
    fn state(d: &Desk) -> State {
        serde_json::from_str(&d.snapshot().unwrap()).unwrap()
    }
    fn intent(s: &State, session: &str, sequence: u64, text: &str) -> String {
        let n = s.notebooks.values().next().unwrap();
        json!({"notebook_id":n.id,"block_id":n.blocks[0].id,"session_id":session,"sequence":sequence,"base_update":n.blocks[0].update,"text":text,"styles":[],"at":sequence+10}).to_string()
    }
    fn text(d: &Desk) -> String {
        state(d).notebooks.values().next().unwrap().blocks[0]
            .text
            .clone()
    }
    #[test]
    fn coalesced_first_input_context_survives_empty_latest_text_and_reopen() {
        let root = Root::new();
        let desk = root.open();
        add(&desk);
        let base = state(&desk);
        let document = base.notebooks.values().next().unwrap();
        let context = json!({"pass_id":document.pass_id,"positions":[{"kind":"provider","provider":"example.reader","source_id":"book","value":"chapter3"}],"position_observations":[]});
        let mut first: serde_json::Value =
            serde_json::from_str(&intent(&base, "first", 1, "thought")).unwrap();
        first["capture_context"] = context.clone();
        desk.stage_text_edit(first.to_string()).unwrap();
        let mut latest: serde_json::Value =
            serde_json::from_str(&intent(&base, "first", 2, "")).unwrap();
        latest["capture_context"] =
            json!({"pass_id":document.pass_id,"positions":[],"position_observations":[]});
        desk.stage_text_edit(latest.to_string()).unwrap();
        drop(desk);
        let reopened = root.open();
        let saved = state(&reopened);
        let note = saved.notebooks.values().next().unwrap().blocks[0]
            .note
            .as_ref()
            .unwrap();
        assert_eq!(note.created.at, Some(11));
        assert_eq!(
            serde_json::to_value(&note.created.positions).unwrap(),
            context["positions"]
        );
        assert_eq!(
            note.created.source,
            booknook_core::model::NoteOriginSource::Recorded
        );
        assert_eq!(text(&reopened), "");
    }
    #[test]
    fn legacy_pending_input_is_committed_before_modern_origin_fallback() {
        let root = Root::new();
        let desk = root.open();
        add(&desk);
        let base = state(&desk);
        desk.stage_text_edit(intent(&base, "first", 1, "old"))
            .unwrap();
        let document = base.notebooks.values().next().unwrap();
        let mut modern: serde_json::Value =
            serde_json::from_str(&intent(&base, "first", 2, "old plus modern")).unwrap();
        modern["capture_context"] =
            json!({"pass_id":document.pass_id,"positions":[],"position_observations":[]});
        desk.stage_text_edit(modern.to_string()).unwrap();
        desk.flush_text_edits().unwrap();
        let saved = state(&desk);
        let origin = &saved.notebooks.values().next().unwrap().blocks[0]
            .note
            .as_ref()
            .unwrap()
            .created;
        assert_eq!(
            origin.source,
            booknook_core::model::NoteOriginSource::HistoryFallback
        );
        assert_eq!(origin.at, Some(11));
    }
    #[test]
    fn coalesced_latest_intent_and_sequential_full_text_do_not_duplicate_characters() {
        let root = Root::new();
        let d = root.open();
        add(&d);
        let base = state(&d);
        for (seq, value) in [(1, "a"), (2, "ab"), (3, "abc")] {
            d.stage_text_edit(intent(&base, "session", seq, value))
                .unwrap();
        }
        let count: i64 = d
            .lock()
            .unwrap()
            .connection
            .query_row("SELECT COUNT(*) FROM pending_edits", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
        d.flush_text_edits().unwrap();
        assert_eq!(text(&d), "abc");
        d.stage_text_edit(intent(&base, "session", 4, "abcd"))
            .unwrap();
        d.flush_text_edits().unwrap();
        assert_eq!(text(&d), "abcd");
        d.stage_text_edit(intent(&base, "session", 2, "old"))
            .unwrap();
        d.flush_text_edits().unwrap();
        assert_eq!(text(&d), "abcd");
    }
    #[test]
    fn acknowledged_pending_unicode_edit_recovers_on_open_before_visible_state_is_read() {
        let root = Root::new();
        let d = root.open();
        add(&d);
        let base = state(&d);
        d.stage_text_edit(intent(&base, "session", 1, "A café 🦊 日本語"))
            .unwrap();
        assert_eq!(
            d.lock()
                .unwrap()
                .load()
                .unwrap()
                .notebooks
                .values()
                .next()
                .unwrap()
                .blocks[0]
                .text,
            ""
        );
        drop(d);
        let reopened = root.open();
        assert_eq!(text(&reopened), "A café 🦊 日本語");
        let count: i64 = reopened
            .lock()
            .unwrap()
            .connection
            .query_row("SELECT COUNT(*) FROM pending_edits", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
    #[test]
    fn newly_received_remote_text_survives_the_next_local_branch_snapshot() {
        let root = Root::new();
        let d = root.open();
        add(&d);
        let base = state(&d);
        d.stage_text_edit(intent(&base, "session", 1, "local"))
            .unwrap();
        d.flush_text_edits().unwrap();
        let n = base.notebooks.values().next().unwrap();
        d.execute(json!({"op":"edit_block","notebook_id":n.id,"block_id":n.blocks[0].id,"base_update":n.blocks[0].update,"text":"REMOTE","at":30}).to_string()).unwrap();
        d.stage_text_edit(intent(&base, "session", 2, "local plus"))
            .unwrap();
        d.flush_text_edits().unwrap();
        let final_text = text(&d);
        assert!(final_text.contains("local plus"));
        assert!(final_text.contains("REMOTE"));
        assert_eq!(final_text.matches("REMOTE").count(), 1);
    }
    #[test]
    fn notebook_scoped_block_ids_cannot_overwrite_each_others_journal() {
        let root = Root::new();
        let d = root.open();
        add(&d);
        add(&d);
        let mut state = state(&d);
        let ids: Vec<String> = state.notebooks.keys().cloned().collect();
        let shared = state.notebooks[&ids[0]].blocks[0].id.clone();
        state.notebooks.get_mut(&ids[1]).unwrap().blocks[0].id = shared.clone();
        {
            let mut store = d.lock().unwrap();
            let prior = state.revision;
            state.revision += 1;
            store.save(prior, &state).unwrap();
        }
        for (index, notebook) in ids.iter().enumerate() {
            let n = &state.notebooks[notebook];
            d.stage_text_edit(json!({"notebook_id":notebook,"block_id":shared,"session_id":format!("session{index}"),"sequence":1,"base_update":n.blocks[0].update,"text":format!("Notebook {index}"),"styles":[],"at":20}).to_string()).unwrap();
        }
        d.flush_text_edits().unwrap();
        let result = super::tests::state(&d);
        assert_eq!(result.notebooks[&ids[0]].blocks[0].text, "Notebook 0");
        assert_eq!(result.notebooks[&ids[1]].blocks[0].text, "Notebook 1");
    }
    #[test]
    fn malformed_styles_rejected_before_ack_and_corrupt_existing_journal_retained() {
        let root = Root::new();
        let d = root.open();
        add(&d);
        let base = state(&d);
        let mut malformed: serde_json::Value =
            serde_json::from_str(&intent(&base, "session", 1, "🦊")).unwrap();
        malformed["styles"] = json!([{"start_utf16":0,"length_utf16":1,"bold":true,"italic":false,"underline":false}]);
        assert!(d.stage_text_edit(malformed.to_string()).is_err());
        assert_eq!(
            d.lock()
                .unwrap()
                .connection
                .query_row("SELECT COUNT(*) FROM pending_edits", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let before = d.lock().unwrap().load().unwrap();
        d.stage_text_edit(intent(&base, "session", 1, "recover me"))
            .unwrap();
        d.lock()
            .unwrap()
            .connection
            .execute("UPDATE pending_edits SET json='broken journal'", [])
            .unwrap();
        assert!(d.flush_text_edits().is_err());
        assert_eq!(d.lock().unwrap().load().unwrap(), before);
        drop(d);
        assert!(Desk::new(root.0.to_string_lossy().into()).is_err());
        let db = rusqlite::Connection::open(root.0.join("booknook.sqlite")).unwrap();
        assert_eq!(
            db.query_row("SELECT json FROM pending_edits", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "broken journal"
        );
    }
    #[test]
    fn independent_handles_stage_highest_sequence_without_regressing_or_losing_acknowledged_text() {
        let root = Root::new();
        let first = root.open();
        add(&first);
        let base = state(&first);
        let second = root.open();
        std::thread::scope(|scope| {
            let base = &base;
            let first = &first;
            let second = &second;
            let a = scope.spawn(move || {
                for sequence in (1..40).step_by(2) {
                    first
                        .stage_text_edit(intent(
                            base,
                            "shared-session",
                            sequence,
                            &format!("text {sequence}"),
                        ))
                        .unwrap();
                }
            });
            let b = scope.spawn(move || {
                for sequence in (2..41).step_by(2) {
                    second
                        .stage_text_edit(intent(
                            base,
                            "shared-session",
                            sequence,
                            &format!("text {sequence}"),
                        ))
                        .unwrap();
                }
            });
            a.join().unwrap();
            b.join().unwrap();
        });
        first.flush_text_edits().unwrap();
        assert_eq!(text(&first), "text 40");
    }
    #[test]
    fn alternating_mounted_sessions_retain_their_own_sequential_branches() {
        let root = Root::new();
        let d = root.open();
        add(&d);
        let first_base = state(&d);
        d.stage_text_edit(intent(&first_base, "editor-a", 1, "a"))
            .unwrap();
        d.flush_text_edits().unwrap();
        let second_base = state(&d);
        d.stage_text_edit(intent(&second_base, "editor-b", 1, "a remote"))
            .unwrap();
        d.flush_text_edits().unwrap();
        d.stage_text_edit(intent(&first_base, "editor-a", 2, "ab"))
            .unwrap();
        d.flush_text_edits().unwrap();
        let text = text(&d);
        assert_eq!(
            text.matches('a').count(),
            1,
            "An older mounted session must not replay already inserted characters"
        );
        assert_eq!(text.matches('b').count(), 1);
        assert!(text.contains("remote"));
    }
}
