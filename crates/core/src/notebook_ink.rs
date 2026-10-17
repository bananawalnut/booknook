//! Immutable notebook canvas snapshots, independent of the prose CRDT and reader annotations.
use crate::{Error, Result, model::*};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn require(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Error::Invalid(message.into()))
    }
}
fn valid_id(id: &str) -> bool {
    crate::castalia_connections::valid_reference(id)
}
fn ink_result(ink: &NotebookInk, id: &str, replayed: bool) -> Value {
    let heads: Vec<&str> = ink.heads().into_iter().map(|v| v.id.as_str()).collect();
    json!({"ink_version_id":id, "conflict":heads.len() > 1, "heads":heads, "replayed":replayed})
}

/// Save a full canvas snapshot. Parents are the versions the editor actually saw, not latest heads.
#[allow(clippy::too_many_arguments)]
pub fn save(
    state: &mut State,
    notebook_id: &str,
    block_id: &str,
    version_id: Option<String>,
    parents: Vec<String>,
    canvas_width: f64,
    canvas_height: f64,
    strokes: Vec<Stroke>,
    native_drawing: Option<String>,
    at: u64,
    capture_context: Option<CapturedReadingContext>,
    note_creation: Option<NoteCreation>,
    canvas_space: Option<CanvasSpace>,
) -> Result<Value> {
    let notebook = state
        .notebooks
        .get(notebook_id)
        .ok_or_else(|| Error::NotFound(notebook_id.into()))?;
    let block = notebook
        .blocks
        .iter()
        .find(|b| b.id == block_id)
        .ok_or_else(|| Error::NotFound(block_id.into()))?;
    if let Some(context) = &capture_context {
        crate::notes::validate_context(state, notebook, context)?;
    }
    if let Some(creation) = &note_creation {
        crate::notes::validate_context(state, notebook, &creation.capture_context)?;
        if creation.at > at {
            return Err(Error::Invalid(
                "Note creation occurs after its drawing".into(),
            ));
        }
    }
    let id = version_id.unwrap_or_else(crate::new_id);
    require(valid_id(&id), "Invalid ink version ID")?;
    if let Some(ink) = &block.ink
        && let Some(known) = ink.versions.iter().find(|v| v.id == id)
    {
        // Positions were captured by the first commit. A retry must not capture later positions.
        if known.parents != parents
            || known.reader != state.local_reader
            || known.at != at
            || known.canvas_width != canvas_width
            || known.canvas_height != canvas_height
            || known.strokes != strokes
            || known.native_drawing != native_drawing
            || known.canvas_space != canvas_space
            || capture_context.as_ref().is_some_and(|c| {
                known.pass_id != c.pass_id
                    || known.positions != c.positions
                    || known.position_observations != c.position_observations
            })
        {
            return Err(Error::Conflict(
                "Ink version ID reused with different content".into(),
            ));
        }
        return Ok(ink_result(ink, &id, true));
    }
    let captured = capture_context
        .clone()
        .unwrap_or_else(|| crate::notes::capture(state, notebook));
    let opt_in = capture_context.is_some() || note_creation.is_some() || canvas_space.is_some();
    let meaningful = !strokes.is_empty();
    let version = NotebookInkVersion {
        id: id.clone(),
        parents,
        reader: state.local_reader.clone(),
        at,
        pass_id: captured.pass_id,
        positions: captured.positions,
        position_observations: captured.position_observations,
        canvas_width,
        canvas_height,
        canvas_space,
        strokes,
        native_drawing,
    };
    let mut ink = block.ink.clone().unwrap_or_else(|| NotebookInk {
        schema: NOTEBOOK_INK_SCHEMA.into(),
        versions: vec![],
        transcriptions: vec![],
    });
    ink.versions.push(version);
    validate(state, notebook, &ink)?;
    let result = ink_result(&ink, &id, false);
    if opt_in {
        crate::notes::initialize(
            state,
            notebook_id,
            block_id,
            at,
            capture_context.as_ref(),
            note_creation.as_ref(),
            meaningful,
            None,
        )?;
    }
    state
        .notebooks
        .get_mut(notebook_id)
        .unwrap()
        .blocks
        .iter_mut()
        .find(|b| b.id == block_id)
        .unwrap()
        .ink = Some(ink);
    if state.schema != NOTE_ARCHIVE_SCHEMA {
        state.schema = INK_ARCHIVE_SCHEMA.into();
        state.ontology_version = INK_ONTOLOGY.into();
    }
    Ok(result)
}

