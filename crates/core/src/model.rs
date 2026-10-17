//! Portable, provider-independent Book Nook records. Times are Unix seconds.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA: &str = "booknook.archive.v1";
pub const ONTOLOGY: &str = "0.1.0";
/// Ink-bearing archives deliberately reject older, ink-unaware readers.
pub const INK_ARCHIVE_SCHEMA: &str = "booknook.archive.v2";
pub const INK_ONTOLOGY: &str = "0.2.0";
pub const NOTE_ARCHIVE_SCHEMA: &str = "booknook.archive.v3";
pub const NOTE_ONTOLOGY: &str = "0.3.0";
pub const NOTE_SCHEMA: &str = "booknook.note.v1";
pub const NOTEBOOK_INK_SCHEMA: &str = "booknook.notebook-ink.v1";

#[derive(Clone, Debug, Default, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SourceRef {
    pub provider: String,
    pub id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ImportEvidence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_connection_id: Option<String>,
    pub source: SourceRef,
    pub imported_at: u64,
    pub digest: String,
    pub fields: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Book {
    pub id: String,
    pub title: String,
    pub authors: Vec<String>,
    pub subjects: Vec<String>,
    pub edition_ids: Vec<String>,
    pub sources: Vec<SourceRef>,
    pub shelf: String,
    pub rating: Option<u8>,
    pub review: String,
    pub reading_dates: Vec<String>,
    pub edited_fields: BTreeSet<String>,
    pub provenance: Vec<ImportEvidence>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Edition {
    pub id: String,
    pub book_id: String,
    pub identifiers: Vec<SourceRef>,
    pub language: Option<String>,
    pub assets: Vec<ContentAsset>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ContentAsset {
    pub id: String,
    pub media_type: String,
    /// Content-addressed local asset key, never an arbitrary absolute path.
    pub storage_key: String,
    pub sha256: String,
    pub spine: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Location {
    Text {
        edition_id: String,
        resource: String,
        quote: String,
        prefix: String,
        suffix: String,
    },
    Audio {
        provider: String,
        source_id: String,
        milliseconds: u64,
        chapter: Option<String>,
    },
    Provider {
        provider: String,
        source_id: String,
        value: String,
    },
    Page {
        edition_id: String,
        page: u32,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProgressObservation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_connection_id: Option<String>,
    pub id: String,
    pub pass_id: String,
    pub location: Location,
    pub observed_at: u64,
    pub received_at: u64,
    pub alignment: Alignment,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_metadata: Option<crate::audio::AudioMetadata>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum Alignment {
    Unmatched,
    Candidate {
        edition_id: String,
        reason: String,
    },
    Confirmed {
        edition_id: String,
        evidence: String,
    },
}

impl ProgressObservation {
    pub fn stale(&self, now: u64, maximum_age: u64) -> bool {
        self.observed_at > now || now.saturating_sub(self.observed_at) > maximum_age
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReadingPass {
    pub id: String,
    pub book_id: String,
    pub reader: String,
    pub started_at: u64,
    pub finished_at: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReadingSession {
    pub id: String,
    pub pass_id: String,
    pub started_at: u64,
    pub ended_at: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Block {
    pub id: String,
    pub kind: String,
    pub text: String,
    pub update: String,
    #[serde(default)]
    pub styles: Vec<InlineStyle>,
    /// Absent on legacy typed blocks; never flattened into the text CRDT.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ink: Option<NotebookInk>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<NoteMetadata>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CapturedReadingContext {
    pub pass_id: String,
    pub positions: Vec<Location>,
    pub position_observations: Vec<ProgressObservation>,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NoteCreation {
    pub at: u64,
    pub capture_context: CapturedReadingContext,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NoteFace {
    Text,
    Whiteboard,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NoteOriginSource {
    Recorded,
    HistoryFallback,
    Unknown,
    Pending,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NoteOrigin {
    pub source: NoteOriginSource,
    pub at: Option<u64>,
    pub reader: String,
    pub pass_id: String,
    pub positions: Vec<Location>,
    pub position_observations: Vec<ProgressObservation>,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NoteMetadata {
    pub schema: String,
    pub face: NoteFace,
    pub created: NoteOrigin,
}
/// Absolute logical world coordinates. Moving the viewport never transforms saved strokes.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CanvasSpace {
    Whiteboard { origin_x: f64, origin_y: f64 },
}

/// A logical canvas belonging to its containing stable notebook block.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NotebookInk {
    pub schema: String,
    pub versions: Vec<NotebookInkVersion>,
    pub transcriptions: Vec<NotebookTranscription>,
}
impl NotebookInk {
    /// Every unresolved branch is returned. Consumers must not pick a winner by timestamp.
    pub fn heads(&self) -> Vec<&NotebookInkVersion> {
        let parents: BTreeSet<&str> = self
            .versions
            .iter()
            .flat_map(|v| v.parents.iter().map(String::as_str))
            .collect();
        self.versions
            .iter()
            .filter(|v| !parents.contains(v.id.as_str()))
            .collect()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NotebookInkVersion {
    pub id: String,
    pub parents: Vec<String>,
    pub reader: String,
    pub at: u64,
    pub pass_id: String,
    pub positions: Vec<Location>,
    pub position_observations: Vec<ProgressObservation>,
    pub canvas_width: f64,
    pub canvas_height: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas_space: Option<CanvasSpace>,
    pub strokes: Vec<Stroke>,
    /// Optional base64 native drawing. Portable strokes remain alongside native data.
    pub native_drawing: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NotebookTranscription {
    pub id: String,
    pub ink_version_id: String,
    pub reader: String,
    pub at: u64,
    pub text: String,
    pub method: TranscriptionMethod,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptionMethod {
    Manual,
    OnDevice,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InlineStyle {
    pub start_utf16: u32,
    pub length_utf16: u32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Notebook {
    pub id: String,
    pub book_id: String,
    pub pass_id: String,
    pub blocks: Vec<Block>,
    pub revisions: Vec<NotebookRevision>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NotebookRevision {
    pub id: String,
    pub block_id: String,
    pub reader: String,
    pub at: u64,
    pub update: String,
}

/// Literary entities never confer directory membership or operational permission.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LiteraryEntity {
    pub id: String,
    pub book_id: String,
    pub kind: LiteraryKind,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum LiteraryKind {
    Character,
    Place,
    Theme,
    Event,
    Concept,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Relationship {
    pub target: String,
    pub description: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CharacterInterpretation {
    pub description: String,
    pub traits: Vec<String>,
    pub relationships: Vec<Relationship>,
    pub observations: Vec<String>,
    pub questions: Vec<String>,
    pub passages: Vec<Location>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CharacterCard {
    pub id: String,
    pub character_id: String,
    pub notebook_id: String,
    pub block_id: String,
    pub versions: Vec<CardVersion>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CardVersion {
    pub id: String,
    pub parents: Vec<String>,
    pub reader: String,
    pub at: u64,
    pub pass_id: String,
    pub positions: Vec<Location>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub position_observations: Vec<ProgressObservation>,
    pub interpretation: CharacterInterpretation,
}

impl CharacterCard {
    pub fn heads(&self) -> Vec<&CardVersion> {
        let parents: BTreeSet<_> = self.versions.iter().flat_map(|v| &v.parents).collect();
        self.versions
            .iter()
            .filter(|v| !parents.contains(&v.id))
            .collect()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InkPoint {
    pub x: f64,
    pub y: f64,
    pub pressure: f64,
    pub time: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Stroke {
    pub points: Vec<InkPoint>,
    pub color: String,
    pub width: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Annotation {
    pub id: String,
    pub pass_id: String,
    pub anchor: Location,
    pub original_context: String,
    pub layout_fingerprint: Option<String>,
    pub strokes: Vec<Stroke>,
    pub native_drawing: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ink_geometry: Option<InkGeometry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum InkSpace {
    EpubDocument,
    PdfPage,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InkRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InkGeometry {
    pub space: InkSpace,
    pub resource: String,
    pub page: Option<u32>,
    pub content_fingerprint: String,
    pub width: f64,
    pub height: f64,
    pub origin_x: f64,
    pub origin_y: f64,
    pub text_quote: Option<String>,
    pub text_prefix: Option<String>,
    pub text_suffix: Option<String>,
    pub text_rects: Vec<InkRect>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReadingList {
    pub id: String,
    pub owner: String,
    pub title: String,
    pub book_ids: Vec<String>,
    pub shared: bool,
    pub version: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub id: String,
    pub owner: String,
    pub name: String,
    pub private: bool,
    pub epoch: u64,
    pub members: BTreeSet<String>,
    pub moderators: BTreeSet<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Activity {
    pub id: String,
    pub actor: String,
    pub published_at: u64,
    pub kind: String,
    pub book_id: Option<String>,
    pub body: String,
    pub spoiler: bool,
    pub group_id: Option<String>,
    pub group_epoch: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ShareDraft {
    pub id: String,
    pub created_at: u64,
    pub kind: String,
    pub book_id: Option<String>,
    pub body: String,
    pub spoiler: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub schema: String,
    pub ontology_version: String,
    pub revision: u64,
    /// Local attribution is not a Castalia identity and cannot authorize publication.
    pub local_reader: String,
    pub books: BTreeMap<String, Book>,
    pub editions: BTreeMap<String, Edition>,
    pub passes: BTreeMap<String, ReadingPass>,
    pub sessions: BTreeMap<String, ReadingSession>,
    pub notebooks: BTreeMap<String, Notebook>,
    pub entities: BTreeMap<String, LiteraryEntity>,
    pub cards: BTreeMap<String, CharacterCard>,
    pub annotations: BTreeMap<String, Annotation>,
    pub positions: BTreeMap<String, ProgressObservation>,
    pub lists: BTreeMap<String, ReadingList>,
    pub groups: BTreeMap<String, Group>,
    pub friendship_requests: BTreeSet<(String, String)>,
    pub blocked: BTreeSet<String>,
    pub activity: BTreeMap<String, Activity>,
    pub drafts: BTreeMap<String, ShareDraft>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            schema: SCHEMA.into(),
            ontology_version: ONTOLOGY.into(),
            revision: 0,
            local_reader: crate::new_id(),
            books: BTreeMap::new(),
            editions: BTreeMap::new(),
            passes: BTreeMap::new(),
            sessions: BTreeMap::new(),
            notebooks: BTreeMap::new(),
            entities: BTreeMap::new(),
            cards: BTreeMap::new(),
            annotations: BTreeMap::new(),
            positions: BTreeMap::new(),
            lists: BTreeMap::new(),
            groups: BTreeMap::new(),
            friendship_requests: BTreeSet::new(),
            blocked: BTreeSet::new(),
            activity: BTreeMap::new(),
            drafts: BTreeMap::new(),
        }
    }
}
