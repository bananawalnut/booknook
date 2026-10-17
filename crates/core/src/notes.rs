//! Note origin is immutable provenance, independent of the current face and edit history.
use crate::{Error, Result, model::*};

pub fn promote(state: &mut State) {
    state.schema = NOTE_ARCHIVE_SCHEMA.into();
    state.ontology_version = NOTE_ONTOLOGY.into();
}
pub fn capture(state: &State, notebook: &Notebook) -> CapturedReadingContext {
    let observations = crate::latest_positions(state, &notebook.pass_id);
    CapturedReadingContext {
        pass_id: notebook.pass_id.clone(),
        positions: observations.iter().map(|p| p.location.clone()).collect(),
        position_observations: observations,
    }
}
pub fn validate_context(
    state: &State,
    notebook: &Notebook,
    context: &CapturedReadingContext,
) -> Result<()> {
    if context.pass_id != notebook.pass_id
        || context.positions.len() > 256
        || context.position_observations.len() > 256
    {
        return Err(Error::Invalid("Invalid captured reading context".into()));
    }
    for location in &context.positions {
        crate::validation::validate_book_location(state, location, &notebook.book_id)?;
    }
    for observation in &context.position_observations {
        if observation.pass_id != notebook.pass_id
            || observation.id.is_empty()
            || observation
                .provider_connection_id
                .as_deref()
                .is_some_and(|id| !crate::castalia_connections::valid_reference(id))
        {
            return Err(Error::Invalid(
                "Invalid captured position observation".into(),
            ));
        }
        crate::validation::validate_book_location(state, &observation.location, &notebook.book_id)?;
        if let Alignment::Candidate { edition_id, .. } | Alignment::Confirmed { edition_id, .. } =
            &observation.alignment
            && state
                .editions
                .get(edition_id)
                .is_none_or(|edition| edition.book_id != notebook.book_id)
        {
            return Err(Error::Invalid(
                "Captured alignment belongs to another book".into(),
            ));
        }
        if let Some(metadata) = &observation.audio_metadata {
            if !matches!(observation.location, Location::Audio { .. }) {
                return Err(Error::Invalid(
                    "Audio metadata requires audio location".into(),
                ));
            }
            metadata.validate()?;
        }
    }
    Ok(())
}
fn origin(
    source: NoteOriginSource,
    at: Option<u64>,
    reader: String,
    context: CapturedReadingContext,
) -> NoteOrigin {
    NoteOrigin {
        source,
        at,
        reader,
        pass_id: context.pass_id,
        positions: context.positions,
        position_observations: context.position_observations,
    }
}
fn prior_origin(state: &State, notebook: &Notebook, block: &Block) -> NoteOrigin {
    let empty = || CapturedReadingContext {
        pass_id: notebook.pass_id.clone(),
        positions: vec![],
        position_observations: vec![],
    };
    // A historical time is evidence of a surviving edit, never a claim about original creation.
    let mut candidates: Vec<NoteOrigin> = notebook
        .revisions
        .iter()
        .filter(|r| {
            r.block_id == block.id
                && crate::notebook::text(&r.update).is_ok_and(|text| !text.trim().is_empty())
        })
        .map(|r| {
            origin(
                NoteOriginSource::HistoryFallback,
                Some(r.at),
                r.reader.clone(),
                empty(),
            )
        })
        .collect();
    if let Some(ink) = &block.ink {
        for v in &ink.versions {
            candidates.push(origin(
                NoteOriginSource::HistoryFallback,
                Some(v.at),
                v.reader.clone(),
                CapturedReadingContext {
                    pass_id: v.pass_id.clone(),
                    positions: v.positions.clone(),
                    position_observations: v.position_observations.clone(),
                },
            ));
        }
    }
    for card in state
        .cards
        .values()
        .filter(|c| c.notebook_id == notebook.id && c.block_id == block.id)
    {
        for v in &card.versions {
            candidates.push(origin(
                NoteOriginSource::HistoryFallback,
                Some(v.at),
                v.reader.clone(),
                CapturedReadingContext {
                    pass_id: v.pass_id.clone(),
                    positions: v.positions.clone(),
                    position_observations: v.position_observations.clone(),
                },
            ));
        }
    }
    candidates.sort_by_key(|o| o.at);
    if let Some(first) = candidates.into_iter().next() {
        return first;
    }
    let source = if !block.text.trim().is_empty() || block.ink.is_some() {
        NoteOriginSource::Unknown
    } else {
        NoteOriginSource::Pending
    };
    origin(source, None, state.local_reader.clone(), empty())
}
/// Opt-in only: older commands do not acquire new fields or silently upgrade an archive.
#[allow(clippy::too_many_arguments)]
pub fn initialize(
    state: &mut State,
    notebook_id: &str,
    block_id: &str,
    at: u64,
    context: Option<&CapturedReadingContext>,
    creation: Option<&NoteCreation>,
    meaningful: bool,
    face: Option<NoteFace>,
) -> Result<()> {
    let notebook = state
        .notebooks
        .get(notebook_id)
        .ok_or_else(|| Error::NotFound(notebook_id.into()))?;
    let block = notebook
        .blocks
        .iter()
        .find(|b| b.id == block_id)
        .ok_or_else(|| Error::NotFound(block_id.into()))?;
    if let Some(context) = context {
        validate_context(state, notebook, context)?;
    }
    if let Some(creation) = creation {
        validate_context(state, notebook, &creation.capture_context)?;
        if creation.at > at {
            return Err(Error::Invalid("Note creation occurs after its edit".into()));
        }
    }
    let mut note = block.note.clone().unwrap_or_else(|| NoteMetadata {
        schema: NOTE_SCHEMA.into(),
        face: NoteFace::Text,
        created: prior_origin(state, notebook, block),
    });
    if note.created.source == NoteOriginSource::Pending {
        let prior = prior_origin(state, notebook, block);
        if prior.source != NoteOriginSource::Pending {
            note.created = prior;
        }
    }
    if (meaningful || creation.is_some()) && note.created.source == NoteOriginSource::Pending {
        let captured = creation
            .map(|c| c.capture_context.clone())
            .or_else(|| context.cloned())
            .unwrap_or_else(|| capture(state, notebook));
        note.created = origin(
            NoteOriginSource::Recorded,
            Some(creation.map_or(at, |c| c.at)),
            state.local_reader.clone(),
            captured,
        );
    }
    if let Some(face) = face {
        note.face = face;
    }
    state
        .notebooks
        .get_mut(notebook_id)
        .unwrap()
        .blocks
        .iter_mut()
        .find(|b| b.id == block_id)
        .unwrap()
        .note = Some(note);
    promote(state);
    Ok(())
}
pub fn validate(state: &State, notebook: &Notebook, note: &NoteMetadata) -> Result<()> {
    if note.schema != NOTE_SCHEMA
        || note.created.reader.is_empty()
        || matches!(
            note.created.source,
            NoteOriginSource::Recorded | NoteOriginSource::HistoryFallback
        ) != note.created.at.is_some()
        || (matches!(
            note.created.source,
            NoteOriginSource::Pending | NoteOriginSource::Unknown
        ) && (!note.created.positions.is_empty()
            || !note.created.position_observations.is_empty()))
    {
        return Err(Error::Invalid("Invalid note origin".into()));
    }
    validate_context(
        state,
        notebook,
        &CapturedReadingContext {
            pass_id: note.created.pass_id.clone(),
            positions: note.created.positions.clone(),
            position_observations: note.created.position_observations.clone(),
        },
    )
}
pub fn merge(
    base: Option<&NoteMetadata>,
    local: Option<&NoteMetadata>,
    incoming: Option<&NoteMetadata>,
) -> Result<Option<NoteMetadata>> {
    let (Some(a), Some(b)) = (local, incoming) else {
        return Ok(local.or(incoming).cloned());
    };
    let mut merged = a.clone();
    if a.created != b.created {
        // Pending is explicitly unrecorded, so a recorded first input can extend it.
        if a.created.source == NoteOriginSource::Pending {
            merged.created = b.created.clone();
        } else if b.created.source != NoteOriginSource::Pending {
            return Err(Error::Conflict(
                "Note creation contexts differ; both replicas must be retained for review".into(),
            ));
        }
    }
    if a.face != b.face {
        if base.is_some_and(|old| old.face == a.face) {
            merged.face = b.face.clone();
        } else if !base.is_some_and(|old| old.face == b.face) {
            return Err(Error::Conflict("Note faces differ without a common base; select a face before retrying synchronization".into()));
        }
    }
    Ok(Some(merged))
}