/// A transcription references an exact drawing; it never changes ink, prose or previous transcripts.
#[allow(clippy::too_many_arguments)]
pub fn transcribe(
    state: &mut State,
    notebook_id: &str,
    block_id: &str,
    transcription_id: Option<String>,
    ink_version_id: String,
    text: String,
    method: TranscriptionMethod,
    at: u64,
) -> Result<Value> {
    let notebook = state
        .notebooks
        .get(notebook_id)
        .ok_or_else(|| Error::NotFound(notebook_id.into()))?;
    let block = notebook
        .blocks
        .iter()
        .find(|b| b.id == block_id)
        .ok_or_else(|| Error::NotFound(block_id.into()))?;
    let mut ink = block
        .ink
        .clone()
        .ok_or_else(|| Error::NotFound("Notebook ink".into()))?;
    let id = transcription_id.unwrap_or_else(crate::new_id);
    let transcript = NotebookTranscription {
        id: id.clone(),
        ink_version_id,
        reader: state.local_reader.clone(),
        at,
        text,
        method,
    };
    if let Some(known) = ink.transcriptions.iter().find(|t| t.id == id) {
        if known != &transcript {
            return Err(Error::Conflict(
                "Transcription ID reused with different content".into(),
            ));
        }
        return Ok(json!({"transcription_id":id, "replayed":true}));
    }
    ink.transcriptions.push(transcript);
    validate(state, notebook, &ink)?;
    state
        .notebooks
        .get_mut(notebook_id)
        .unwrap()
        .blocks
        .iter_mut()
        .find(|b| b.id == block_id)
        .unwrap()
        .ink = Some(ink);
    Ok(json!({"transcription_id":id, "replayed":false}))
}

pub fn validate(state: &State, notebook: &Notebook, ink: &NotebookInk) -> Result<()> {
    require(
        ink.schema == NOTEBOOK_INK_SCHEMA && !ink.versions.is_empty(),
        "Invalid notebook ink schema/history",
    )?;
    let mut seen = BTreeSet::new();
    for v in &ink.versions {
        require(
            valid_id(&v.id)
                && !seen.contains(&v.id)
                && !v.reader.is_empty()
                && v.pass_id == notebook.pass_id,
            "Invalid ink version identity/context",
        )?;
        let parents: BTreeSet<_> = v.parents.iter().collect();
        require(
            parents.len() == v.parents.len() && v.parents.iter().all(|p| seen.contains(p)),
            "Duplicate, missing or cyclic ink parents",
        )?;
        seen.insert(v.id.clone());
        let valid_canvas = match &v.canvas_space {
            None => {
                (1.0..=16384.0).contains(&v.canvas_width)
                    && (1.0..=16384.0).contains(&v.canvas_height)
            }
            Some(CanvasSpace::Whiteboard { origin_x, origin_y }) => {
                origin_x.is_finite()
                    && origin_y.is_finite()
                    && (1.0..=2_000_000.0).contains(&v.canvas_width)
                    && (1.0..=2_000_000.0).contains(&v.canvas_height)
                    && origin_x.abs() <= 1_000_000.0
                    && origin_y.abs() <= 1_000_000.0
                    && (origin_x + v.canvas_width).abs() <= 1_000_000.0
                    && (origin_y + v.canvas_height).abs() <= 1_000_000.0
            }
        };
        require(
            v.canvas_width.is_finite() && v.canvas_height.is_finite() && valid_canvas,
            "Invalid notebook canvas dimensions/world bounds",
        )?;
        require(v.strokes.len() <= 10000, "Too many notebook strokes")?;
        let mut point_count = 0usize;
        for stroke in &v.strokes {
            point_count = point_count.saturating_add(stroke.points.len());
            require(
                !stroke.points.is_empty()
                    && point_count <= 1_000_000
                    && stroke.width.is_finite()
                    && stroke.width > 0.0
                    && stroke.width <= 1024.0
                    && stroke.color.len() == 7
                    && stroke.color.starts_with('#')
                    && stroke.color.as_bytes()[1..]
                        .iter()
                        .all(u8::is_ascii_hexdigit),
                "Invalid notebook stroke",
            )?;
            for p in &stroke.points {
                // Preserve control points outside the canvas edge; native renderers clip to the canvas.
                require(
                    p.x.is_finite()
                        && p.y.is_finite()
                        && p.x.abs() <= 1_000_000.0
                        && p.y.abs() <= 1_000_000.0
                        && p.pressure.is_finite()
                        && (0.0..=1.0).contains(&p.pressure)
                        && p.time.is_finite()
                        && p.time >= 0.0,
                    "Invalid notebook stroke point",
                )?;
            }
        }
        if let Some(native) = &v.native_drawing {
            require(
                native.len() <= (8 * 1024 * 1024_usize).div_ceil(3) * 4,
                "Native notebook drawing exceeds 8 MiB",
            )?;
            let bytes = STANDARD
                .decode(native)
                .map_err(|_| Error::Invalid("Invalid native drawing base64".into()))?;
            require(
                bytes.len() <= 8 * 1024 * 1024 && STANDARD.encode(&bytes) == *native,
                "Noncanonical or oversized native drawing",
            )?;
        }
        for p in &v.positions {
            crate::validation::validate_book_location(state, p, &notebook.book_id)?;
        }
        for o in &v.position_observations {
            require(
                o.pass_id == v.pass_id
                    && o.provider_connection_id
                        .as_deref()
                        .is_none_or(crate::castalia_connections::valid_reference),
                "Invalid ink position context",
            )?;
            crate::validation::validate_book_location(state, &o.location, &notebook.book_id)?;
            if let Alignment::Candidate { edition_id, .. }
            | Alignment::Confirmed { edition_id, .. } = &o.alignment
            {
                require(
                    state
                        .editions
                        .get(edition_id)
                        .is_some_and(|e| e.book_id == notebook.book_id),
                    "Ink position alignment references another book",
                )?;
            }
            if let Some(metadata) = &o.audio_metadata {
                require(
                    matches!(o.location, Location::Audio { .. }),
                    "Audio metadata requires audio location",
                )?;
                metadata.validate()?;
            }
        }
    }
    let mut transcripts = BTreeSet::new();
    for t in &ink.transcriptions {
        require(
            valid_id(&t.id)
                && transcripts.insert(&t.id)
                && seen.contains(&t.ink_version_id)
                && !t.reader.is_empty()
                && !t.text.trim().is_empty()
                && t.text.len() <= 1024 * 1024,
            "Invalid notebook transcription/source",
        )?;
    }
    Ok(())
}

