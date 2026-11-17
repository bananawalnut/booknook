//! Versioned notebook command extensions share the legacy validation and persistence paths.
use crate::{Command, Error, Result, model::*, notes};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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

/// Leaves existing Rust command constructors and exhaustive matches source-compatible.
#[allow(clippy::large_enum_variant)] // Preserve the existing in-memory command without heap indirection.
#[derive(Debug)]
pub enum NotebookCommand {
    Existing(Command),
    WithCardContext(CardContextCommand),
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
        if value.get("op").and_then(Value::as_str) == Some("save_card_with_context") {
            Ok(serde_json::from_value::<CardContextCommand>(value)?.into())
        } else {
            Ok(serde_json::from_value::<Command>(value)?.into())
        }
    }

    pub fn apply(self, state: &mut State) -> Result<Value> {
        match self {
            Self::Existing(command) => crate::apply(state, command),
            Self::WithCardContext(command) => command.apply(state),
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
