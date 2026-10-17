//! Three-way private replica reconciliation. Conflicting personal edits remain explicit.
use crate::{Error, Result, canonical, digest, history, model::*};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Conflict {
    pub id: String,
    pub collection: String,
    pub record_id: String,
    pub field: String,
    pub local: Value,
    pub incoming: Value,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MergeReport {
    pub state: State,
    pub conflicts: Vec<Conflict>,
}
fn value<T: Serialize>(v: &T) -> Result<Value> {
    Ok(serde_json::to_value(v)?)
}
fn conflict(
    out: &mut Vec<Conflict>,
    collection: &str,
    id: &str,
    field: &str,
    a: Value,
    b: Value,
) -> Result<()> {
    let key = digest(&canonical(&(collection, id, field, &a, &b))?);
    out.push(Conflict {
        id: key,
        collection: collection.into(),
        record_id: id.into(),
        field: field.into(),
        local: a,
        incoming: b,
    });
    Ok(())
}
fn merge_records<T: Serialize + serde::de::DeserializeOwned + Clone + PartialEq>(
    collection: &str,
    base: Option<&BTreeMap<String, T>>,
    local: &mut BTreeMap<String, T>,
    incoming: &BTreeMap<String, T>,
    immutable: &[&str],
    unions: &[&str],
    conflicts: &mut Vec<Conflict>,
) -> Result<()> {
    for (id, b) in incoming {
        let Some(a) = local.get(id) else {
            local.insert(id.clone(), b.clone());
            continue;
        };
        if a == b {
            continue;
        }
        let mut av = value(a)?;
        let bv = value(b)?;
        let base = base.and_then(|m| m.get(id)).map(value).transpose()?;
        for field in immutable {
            if av[*field] != bv[*field] {
                return Err(Error::Conflict(format!(
                    "{collection}/{id}: immutable identity changed ({field})"
                )));
            }
        }
        for (field, remote) in bv
            .as_object()
            .ok_or_else(|| Error::Invalid("Expected record".into()))?
        {
            let current = &av[field];
            if current == remote {
                continue;
            }
            if unions.contains(&field.as_str()) {
                let mut all = current
                    .as_array()
                    .ok_or_else(|| Error::Invalid("Expected collection".into()))?
                    .clone();
                for item in remote
                    .as_array()
                    .ok_or_else(|| Error::Invalid("Expected collection".into()))?
                {
                    if !all.contains(item) {
                        all.push(item.clone());
                    }
                }
                av[field] = Value::Array(all);
            } else if base.as_ref().is_some_and(|old| &old[field] == current) {
                av[field] = remote.clone();
            } else if !base.as_ref().is_some_and(|old| &old[field] == remote) {
                conflict(
                    conflicts,
                    collection,
                    id,
                    field,
                    current.clone(),
                    remote.clone(),
                )?;
            }
        }
        local.insert(id.clone(), serde_json::from_value(av)?);
    }
    Ok(())
}
/// Neither absent records nor an older replica can delete local work. Deletions require a
/// future explicit tombstone contract. An unresolved value is stored outside the active record.
pub fn reconcile(base: Option<&State>, local: &State, incoming: &State) -> Result<MergeReport> {
    crate::validate(local)?;
    crate::validate(incoming)?;
    if let Some(base) = base {
        crate::validate(base)?;
    }
    let mut state = local.clone();
    if incoming.schema == NOTE_ARCHIVE_SCHEMA {
        crate::notes::promote(&mut state);
    } else if incoming.schema == INK_ARCHIVE_SCHEMA && state.schema != NOTE_ARCHIVE_SCHEMA {
        state.schema = INK_ARCHIVE_SCHEMA.into();
        state.ontology_version = INK_ONTOLOGY.into();
    }
    let mut conflicts = vec![];
    macro_rules! records {
        ($name:ident,$immutable:expr,$unions:expr) => {
            merge_records(
                stringify!($name),
                base.map(|s| &s.$name),
                &mut state.$name,
                &incoming.$name,
                $immutable,
                $unions,
                &mut conflicts,
            )?;
        };
    }
    records!(
        books,
        &["id"],
        &[
            "edition_ids",
            "sources",
            "provenance",
            "edited_fields",
            "reading_dates"
        ]
    );
    records!(editions, &["id", "book_id"], &["identifiers", "assets"]);
    records!(passes, &["id", "book_id", "reader", "started_at"], &[]);
    records!(sessions, &["id", "pass_id", "started_at"], &[]);
    records!(entities, &["id", "book_id", "kind"], &[]);
    records!(annotations, &["id", "pass_id"], &[]);
    records!(lists, &["id", "owner"], &[]);
    records!(groups, &["id", "owner"], &[]);
    records!(
        activity,
        &[
            "id",
            "actor",
            "published_at",
            "kind",
            "book_id",
            "body",
            "spoiler",
            "group_id",
            "group_epoch"
        ],
        &[]
    );
    records!(drafts, &["id", "created_at"], &[]);
    for (id, b) in &incoming.positions {
        match state.positions.get(id) {
            Some(a) if a.pass_id != b.pass_id => {
                return Err(Error::Conflict("Position identity changed".into()));
            }
            Some(a) if a.observed_at > b.observed_at => {}
            Some(a) if a.observed_at == b.observed_at && a != b => {
                conflict(&mut conflicts, "positions", id, "", value(a)?, value(b)?)?;
            }
            _ => {
                state.positions.insert(id.clone(), b.clone());
            }
        }
    }
    for (id, b) in &incoming.notebooks {
        if let Some(a) = state.notebooks.get_mut(id) {
            if a.book_id != b.book_id || a.pass_id != b.pass_id {
                return Err(Error::Conflict("Notebook identity changed".into()));
            }
            for block in &b.blocks {
                match a.blocks.iter_mut().find(|known| known.id == block.id) {
                    Some(known) => {
                        *known = history::merge_block_with_base(
                            base.and_then(|s| s.notebooks.get(id))
                                .and_then(|n| n.blocks.iter().find(|b| b.id == block.id)),
                            known,
                            block,
                        )?
                    }
                    None => a.blocks.push(block.clone()),
                }
            }
            for revision in &b.revisions {
                if let Some(known) = a.revisions.iter().find(|known| known.id == revision.id) {
                    if known != revision {
                        return Err(Error::Conflict("Notebook revision ID reused".into()));
                    }
                } else {
                    a.revisions.push(revision.clone());
                }
            }
            a.revisions
                .sort_by(|a, b| (a.at, &a.id).cmp(&(b.at, &b.id)));
        } else {
            state.notebooks.insert(id.clone(), b.clone());
        }
    }
    for (id, b) in &incoming.cards {
        let merged = match state.cards.get(id) {
            Some(a) => history::merge_card(a, b)?,
            None => b.clone(),
        };
        state.cards.insert(id.clone(), merged);
    }
    state
        .friendship_requests
        .extend(incoming.friendship_requests.iter().cloned());
    state.blocked.extend(incoming.blocked.iter().cloned());
    state.revision = local
        .revision
        .checked_add(1)
        .ok_or_else(|| Error::Invalid("Revision exhausted".into()))?;
    crate::validate(&state)?;
    Ok(MergeReport { state, conflicts })
}
/// Resolution is optimistic: a subsequent edit must be reviewed again, never overwritten.
pub fn resolve(state: &State, conflict: &Conflict, use_incoming: bool) -> Result<State> {
    let mut json = value(state)?;
    let collection = json
        .get_mut(&conflict.collection)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| Error::Invalid("Unknown conflict collection".into()))?;
    let record = collection
        .get_mut(&conflict.record_id)
        .ok_or_else(|| Error::NotFound("Conflict record".into()))?;
    let target = if conflict.field.is_empty() {
        record
    } else {
        record
            .get_mut(&conflict.field)
            .ok_or_else(|| Error::Invalid("Unknown conflict field".into()))?
    };
    if use_incoming && *target != conflict.local {
        return Err(Error::Conflict(
            "This value changed since conflict review; reload it before resolving".into(),
        ));
    }
    if use_incoming {
        *target = conflict.incoming.clone();
    }
    let mut result: State = serde_json::from_value(json)?;
    result.revision = state
        .revision
        .checked_add(1)
        .ok_or_else(|| Error::Invalid("Revision exhausted".into()))?;
    crate::validate(&result)?;
    Ok(result)
}
