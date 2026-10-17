pub mod anchors;
pub mod audio;
pub mod authority;
pub mod castalia_connections;
mod character_sharing;
pub mod history;
pub mod identity;
pub mod imports;
pub mod model;
pub mod notebook;
pub mod notebook_ink;
pub mod notes;
pub mod portable_archive;
pub mod provider_runtime;
pub mod providers;
pub mod reconcile;
pub mod settings;
pub mod social;
pub mod social_protocol;
pub mod sync;
mod validation;
use model::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
pub use validation::{MAX_JSON_BYTES, strict_parse, validate};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Invalid data: {0}")]
    Invalid(String),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Conflict: {0}")]
    Conflict(String),
    #[error("Authorization denied: {0}")]
    Denied(String),
    #[error("Unavailable: {0}")]
    Unavailable(String),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
pub type Result<T> = std::result::Result<T, Error>;
pub fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
pub fn new_id() -> String {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).expect("OS entropy is required; refuse predictable IDs");
    hex::encode(bytes)
}
pub fn canonical(value: &impl Serialize) -> Result<Vec<u8>> {
    // serde_json::Map uses BTreeMap without preserve_order. No floating point in signed contracts.
    Ok(serde_json::to_vec(&serde_json::to_value(value)?)?)
}

// Keep public Rust command constructors stable; large commands cross JSON/FFI boundaries.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    ImportGoodreads {
        csv: String,
        at: u64,
    },
    ImportKindle {
        json: String,
        at: u64,
    },
    ImportKindleFromConnection {
        json: String,
        at: u64,
        provider_connection_id: String,
    },
    AddBook {
        title: String,
        author: String,
        at: u64,
    },
    UpdateBook {
        book_id: String,
        title: String,
        shelf: String,
        rating: Option<u8>,
        review: String,
    },
    SetBookSubjects {
        book_id: String,
        subjects: Vec<String>,
    },
    OpenNotebook {
        book_id: String,
        at: u64,
    },
    StartPass {
        book_id: String,
        at: u64,
    },
    StartSession {
        pass_id: String,
        at: u64,
    },
    EndSession {
        session_id: String,
        at: u64,
    },
    CreateLiteraryEntity {
        book_id: String,
        kind: LiteraryKind,
        name: String,
    },
    AddBlock {
        notebook_id: String,
        at: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        capture_context: Option<CapturedReadingContext>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        face: Option<NoteFace>,
    },
    EditBlock {
        notebook_id: String,
        block_id: String,
        text: String,
        base_update: String,
        at: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        capture_context: Option<CapturedReadingContext>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note_creation: Option<NoteCreation>,
    },
    MergeBlockUpdate {
        notebook_id: String,
        block_id: String,
        update: String,
        at: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        capture_context: Option<CapturedReadingContext>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note_creation: Option<NoteCreation>,
    },
    EditRichBlock {
        notebook_id: String,
        block_id: String,
        text: String,
        base_update: String,
        styles: Vec<InlineStyle>,
        at: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        capture_context: Option<CapturedReadingContext>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note_creation: Option<NoteCreation>,
    },
    SetNotebookInk {
        notebook_id: String,
        block_id: String,
        version_id: Option<String>,
        parents: Vec<String>,
        canvas_width: f64,
        canvas_height: f64,
        strokes: Vec<Stroke>,
        native_drawing: Option<String>,
        at: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        capture_context: Option<CapturedReadingContext>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note_creation: Option<NoteCreation>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        canvas_space: Option<CanvasSpace>,
    },
    SetNoteFace {
        notebook_id: String,
        block_id: String,
        face: NoteFace,
        at: u64,
    },
    SaveNotebookTranscription {
        notebook_id: String,
        block_id: String,
        transcription_id: Option<String>,
        ink_version_id: String,
        text: String,
        method: TranscriptionMethod,
        at: u64,
    },
    AddCharacter {
        notebook_id: String,
        block_id: String,
        name: String,
        at: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        capture_context: Option<CapturedReadingContext>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note_creation: Option<NoteCreation>,
    },
    AttachCharacter {
        notebook_id: String,
        block_id: String,
        character_id: String,
        at: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        capture_context: Option<CapturedReadingContext>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note_creation: Option<NoteCreation>,
    },
    SaveCard {
        card_id: String,
        parents: Vec<String>,
        interpretation: CharacterInterpretation,
        positions: Vec<Location>,
        at: u64,
    },
    SaveAnnotation {
        annotation: Annotation,
    },
    ObservePosition {
        observation: ProgressObservation,
    },
    SaveList {
        id: Option<String>,
        title: String,
        book_ids: Vec<String>,
        expected_version: Option<u64>,
    },
    CreateGroup {
        name: String,
        private: bool,
    },
    BlockActor {
        actor: String,
    },
    PrepareShare {
        kind: String,
        book_id: Option<String>,
        body: String,
        spoiler: bool,
        at: u64,
    },
    PrepareCharacterShare {
        card_id: String,
        version_id: String,
        at: u64,
    },
    RemoveDraft {
        id: String,
    },
}

