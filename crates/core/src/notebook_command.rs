//! Versioned notebook command extensions share the legacy validation and persistence paths.
use crate::{Command, Error, Result, model::*, notes};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum CardContextCommand {
    SaveCardWithContext {
        card_id: String,
        parents: Vec<String>,
        interpretation: CharacterInterpretation,
        at: u64,
        capture_context: CapturedReadingContext,
    },
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum NotebookOrderCommand {
    AppendOrderedNote {
        notebook_id: String,
        block_id: String,
        visible_block_ids: Vec<String>,
        at: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        face: Option<NoteFace>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        capture_context: Option<CapturedReadingContext>,
    },
    ReviewNotebookOrder {
        notebook_id: String,
        review_id: String,
        evidence: String,
        heads: Vec<String>,
        selected: Vec<String>,
    },
}

/// Leaves existing Rust command constructors and exhaustive matches source-compatible.
#[allow(clippy::large_enum_variant)] // Preserve the existing in-memory command without heap indirection.
#[derive(Debug)]
pub enum NotebookCommand {
    Existing(Command),
    WithCardContext(CardContextCommand),
    WithNotebookOrder(NotebookOrderCommand),
    WithIsbnHistory(crate::isbn_history::IsbnHistoryCommand),
}

impl From<Command> for NotebookCommand {
    fn from(command: Command) -> Self {
        Self::Existing(command)
    }
}

impl From<CardContextCommand> for NotebookCommand {
    fn from(command: CardContextCommand) -> Self {
        Self::WithCardContext(command)
    }
}

impl NotebookCommand {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let value: Value = crate::strict_parse(bytes)?;
        match value.get("op").and_then(Value::as_str) {
            Some("save_card_with_context") => {
                Ok(serde_json::from_value::<CardContextCommand>(value)?.into())
            }
            Some("append_ordered_note" | "review_notebook_order") => {
                Ok(Self::WithNotebookOrder(serde_json::from_value(value)?))
            }
            Some("review_isbn" | "reassert_isbn") => {
                Ok(Self::WithIsbnHistory(serde_json::from_value(value)?))
            }
            _ => Ok(serde_json::from_value::<Command>(value)?.into()),
        }
    }

    pub fn apply(self, state: &mut State) -> Result<Value> {
        match self {
            Self::Existing(command) => crate::apply(state, command),
            Self::WithCardContext(command) => command.apply(state),
            Self::WithNotebookOrder(command) => command.apply(state),
            Self::WithIsbnHistory(command) => command.apply(state),
        }
    }
}

impl CardContextCommand {
    /// Store the exact caller-captured history. Current live positions are never recaptured here.
    pub fn apply(self, state: &mut State) -> Result<Value> {
        let Self::SaveCardWithContext {
            card_id,
            parents,
            interpretation,
            at,
            capture_context,
        } = self;
        crate::apply_atomic(state, |next| {
            save_card(
                next,
                card_id,
                parents,
                interpretation,
                at,
                CardReadingContext::Captured(capture_context),
            )
        })
    }
}

impl NotebookOrderCommand {
    pub fn apply(self, state: &mut State) -> Result<Value> {
        use crate::notebook_order::{invalid_order, materialize, snapshot};
        let request = crate::digest(&crate::canonical(&self)?);
        crate::apply_atomic(state, |state| match self {
            Self::AppendOrderedNote {
                notebook_id,
                block_id,
                visible_block_ids,
                at,
                face,
                capture_context,
            } => {
                let notebook = state
                    .notebooks
                    .get(&notebook_id)
                    .ok_or_else(|| Error::NotFound(notebook_id.clone()))?;
                let order = snapshot(notebook).map_err(invalid_order)?;
                if order
                    .replayed_request(&block_id, &request)
                    .map_err(invalid_order)?
                {
                    return Ok(json!({"block_id":block_id,"replayed":true}));
                }
                let intent = order
                    .prepare_visible_append(block_id.clone(), &visible_block_ids)
                    .map_err(invalid_order)?;
                let order = order
                    .record_append(intent, Some(request))
                    .map_err(invalid_order)?;
                crate::append_block(
                    state,
                    &notebook_id,
                    &block_id,
                    at,
                    capture_context,
                    face,
                    Some(order),
                )?;
                Ok(json!({"block_id":block_id}))
            }
            Self::ReviewNotebookOrder {
                notebook_id,
                review_id,
                evidence,
                heads,
                selected,
            } => {
                let notebook = state
                    .notebooks
                    .get_mut(&notebook_id)
                    .ok_or_else(|| Error::NotFound(notebook_id.clone()))?;
                let previous = notebook.order.as_ref().ok_or_else(|| {
                    Error::Conflict("There is no notebook order to review.".into())
                })?;
                let unique: BTreeSet<_> = heads.iter().cloned().collect();
                if unique.len() != heads.len() {
                    return Err(Error::Invalid("Duplicate review heads".into()));
                }
                let order = previous
                    .review_order(review_id.clone(), evidence, unique, selected)
                    .map_err(invalid_order)?;
                if &order == previous {
                    return Ok(json!({"review_id":review_id,"replayed":true}));
                }
                materialize(notebook, order, true).map_err(invalid_order)?;
                Ok(json!({"review_id":review_id}))
            }
        })
    }
}

pub(crate) enum CardReadingContext {
    Current(Vec<Location>),
    Captured(CapturedReadingContext),
}

pub(crate) fn save_card(
    state: &mut State,
    card_id: String,
    parents: Vec<String>,
    interpretation: CharacterInterpretation,
    at: u64,
    reading: CardReadingContext,
) -> Result<Value> {
    let notebook = state
        .cards
        .get(&card_id)
        .and_then(|card| state.notebooks.get(&card.notebook_id))
        .ok_or_else(|| Error::NotFound(card_id.clone()))?;
    let pass_id = notebook.pass_id.clone();
    let (positions, position_observations) = match reading {
        CardReadingContext::Current(mut positions) => {
            let observations = crate::latest_positions(state, &pass_id);
            for observation in &observations {
                if !positions.contains(&observation.location) {
                    positions.push(observation.location.clone());
                }
            }
            (positions, observations)
        }
        CardReadingContext::Captured(context) => {
            notes::validate_context(state, notebook, &context)?;
            (context.positions, context.position_observations)
        }
    };
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
    let id = crate::new_id();
    card.versions.push(CardVersion {
        id: id.clone(),
        parents,
        reader: state.local_reader.clone(),
        at,
        pass_id,
        positions,
        position_observations,
        interpretation,
    });
    Ok(json!({"version_id":id,"conflict":card.heads().len()>1}))
}
