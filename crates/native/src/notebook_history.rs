// SPDX-License-Identifier: MIT OR Apache-2.0
//! Notebook history previews decode the same Yrs snapshots used by synchronization.
//! Restore is a new edit, so earlier versions and concurrent replica changes survive.
use crate::{Desk, Result};
use booknook_core::{Command, Error, model::InlineStyle, notebook};
use serde::Serialize;

#[derive(Serialize)]
struct RevisionPreview {
    id: String,
    block_id: String,
    at: u64,
    reader: String,
    text: String,
    styles: Vec<InlineStyle>,
}

#[uniffi::export]
impl Desk {
    pub fn notebook_history(&self, notebook_id: String, block_id: String) -> Result<String> {
        let mut store = self.lock()?;
        crate::edit_journal::recover(&mut store)?;
        let state = store.load()?;
        let document = state
            .notebooks
            .get(&notebook_id)
            .ok_or(Error::NotFound(notebook_id))?;
        if !document.blocks.iter().any(|block| block.id == block_id) {
            return Err(Error::NotFound(block_id).into());
        }
        let history = document
            .revisions
            .iter()
            .rev()
            .filter(|revision| revision.block_id == block_id)
            .map(|revision| -> booknook_core::Result<RevisionPreview> {
                Ok(RevisionPreview {
                    id: revision.id.clone(),
                    block_id: revision.block_id.clone(),
                    at: revision.at,
                    reader: revision.reader.clone(),
                    text: notebook::text(&revision.update)?,
                    styles: notebook::styles(&revision.update)?,
                })
            })
            .collect::<booknook_core::Result<Vec<_>>>()?;
        Ok(serde_json::to_string(&history)?)
    }

    pub fn restore_notebook_revision(
        &self,
        notebook_id: String,
        revision_id: String,
        base_update: String,
    ) -> Result<String> {
        let mut store = self.lock()?;
        crate::edit_journal::recover(&mut store)?;
        let mut state = store.load()?;
        let revision = state
            .notebooks
            .get(&notebook_id)
            .ok_or_else(|| Error::NotFound(notebook_id.clone()))?
            .revisions
            .iter()
            .find(|revision| revision.id == revision_id)
            .cloned()
            .ok_or(Error::NotFound(revision_id))?;
        let before = state.revision;
        booknook_core::apply(
            &mut state,
            Command::EditRichBlock {
                capture_context: None,
                note_creation: None,
                notebook_id,
                block_id: revision.block_id,
                text: notebook::text(&revision.update)?,
                styles: notebook::styles(&revision.update)?,
                base_update,
                at: crate::now()?,
            },
        )?;
        store.save(before, &state)?;
        Ok(serde_json::to_string(&state)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use booknook_core::model::State;
    use serde_json::json;
    #[test]
    fn restore_is_a_new_rich_revision_and_keeps_original_history() {
        let path =
            std::env::temp_dir().join(format!("booknook-history-{}", booknook_core::new_id()));
        let desk = Desk::new(path.to_string_lossy().into()).unwrap();
        desk.execute(
            json!({"op":"add_book","title":"History","author":"Reader","at":1}).to_string(),
        )
        .unwrap();
        let state: State = serde_json::from_str(&desk.snapshot().unwrap()).unwrap();
        let n = state.notebooks.values().next().unwrap();
        let id = n.id.clone();
        let block = n.blocks[0].id.clone();
        let base = n.blocks[0].update.clone();
        desk.execute(json!({"op":"edit_rich_block","notebook_id":id,"block_id":block,"base_update":base,"text":"First 😀 impression","styles":[{"start_utf16":0,"length_utf16":5,"bold":true,"italic":false,"underline":false}],"at":2}).to_string()).unwrap();
        let first: State = serde_json::from_str(&desk.snapshot().unwrap()).unwrap();
        let old = first.notebooks[&id].revisions.last().unwrap().clone();
        desk.execute(json!({"op":"edit_rich_block","notebook_id":id,"block_id":block,"base_update":first.notebooks[&id].blocks[0].update,"text":"Reconsidered impression","styles":[],"at":3}).to_string()).unwrap();
        let second: State = serde_json::from_str(&desk.snapshot().unwrap()).unwrap();
        let history: serde_json::Value =
            serde_json::from_str(&desk.notebook_history(id.clone(), block.clone()).unwrap())
                .unwrap();
        assert_eq!(history[1]["text"], "First 😀 impression");
        let restored: State = serde_json::from_str(
            &desk
                .restore_notebook_revision(
                    id.clone(),
                    old.id.clone(),
                    second.notebooks[&id].blocks[0].update.clone(),
                )
                .unwrap(),
        )
        .unwrap();
        let document = &restored.notebooks[&id];
        assert_eq!(document.blocks[0].text, "First 😀 impression");
        assert!(document.blocks[0].styles[0].bold);
        assert_eq!(
            document.revisions.len(),
            second.notebooks[&id].revisions.len() + 1
        );
        assert!(document.revisions.iter().any(|revision| revision == &old));
        assert!(
            desk.restore_notebook_revision(
                id.clone(),
                "unknown".into(),
                document.blocks[0].update.clone()
            )
            .is_err()
        );
        assert_eq!(
            desk.snapshot().unwrap(),
            serde_json::to_string(&restored).unwrap()
        );
        // A restore based on the viewed revision must not delete unseen remote text.
        let stale_base = document.blocks[0].update.clone();
        desk.execute(json!({"op":"edit_rich_block","notebook_id":id,"block_id":block,"base_update":stale_base,"text":"First 😀 impression + remote note","styles":[],"at":4}).to_string()).unwrap();
        let merged: State = serde_json::from_str(
            &desk
                .restore_notebook_revision(id.clone(), old.id.clone(), stale_base)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            merged.notebooks[&id].blocks[0].text,
            "First 😀 impression + remote note"
        );
        assert!(
            merged.notebooks[&id]
                .revisions
                .iter()
                .any(|revision| revision == &old)
        );
        drop(desk);
        let _ = std::fs::remove_dir_all(path);
    }
}