pub fn ensure_notebook(state: &mut State, book_id: &str, at: u64) -> Result<Notebook> {
    if !state.books.contains_key(book_id) {
        return Err(Error::NotFound(book_id.into()));
    }
    if let Some(n) = state
        .notebooks
        .values()
        .filter(|n| n.book_id == book_id)
        .max_by_key(|n| state.passes.get(&n.pass_id).map(|p| p.started_at))
    {
        return Ok(n.clone());
    }
    start_pass(state, book_id, at)
}

fn start_pass(state: &mut State, book_id: &str, at: u64) -> Result<Notebook> {
    if !state.books.contains_key(book_id) {
        return Err(Error::NotFound(book_id.into()));
    }
    let pass = ReadingPass {
        id: new_id(),
        book_id: book_id.into(),
        reader: state.local_reader.clone(),
        started_at: at,
        finished_at: None,
    };
    let notebook = Notebook {
        id: new_id(),
        book_id: book_id.into(),
        pass_id: pass.id.clone(),
        blocks: vec![Block {
            id: new_id(),
            kind: "paragraph".into(),
            text: String::new(),
            update: notebook::edit("", "")?,
            styles: vec![],
            ink: None,
            note: None,
        }],
        revisions: vec![],
    };
    state.passes.insert(pass.id.clone(), pass);
    state
        .notebooks
        .insert(notebook.id.clone(), notebook.clone());
    Ok(notebook)
}

/// Atomic at the contract boundary: invalid commands never partially modify the caller's state.
pub fn apply(state: &mut State, command: Command) -> Result<serde_json::Value> {
    let mut next = state.clone();
    let result = execute(&mut next, command)?;
    // An acknowledged immutable retry has no storage or revision side effects.
    if result.get("replayed").and_then(serde_json::Value::as_bool) == Some(true) {
        validate(&next)?;
        return Ok(result);
    }
    next.revision = state
        .revision
        .checked_add(1)
        .ok_or_else(|| Error::Invalid("Revision overflow".into()))?;
    validate(&next)?;
    *state = next;
    Ok(result)
}

