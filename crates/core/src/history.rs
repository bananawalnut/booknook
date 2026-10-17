//! Merge history without collapsing distinct interpretations into a last-writer value.
use crate::{Error, Result, model::*};
use std::collections::BTreeMap;

pub fn merge_card(local: &CharacterCard, incoming: &CharacterCard) -> Result<CharacterCard> {
    if local.id != incoming.id
        || local.character_id != incoming.character_id
        || local.notebook_id != incoming.notebook_id
        || local.block_id != incoming.block_id
    {
        return Err(Error::Conflict("Card identity/attachment mismatch".into()));
    }
    let mut versions: BTreeMap<String, CardVersion> = BTreeMap::new();
    for v in local.versions.iter().chain(&incoming.versions) {
        if let Some(existing) = versions.get(&v.id) {
            if existing != v {
                return Err(Error::Conflict(
                    "Card version ID was reused with different content".into(),
                ));
            }
        } else {
            versions.insert(v.id.clone(), v.clone());
        }
    }
    let mut merged = local.clone();
    merged.versions.clear();
    while !versions.is_empty() {
        let ready: Vec<String> = versions
            .iter()
            .filter(|(_, v)| {
                v.parents
                    .iter()
                    .all(|p| merged.versions.iter().any(|known| &known.id == p))
            })
            .map(|(id, _)| id.clone())
            .collect();
        if ready.is_empty() {
            return Err(Error::Invalid("Missing or cyclic snapshot parents".into()));
        }
        for id in ready {
            merged.versions.push(versions.remove(&id).unwrap());
        }
    }
    Ok(merged)
}

pub fn merge_block(local: &Block, incoming: &Block) -> Result<Block> {
    merge_block_with_base(None, local, incoming)
}
pub fn merge_block_with_base(
    base: Option<&Block>,
    local: &Block,
    incoming: &Block,
) -> Result<Block> {
    if local.id != incoming.id || local.kind != incoming.kind {
        return Err(Error::Conflict("Block identity/type mismatch".into()));
    }
    let (update, text) = crate::notebook::merge(&local.update, &incoming.update)?;
    let styles = crate::notebook::styles(&update)?;
    Ok(Block {
        id: local.id.clone(),
        kind: local.kind.clone(),
        text,
        update,
        styles,
        ink: crate::notebook_ink::merge(local.ink.as_ref(), incoming.ink.as_ref())?,
        note: crate::notes::merge(
            base.and_then(|b| b.note.as_ref()),
            local.note.as_ref(),
            incoming.note.as_ref(),
        )?,
    })
}