/// Union immutable snapshots and transcripts. Absent/older replicas cannot delete handwriting.
pub fn merge(
    local: Option<&NotebookInk>,
    incoming: Option<&NotebookInk>,
) -> Result<Option<NotebookInk>> {
    let (Some(local), Some(incoming)) = (local, incoming) else {
        return Ok(local.or(incoming).cloned());
    };
    if local.schema != incoming.schema {
        return Err(Error::Conflict("Ink schema mismatch".into()));
    }
    let mut versions = BTreeMap::new();
    for version in local.versions.iter().chain(&incoming.versions) {
        if let Some(known) = versions.insert(version.id.clone(), version.clone())
            && known != *version
        {
            return Err(Error::Conflict(
                "Ink version ID reused with different content".into(),
            ));
        }
    }
    let mut merged = NotebookInk {
        schema: local.schema.clone(),
        versions: vec![],
        transcriptions: vec![],
    };
    let mut seen = BTreeSet::new();
    while !versions.is_empty() {
        let ready: Vec<_> = versions
            .iter()
            .filter(|(_, v)| v.parents.iter().all(|p| seen.contains(p)))
            .map(|(id, _)| id.clone())
            .collect();
        if ready.is_empty() {
            return Err(Error::Invalid("Missing or cyclic ink parents".into()));
        }
        for id in ready {
            seen.insert(id.clone());
            merged.versions.push(versions.remove(&id).unwrap());
        }
    }
    let mut transcripts = BTreeMap::new();
    for transcript in local.transcriptions.iter().chain(&incoming.transcriptions) {
        if let Some(known) = transcripts.insert(transcript.id.clone(), transcript.clone())
            && known != *transcript
        {
            return Err(Error::Conflict(
                "Transcription ID reused with different content".into(),
            ));
        }
    }
    merged.transcriptions = transcripts.into_values().collect();
    Ok(Some(merged))
}