fn execute(state: &mut State, command: Command) -> Result<serde_json::Value> {
    use serde_json::json;
    match command {
        Command::ImportGoodreads { csv, at } => {
            Ok(serde_json::to_value(imports::goodreads(state, &csv, at)?)?)
        }
        Command::ImportKindle { json, at } => {
            Ok(serde_json::to_value(imports::kindle(state, &json, at)?)?)
        }
        Command::ImportKindleFromConnection {
            json,
            at,
            provider_connection_id,
        } => Ok(serde_json::to_value(imports::kindle_from_connection(
            state,
            &json,
            at,
            Some(&provider_connection_id),
        )?)?),
        Command::AddBook { title, author, at } => {
            if title.trim().is_empty() {
                return Err(Error::Invalid("A title is required".into()));
            }
            let id = new_id();
            let edition_id = new_id();
            state.books.insert(
                id.clone(),
                Book {
                    id: id.clone(),
                    title,
                    authors: vec![author],
                    subjects: vec![],
                    edition_ids: vec![edition_id.clone()],
                    sources: vec![],
                    shelf: "currently-reading".into(),
                    rating: None,
                    review: String::new(),
                    reading_dates: vec![],
                    edited_fields: BTreeSet::new(),
                    provenance: vec![],
                },
            );
            state.editions.insert(
                edition_id.clone(),
                Edition {
                    id: edition_id,
                    book_id: id.clone(),
                    identifiers: vec![],
                    language: None,
                    assets: vec![],
                },
            );
            ensure_notebook(state, &id, at)?;
            Ok(json!({"book_id":id}))
        }
        Command::UpdateBook {
            book_id,
            title,
            shelf,
            rating,
            review,
        } => {
            if title.trim().is_empty() || rating.is_some_and(|r| !(1..=5).contains(&r)) {
                return Err(Error::Invalid("Invalid title or rating".into()));
            }
            let b = state
                .books
                .get_mut(&book_id)
                .ok_or(Error::NotFound(book_id))?;
            b.title = title;
            b.shelf = shelf;
            b.rating = rating;
            b.review = review;
            b.edited_fields
                .extend(["title", "shelf", "rating", "review"].map(str::to_owned));
            Ok(json!({}))
        }
        Command::SetBookSubjects { book_id, subjects } => {
            if subjects.len() > 32 || subjects.iter().any(|s| s.chars().count() > 128) {
                return Err(Error::Invalid(
                    "Use at most 32 subjects of 128 characters each".into(),
                ));
            }
            let b = state
                .books
                .get_mut(&book_id)
                .ok_or(Error::NotFound(book_id))?;
            b.subjects = subjects
                .into_iter()
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            b.edited_fields.insert("subjects".into());
            Ok(json!({}))
        }
        Command::OpenNotebook { book_id, at } => {
            Ok(serde_json::to_value(ensure_notebook(state, &book_id, at)?)?)
        }
        Command::StartPass { book_id, at } => {
            Ok(serde_json::to_value(start_pass(state, &book_id, at)?)?)
        }
        Command::StartSession { pass_id, at } => {
            let pass = state
                .passes
                .get(&pass_id)
                .ok_or_else(|| Error::NotFound(pass_id.clone()))?;
            if at < pass.started_at || pass.finished_at.is_some_and(|end| at > end) {
                return Err(Error::Invalid(
                    "Session must start within its reading pass".into(),
                ));
            }
            let id = new_id();
            state.sessions.insert(
                id.clone(),
                ReadingSession {
                    id: id.clone(),
                    pass_id,
                    started_at: at,
                    ended_at: None,
                },
            );
            Ok(json!({"session_id":id}))
        }
        Command::EndSession { session_id, at } => {
            let session = state
                .sessions
                .get_mut(&session_id)
                .ok_or_else(|| Error::NotFound(session_id.clone()))?;
            let pass = state
                .passes
                .get(&session.pass_id)
                .ok_or_else(|| Error::Invalid("Session has no reading pass".into()))?;
            if at < session.started_at || pass.finished_at.is_some_and(|end| at > end) {
                return Err(Error::Invalid(
                    "Session end is outside its reading interval".into(),
                ));
            }
            if session.ended_at.is_some_and(|ended| ended != at) {
                return Err(Error::Conflict(
                    "Session has already ended; its recorded duration is preserved".into(),
                ));
            }
            session.ended_at = Some(at);
            Ok(json!({"session_id":session_id}))
        }
        Command::CreateLiteraryEntity {
            book_id,
            kind,
            name,
        } => {
            if !state.books.contains_key(&book_id) {
                return Err(Error::NotFound(book_id));
            }
            let name = name.trim().to_owned();
            if name.is_empty() || name.chars().count() > 512 {
                return Err(Error::Invalid(
                    "Literary entities need a name of at most 512 characters".into(),
                ));
            }
            if kind == LiteraryKind::Character {
                return Err(Error::Invalid(
                    "Create a character with add_character to attach its first interpretation card"
                        .into(),
                ));
            }
            let id = new_id();
            state.entities.insert(
                id.clone(),
                LiteraryEntity {
                    id: id.clone(),
                    book_id,
                    kind,
                    name,
                },
            );
            Ok(json!({"entity_id":id}))
        }
        Command::AddBlock {
            notebook_id,
            at,
            capture_context,
            face,
        } => {
            let n = state
                .notebooks
                .get_mut(&notebook_id)
                .ok_or(Error::NotFound(notebook_id.clone()))?;
            let id = new_id();
            n.blocks.push(Block {
                id: id.clone(),
                kind: "paragraph".into(),
                text: String::new(),
                update: notebook::edit("", "")?,
                styles: vec![],
                ink: None,
                note: None,
            });
            if capture_context.is_some() || face.is_some() {
                notes::initialize(
                    state,
                    &notebook_id,
                    &id,
                    at,
                    capture_context.as_ref(),
                    None,
                    true,
                    face,
                )?;
            }
            Ok(json!({"block_id":id}))
        }
        Command::EditBlock {
            notebook_id,
            block_id,
            text,
            base_update,
            at,
            capture_context,
            note_creation,
        } => {
            if capture_context.is_some() || note_creation.is_some() {
                notes::initialize(
                    state,
                    &notebook_id,
                    &block_id,
                    at,
                    capture_context.as_ref(),
                    note_creation.as_ref(),
                    !text.trim().is_empty(),
                    None,
                )?;
            }
            if text.len() > 8 * 1024 * 1024 {
                return Err(Error::Invalid("Block exceeds 8 MiB".into()));
            }
            let n = state
                .notebooks
                .get_mut(&notebook_id)
                .ok_or(Error::NotFound(notebook_id))?;
            let b = n
                .blocks
                .iter_mut()
                .find(|b| b.id == block_id)
                .ok_or(Error::NotFound(block_id.clone()))?;
            // A stale editor must derive its edit from what it actually saw, then merge.
            let incoming = notebook::edit(&base_update, &text)?;
            let (update, merged) = notebook::merge(&b.update, &incoming)?;
            if b.text != merged || b.update != update {
                b.update = update.clone();
                b.styles = notebook::styles(&update)?;
                b.text = merged;
                n.revisions.push(NotebookRevision {
                    id: new_id(),
                    block_id,
                    reader: state.local_reader.clone(),
                    at,
                    update,
                });
            }
            Ok(json!({"text":b.text,"update":b.update}))
        }
        Command::MergeBlockUpdate {
            notebook_id,
            block_id,
            update: incoming,
            at,
            capture_context,
            note_creation,
        } => {
            if capture_context.is_some() || note_creation.is_some() {
                notes::initialize(
                    state,
                    &notebook_id,
                    &block_id,
                    at,
                    capture_context.as_ref(),
                    note_creation.as_ref(),
                    !notebook::text(&incoming)?.trim().is_empty(),
                    None,
                )?;
            }
            let n = state
                .notebooks
                .get_mut(&notebook_id)
                .ok_or(Error::NotFound(notebook_id))?;
            let b = n
                .blocks
                .iter_mut()
                .find(|b| b.id == block_id)
                .ok_or(Error::NotFound(block_id.clone()))?;
            let (update, merged) = notebook::merge(&b.update, &incoming)?;
            if merged.len() > 8 * 1024 * 1024 {
                return Err(Error::Invalid("Block exceeds 8 MiB".into()));
            }
            if b.update != update {
                b.text = merged;
                b.styles = notebook::styles(&update)?;
                b.update = update.clone();
                n.revisions.push(NotebookRevision {
                    id: new_id(),
                    block_id,
                    reader: state.local_reader.clone(),
                    at,
                    update,
                });
            }
            Ok(json!({"text":b.text,"update":b.update,"styles":b.styles}))
        }
        Command::EditRichBlock {
            notebook_id,
            block_id,
            text,
            base_update,
            styles,
            at,
            capture_context,
            note_creation,
        } => {
            if capture_context.is_some() || note_creation.is_some() {
                notes::initialize(
                    state,
                    &notebook_id,
                    &block_id,
                    at,
                    capture_context.as_ref(),
                    note_creation.as_ref(),
                    !text.trim().is_empty(),
                    None,
                )?;
            }
            if text.len() > 8 * 1024 * 1024 {
                return Err(Error::Invalid("Block exceeds 8 MiB".into()));
            }
            let incoming = notebook::rich_edit(&base_update, &text, &styles)?;
            let n = state
                .notebooks
                .get_mut(&notebook_id)
                .ok_or(Error::NotFound(notebook_id))?;
            let b = n
                .blocks
                .iter_mut()
                .find(|b| b.id == block_id)
                .ok_or(Error::NotFound(block_id.clone()))?;
            let (update, merged) = notebook::merge(&b.update, &incoming)?;
            if b.update != update {
                b.text = merged;
                b.styles = notebook::styles(&update)?;
                b.update = update.clone();
                n.revisions.push(NotebookRevision {
                    id: new_id(),
                    block_id,
                    reader: state.local_reader.clone(),
                    at,
                    update,
                });
            }
            Ok(json!({"text":b.text,"update":b.update,"styles":b.styles}))
        }
        Command::SetNotebookInk {
            notebook_id,
            block_id,
            version_id,
            parents,
            canvas_width,
            canvas_height,
            strokes,
            native_drawing,
            at,
            capture_context,
            note_creation,
            canvas_space,
        } => notebook_ink::save(
            state,
            &notebook_id,
            &block_id,
            version_id,
            parents,
            canvas_width,
            canvas_height,
            strokes,
            native_drawing,
            at,
            capture_context,
            note_creation,
            canvas_space,
        ),
        Command::SetNoteFace {
            notebook_id,
            block_id,
            face,
            at,
        } => {
            notes::initialize(
                state,
                &notebook_id,
                &block_id,
                at,
                None,
                None,
                false,
                Some(face),
            )?;
            Ok(
                json!({"note":state.notebooks[&notebook_id].blocks.iter().find(|b| b.id == block_id).unwrap().note}),
            )
        }
        Command::SaveNotebookTranscription {
            notebook_id,
            block_id,
            transcription_id,
            ink_version_id,
            text,
            method,
            at,
        } => notebook_ink::transcribe(
            state,
            &notebook_id,
            &block_id,
            transcription_id,
            ink_version_id,
            text,
            method,
            at,
        ),
        Command::AddCharacter {
            notebook_id,
            block_id,
            name,
            at,
            capture_context,
            note_creation,
        } => {
            if capture_context.is_some() || note_creation.is_some() {
                notes::initialize(
                    state,
                    &notebook_id,
                    &block_id,
                    at,
                    capture_context.as_ref(),
                    note_creation.as_ref(),
                    true,
                    None,
                )?;
            }
            let n = state
                .notebooks
                .get(&notebook_id)
                .ok_or(Error::NotFound(notebook_id.clone()))?;
            if name.trim().is_empty() || !n.blocks.iter().any(|b| b.id == block_id) {
                return Err(Error::Invalid(
                    "Name and valid notebook block required".into(),
                ));
            }
            let entity = LiteraryEntity {
                id: new_id(),
                book_id: n.book_id.clone(),
                kind: LiteraryKind::Character,
                name,
            };
            let position_observations = capture_context
                .as_ref()
                .map(|c| c.position_observations.clone())
                .unwrap_or_else(|| latest_positions(state, &n.pass_id));
            let card = CharacterCard {
                id: new_id(),
                character_id: entity.id.clone(),
                notebook_id,
                block_id,
                versions: vec![CardVersion {
                    id: new_id(),
                    parents: vec![],
                    reader: state.local_reader.clone(),
                    at,
                    pass_id: n.pass_id.clone(),
                    positions: capture_context
                        .as_ref()
                        .map(|c| c.positions.clone())
                        .unwrap_or_else(|| {
                            position_observations
                                .iter()
                                .map(|p| p.location.clone())
                                .collect()
                        }),
                    position_observations,
                    interpretation: CharacterInterpretation::default(),
                }],
            };
            let id = card.id.clone();
            state.entities.insert(entity.id.clone(), entity);
            state.cards.insert(id.clone(), card);
            Ok(json!({"card_id":id}))
        }
        Command::AttachCharacter {
            notebook_id,
            block_id,
            character_id,
            at,
            capture_context,
            note_creation,
        } => {
            if capture_context.is_some() || note_creation.is_some() {
                notes::initialize(
                    state,
                    &notebook_id,
                    &block_id,
                    at,
                    capture_context.as_ref(),
                    note_creation.as_ref(),
                    true,
                    None,
                )?;
            }
            let notebook = state
                .notebooks
                .get(&notebook_id)
                .ok_or_else(|| Error::NotFound(notebook_id.clone()))?;
            let character = state
                .entities
                .get(&character_id)
                .ok_or_else(|| Error::NotFound(character_id.clone()))?;
            if !notebook.blocks.iter().any(|block| block.id == block_id)
                || character.book_id != notebook.book_id
                || character.kind != LiteraryKind::Character
            {
                return Err(Error::Invalid(
                    "Choose a character and block from this book".into(),
                ));
            }
            if let Some(existing) = state.cards.values().find(|card| {
                card.notebook_id == notebook_id
                    && card.block_id == block_id
                    && card.character_id == character_id
            }) {
                return Ok(json!({"card_id":existing.id}));
            }
            let position_observations = capture_context
                .as_ref()
                .map(|c| c.position_observations.clone())
                .unwrap_or_else(|| latest_positions(state, &notebook.pass_id));
            let card = CharacterCard {
                id: new_id(),
                character_id,
                notebook_id,
                block_id,
                versions: vec![CardVersion {
                    id: new_id(),
                    parents: vec![],
                    reader: state.local_reader.clone(),
                    at,
                    pass_id: notebook.pass_id.clone(),
                    positions: capture_context
                        .as_ref()
                        .map(|c| c.positions.clone())
                        .unwrap_or_else(|| {
                            position_observations
                                .iter()
                                .map(|p| p.location.clone())
                                .collect()
                        }),
                    position_observations,
                    interpretation: CharacterInterpretation::default(),
                }],
            };
            let id = card.id.clone();
            state.cards.insert(id.clone(), card);
            Ok(json!({"card_id":id}))
        }
        Command::SaveCard {
            card_id,
            parents,
            interpretation,
            mut positions,
            at,
        } => {
            let pass_id = state
                .cards
                .get(&card_id)
                .and_then(|card| state.notebooks.get(&card.notebook_id))
                .ok_or_else(|| Error::NotFound(card_id.clone()))?
                .pass_id
                .clone();
            let position_observations = latest_positions(state, &pass_id);
            for observation in &position_observations {
                if !positions.contains(&observation.location) {
                    positions.push(observation.location.clone());
                }
            }
            let card = state
                .cards
                .get_mut(&card_id)
                .ok_or(Error::NotFound(card_id))?;
            if parents.is_empty()
                || parents
                    .iter()
                    .any(|p| !card.versions.iter().any(|v| &v.id == p))
            {
                return Err(Error::Invalid(
                    "Snapshot must name existing parent versions".into(),
                ));
            }
            let version = CardVersion {
                id: new_id(),
                parents,
                reader: state.local_reader.clone(),
                at,
                pass_id: state.notebooks[&card.notebook_id].pass_id.clone(),
                positions,
                position_observations,
                interpretation,
            };
            let id = version.id.clone();
            card.versions.push(version);
            Ok(json!({"version_id":id,"conflict":card.heads().len()>1}))
        }
        Command::SaveAnnotation { annotation } => {
            state.annotations.insert(annotation.id.clone(), annotation);
            Ok(json!({}))
        }
        Command::ObservePosition { observation } => {
            state.positions.insert(observation.id.clone(), observation);
            Ok(json!({}))
        }
        Command::SaveList {
            id,
            title,
            book_ids,
            expected_version,
        } => {
            let id = id.unwrap_or_else(new_id);
            let version = if let Some(old) = state.lists.get(&id) {
                if old.owner != state.local_reader || expected_version != Some(old.version) {
                    return Err(Error::Conflict("List changed or not owned".into()));
                }
                old.version
                    .checked_add(1)
                    .ok_or_else(|| Error::Invalid("List version overflow".into()))?
            } else {
                if expected_version.is_some() {
                    return Err(Error::Conflict("Missing list".into()));
                }
                1
            };
            if title.trim().is_empty() {
                return Err(Error::Invalid("List needs a title".into()));
            }
            state.lists.insert(
                id.clone(),
                ReadingList {
                    id: id.clone(),
                    owner: state.local_reader.clone(),
                    title,
                    book_ids,
                    shared: false,
                    version,
                },
            );
            Ok(json!({"list_id":id}))
        }
        Command::CreateGroup { name, private } => {
            if name.trim().is_empty() {
                return Err(Error::Invalid("Group needs a name".into()));
            }
            let id = new_id();
            let owner = state.local_reader.clone();
            state.groups.insert(
                id.clone(),
                Group {
                    id: id.clone(),
                    owner: owner.clone(),
                    name,
                    private,
                    epoch: 0,
                    members: BTreeSet::from([owner]),
                    moderators: BTreeSet::new(),
                },
            );
            Ok(json!({"group_id":id}))
        }
        Command::BlockActor { actor } => {
            state.blocked.insert(actor);
            Ok(json!({}))
        }
        Command::PrepareShare {
            kind,
            book_id,
            body,
            spoiler,
            at,
        } => {
            character_sharing::validate_share_body(&body)?;
            let id = new_id();
            // Deliberately no notebook ID, CRDT update, card history, or neighboring blocks.
            state.drafts.insert(
                id.clone(),
                ShareDraft {
                    id: id.clone(),
                    created_at: at,
                    kind,
                    book_id,
                    body,
                    spoiler,
                },
            );
            Ok(json!({"draft_id":id,"published":false}))
        }
        Command::PrepareCharacterShare {
            card_id,
            version_id,
            at,
        } => {
            let body = character_sharing::selected_version(state, &card_id, &version_id)?;
            let id = new_id();
            state.drafts.insert(
                id.clone(),
                ShareDraft {
                    id: id.clone(),
                    created_at: at,
                    kind: "selected_note".into(),
                    book_id: None,
                    body,
                    spoiler: true,
                },
            );
            Ok(json!({"draft_id":id,"published":false}))
        }
        Command::RemoveDraft { id } => {
            state.drafts.remove(&id);
            Ok(json!({}))
        }
    }
}

/// Keep the source clock and alignment evidence with each automatic history snapshot.
fn latest_positions(state: &State, pass_id: &str) -> Vec<ProgressObservation> {
    let mut latest = std::collections::BTreeMap::new();
    for observation in state.positions.values().filter(|p| p.pass_id == pass_id) {
        let key = match &observation.location {
            Location::Audio {
                provider,
                source_id,
                ..
            }
            | Location::Provider {
                provider,
                source_id,
                ..
            } => format!("{provider}:{source_id}"),
            Location::Text { edition_id, .. } | Location::Page { edition_id, .. } => {
                format!("edition:{edition_id}")
            }
        };
        let current: &mut &ProgressObservation = latest.entry(key).or_insert(observation);
        if (observation.observed_at, &observation.id) > (current.observed_at, &current.id) {
            *current = observation;
        }
    }
    latest.into_values().cloned().collect()
}
