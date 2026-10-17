// SPDX-License-Identifier: MIT OR Apache-2.0
//! Data-only provider composition. Validation grants no signing or publication authority.
use crate::{
    Error, Result,
    audio::AudioObservation,
    castalia_connections::{ConnectionStatus, ProviderConnection, valid_hex, valid_reference},
    identity::DirectoryReference,
    imports::ImportReport,
    model::{SourceRef, State},
    social::{Recommendation, RecommendationInput},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_CONTENT_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_GENERATION: u64 = 9_007_199_254_740_991;
fn invalid(message: &str) -> Error {
    Error::Invalid(message.into())
}
fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> std::result::Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}
fn text(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCapability {
    Catalog,
    Content,
    Positions,
    Identity,
    Recommendations,
    SocialTransport,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["id", "source_namespace", "capabilities", "available", "reason", "connection"]))]
pub struct RuntimeProviderDescriptor {
    pub id: String,
    pub source_namespace: String,
    pub capabilities: Vec<RuntimeCapability>,
    pub available: bool,
    #[serde(deserialize_with = "required_option")]
    pub reason: Option<String>,
    #[serde(deserialize_with = "required_option")]
    pub connection: Option<ProviderConnection>,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProviderRegistry {
    #[schemars(extend("const" = "booknook.provider-registry.v1"))]
    pub schema: String,
    pub providers: Vec<RuntimeProviderDescriptor>,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProviderOperation {
    Catalog {
        source_id: String,
    },
    Content {
        source_id: String,
        media_type: String,
    },
    Positions {
        source_ids: Vec<String>,
    },
    SourcePositions {
        source_ids: Vec<String>,
    },
    Identity {
        entity_iri: String,
    },
    Recommendations {
        input: RecommendationInput,
    },
    SocialPublish {
        event: serde_json::Value,
    },
    #[schemars(extend("required" = ["kind", "authors", "recipient", "since", "before", "limit"]))]
    SocialFetch {
        authors: Vec<String>,
        #[serde(deserialize_with = "required_option")]
        recipient: Option<String>,
        since: u64,
        #[serde(deserialize_with = "required_option")]
        before: Option<u64>,
        limit: u16,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["schema", "request_id", "generation", "provider_id", "descriptor_digest", "connection_id", "operation"]))]
pub struct ProviderRequest {
    #[schemars(extend("const" = "booknook.provider-request.v1"))]
    pub schema: String,
    pub request_id: String,
    pub generation: u64,
    pub provider_id: String,
    pub descriptor_digest: String,
    #[serde(deserialize_with = "required_option")]
    pub connection_id: Option<String>,
    pub operation: ProviderOperation,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["source_id", "title", "authors", "subjects", "edition_identifiers", "language"]))]
pub struct SourceCatalogEntry {
    pub source_id: String,
    pub title: String,
    pub authors: Vec<String>,
    pub subjects: Vec<String>,
    pub edition_identifiers: Vec<SourceRef>,
    #[serde(deserialize_with = "required_option")]
    pub language: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "format", rename_all = "snake_case", deny_unknown_fields)]
pub enum CatalogInput {
    Kindle { json: String },
    Goodreads { csv: String },
    Normalized { entries: Vec<SourceCatalogEntry> },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourcePosition {
    pub source_id: String,
    pub value: String,
    pub observed_at: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProviderResult {
    Catalog {
        catalog: CatalogInput,
    },
    Content {
        source_id: String,
        media_type: String,
        sha256: String,
    },
    Positions {
        observations: Vec<AudioObservation>,
    },
    SourcePositions {
        observations: Vec<SourcePosition>,
    },
    Identity {
        reference: DirectoryReference,
    },
    Recommendations {
        recommendations: Vec<Recommendation>,
    },
    SocialPublish {
        event_id: String,
        relays: Vec<String>,
    },
    SocialFetch {
        events: Vec<serde_json::Value>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["schema", "request_id", "generation", "provider_id", "connection_id", "result"]))]
pub struct ProviderResponse {
    #[schemars(extend("const" = "booknook.provider-response.v1"))]
    pub schema: String,
    pub request_id: String,
    pub generation: u64,
    pub provider_id: String,
    #[serde(deserialize_with = "required_option")]
    pub connection_id: Option<String>,
    pub result: ProviderResult,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProviderExecution {
    #[schemars(extend("const" = "booknook.provider-execution.v1"))]
    pub schema: String,
    pub request: ProviderRequest,
    pub response: ProviderResponse,
}

pub fn parse_registry(raw: &[u8]) -> Result<ProviderRegistry> {
    if raw.len() > 1024 * 1024 {
        return Err(invalid("Provider registry exceeds 1 MiB"));
    }
    let registry: ProviderRegistry = crate::strict_parse(raw)?;
    registry.validate()?;
    Ok(registry)
}
impl ProviderRegistry {
    pub fn validate(&self) -> Result<()> {
        if self.schema != "booknook.provider-registry.v1"
            || self.providers.len() > 64
            || crate::canonical(self)?.len() > 1024 * 1024
        {
            return Err(invalid("Invalid provider registry schema or size"));
        }
        let mut ids = BTreeSet::new();
        for p in &self.providers {
            if !valid_reference(&p.id)
                || !ids.insert(&p.id)
                || !valid_reference(&p.source_namespace)
                || p.source_namespace.bytes().any(|b| b.is_ascii_uppercase())
                || p.capabilities.len() > 6
                || p.capabilities
                    .iter()
                    .enumerate()
                    .any(|(i, c)| p.capabilities[..i].contains(c))
                || p.reason.as_ref().is_some_and(|s| !text(s, 2000))
            {
                return Err(invalid("Invalid or duplicate provider descriptor"));
            }
            if let Some(c) = &p.connection {
                c.validate()?;
            }
        }
        Ok(())
    }
    pub fn descriptor(&self, id: &str) -> Result<&RuntimeProviderDescriptor> {
        self.providers
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| Error::Unavailable("Provider is not registered".into()))
    }
}
impl ProviderOperation {
    pub fn capability(&self) -> RuntimeCapability {
        match self {
            Self::Catalog { .. } => RuntimeCapability::Catalog,
            Self::Content { .. } => RuntimeCapability::Content,
            Self::Positions { .. } | Self::SourcePositions { .. } => RuntimeCapability::Positions,
            Self::Identity { .. } => RuntimeCapability::Identity,
            Self::Recommendations { .. } => RuntimeCapability::Recommendations,
            Self::SocialPublish { .. } | Self::SocialFetch { .. } => {
                RuntimeCapability::SocialTransport
            }
        }
    }
    fn validate(&self) -> Result<()> {
        let valid = match self {
            Self::Catalog { source_id } => text(source_id, 4096),
            Self::Content {
                source_id,
                media_type,
            } => {
                text(source_id, 4096)
                    && ["application/epub+zip", "application/pdf"].contains(&media_type.as_str())
            }
            Self::Positions { source_ids } | Self::SourcePositions { source_ids } => {
                !source_ids.is_empty() && source_ids.len() <= 1000 && unique_text(source_ids, 4096)
            }
            Self::Identity { entity_iri } => DirectoryReference {
                entity_iri: entity_iri.clone(),
                kind: crate::identity::DirectoryKind::Entity,
                display_name: None,
            }
            .validate()
            .is_ok(),
            Self::Recommendations { input } => valid_recommendation_input(input),
            Self::SocialPublish { event } => valid_event(event),
            Self::SocialFetch {
                authors,
                recipient,
                since,
                before,
                limit,
            } => {
                authors.len() <= 1000
                    && authors.iter().all(|s| valid_hex(s))
                    && authors.iter().collect::<BTreeSet<_>>().len() == authors.len()
                    && recipient.as_ref().is_none_or(|s| valid_hex(s))
                    && *since <= MAX_GENERATION
                    && before.is_none_or(|b| b >= *since && b <= MAX_GENERATION)
                    && (1..=500).contains(limit)
            }
        };
        if !valid {
            return Err(invalid("Invalid provider operation or input bounds"));
        }
        Ok(())
    }
}
fn unique_text(values: &[String], limit: usize) -> bool {
    values.iter().all(|s| text(s, limit))
        && values.iter().collect::<BTreeSet<_>>().len() == values.len()
}
fn valid_recommendation_input(input: &RecommendationInput) -> bool {
    input.candidates.len() <= 10000
        && input.candidates.iter().all(|c| {
            text(&c.book_id, 2000)
                && text(&c.title, 4000)
                && c.subjects.len() <= 100
                && c.subjects.iter().all(|s| text(s, 1000))
        })
        && input
            .candidates
            .iter()
            .map(|c| &c.book_id)
            .collect::<BTreeSet<_>>()
            .len()
            == input.candidates.len()
        && input.preferred_subjects.len() <= 1000
        && input.preferred_subjects.iter().all(|s| text(s, 1000))
        && input.shared_ratings.len() <= 10000
        && input
            .shared_ratings
            .iter()
            .all(|r| text(&r.actor, 2000) && text(&r.book_id, 2000) && (1..=5).contains(&r.stars))
        && input.friends.len() <= 10000
        && input.excluded.len() <= 10000
        && input
            .friends
            .iter()
            .chain(input.excluded.iter())
            .all(|s| text(s, 2000))
}
fn valid_event(event: &serde_json::Value) -> bool {
    let Some(o) = event.as_object() else {
        return false;
    };
    o.len() == 7
        && [
            "id",
            "pubkey",
            "created_at",
            "kind",
            "tags",
            "content",
            "sig",
        ]
        .iter()
        .all(|k| o.contains_key(*k))
        && o["id"].as_str().is_some_and(valid_hex)
        && o["pubkey"].as_str().is_some_and(valid_hex)
        && o["sig"].as_str().is_some_and(|s| {
            s.len() == 128
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
        && o["created_at"]
            .as_u64()
            .is_some_and(|n| n <= MAX_GENERATION)
        && o["kind"].as_u64().is_some_and(|n| n <= 65535)
        && o["content"].is_string()
        && o["tags"].as_array().is_some_and(|a| {
            a.len() <= 2000
                && a.iter().all(|v| {
                    v.as_array().is_some_and(|a| {
                        a.len() <= 100
                            && a.iter()
                                .all(|v| v.as_str().is_some_and(|s| s.len() <= 100000))
                    })
                })
        })
        && serde_json::to_vec(event).is_ok_and(|v| v.len() <= 100000)
}
fn descriptor_for<'a>(
    registry: &'a ProviderRegistry,
    generation: u64,
    request: &ProviderRequest,
) -> Result<&'a RuntimeProviderDescriptor> {
    registry.validate()?;
    if request.schema != "booknook.provider-request.v1"
        || request.generation != generation
        || generation > MAX_GENERATION
        || !valid_reference(&request.request_id)
    {
        return Err(Error::Conflict(
            "Provider request generation or identity is stale".into(),
        ));
    }
    request.operation.validate()?;
    let descriptor = registry.descriptor(&request.provider_id)?;
    if request.descriptor_digest != crate::digest(&crate::canonical(descriptor)?)
        || request.connection_id
            != descriptor
                .connection
                .as_ref()
                .map(|c| c.connection_id.clone())
    {
        return Err(Error::Conflict(
            "Provider descriptor changed after selection".into(),
        ));
    }
    if !descriptor.available
        || descriptor
            .connection
            .as_ref()
            .is_some_and(|c| c.status != ConnectionStatus::Available)
    {
        return Err(Error::Unavailable(
            "Selected provider connection is unavailable".into(),
        ));
    }
    if !descriptor
        .capabilities
        .contains(&request.operation.capability())
    {
        return Err(Error::Unavailable(
            "Selected provider lacks the requested capability".into(),
        ));
    }
    if let Some(connection) = &descriptor.connection {
        let (provider, capabilities): (&str, &[&str]) =
            match (&request.operation, descriptor.source_namespace.as_str()) {
                (ProviderOperation::Catalog { .. }, "kindle") => (
                    "com.amazon.kindle",
                    &["kindle.catalog", "kindle.catalog.import"],
                ),
                (ProviderOperation::Content { .. }, "kindle") => {
                    ("com.amazon.kindle", &["kindle.content"])
                }
                (ProviderOperation::SourcePositions { .. }, "kindle") => {
                    ("com.amazon.kindle", &["kindle.positions"])
                }
                (ProviderOperation::Positions { .. }, "audible") => {
                    ("com.audible", &["audible.positions"])
                }
                (ProviderOperation::Catalog { .. }, namespace) => (namespace, &["catalog"]),
                (ProviderOperation::Content { .. }, namespace) => (namespace, &["content"]),
                (
                    ProviderOperation::Positions { .. } | ProviderOperation::SourcePositions { .. },
                    namespace,
                ) => (namespace, &["positions"]),
                (ProviderOperation::Identity { .. }, namespace) => (namespace, &["identity"]),
                (ProviderOperation::Recommendations { .. }, namespace) => {
                    (namespace, &["recommendations"])
                }
                (_, namespace) => (namespace, &["social_transport"]),
            };
        if connection.provider_id != provider
            || !capabilities
                .iter()
                .any(|cap| connection.capabilities.iter().any(|c| c == cap))
        {
            return Err(Error::Denied(
                "Provider connection does not advertise this source and capability".into(),
            ));
        }
    }
    Ok(descriptor)
}
pub fn prepare_operation(
    registry: &ProviderRegistry,
    generation: u64,
    provider_id: &str,
    operation_json: &[u8],
    request_id: &str,
) -> Result<ProviderRequest> {
    if operation_json.len() > 16 * 1024 * 1024 {
        return Err(invalid("Provider operation exceeds 16 MiB"));
    }
    let descriptor = registry.descriptor(provider_id)?;
    let request = ProviderRequest {
        schema: "booknook.provider-request.v1".into(),
        request_id: request_id.into(),
        generation,
        provider_id: provider_id.into(),
        descriptor_digest: crate::digest(&crate::canonical(descriptor)?),
        connection_id: descriptor
            .connection
            .as_ref()
            .map(|c| c.connection_id.clone()),
        operation: crate::strict_parse(operation_json)?,
    };
    descriptor_for(registry, generation, &request)?;
    Ok(request)
}
pub fn validate_reply(
    registry: &ProviderRegistry,
    generation: u64,
    request: &ProviderRequest,
    response_json: &[u8],
    content_bytes: &[u8],
    now: u64,
) -> Result<ProviderExecution> {
    let descriptor = descriptor_for(registry, generation, request)?;
    let response: ProviderResponse = crate::strict_parse(response_json)?;
    if response.schema != "booknook.provider-response.v1"
        || response.request_id != request.request_id
        || response.generation != request.generation
        || response.provider_id != request.provider_id
        || response.connection_id != request.connection_id
    {
        return Err(invalid(
            "Provider response correlation does not match request",
        ));
    }
    if !matches!(response.result, ProviderResult::Content { .. }) && !content_bytes.is_empty() {
        return Err(invalid("Unexpected provider content bytes"));
    }
    match (&request.operation, &response.result) {
        (ProviderOperation::Catalog { .. }, ProviderResult::Catalog { catalog }) => {
            validate_catalog(descriptor, catalog)?
        }
        (
            ProviderOperation::Content {
                source_id,
                media_type,
            },
            ProviderResult::Content {
                source_id: s,
                media_type: m,
                sha256,
            },
        ) => {
            if source_id != s
                || media_type != m
                || content_bytes.is_empty()
                || content_bytes.len() > MAX_CONTENT_BYTES
                || !valid_hex(sha256)
                || crate::digest(content_bytes) != *sha256
            {
                return Err(invalid(
                    "Provider content identity, media, bounds or digest mismatch",
                ));
            }
        }
        (
            ProviderOperation::Positions { source_ids },
            ProviderResult::Positions { observations },
        ) => {
            if descriptor.source_namespace != "audible" || observations.len() > 1000 {
                return Err(invalid("Invalid audio position provider or count"));
            }
            for o in observations {
                o.validate(source_ids, now)?;
            }
        }
        (
            ProviderOperation::SourcePositions { source_ids },
            ProviderResult::SourcePositions { observations },
        ) => {
            if observations.len() > 1000
                || observations.iter().any(|o| {
                    !source_ids.contains(&o.source_id)
                        || !text(&o.value, 16384)
                        || o.observed_at == 0
                        || o.observed_at > now.saturating_add(300)
                })
            {
                return Err(invalid(
                    "Invalid source position identity, location or time",
                ));
            }
        }
        (ProviderOperation::Identity { entity_iri }, ProviderResult::Identity { reference }) => {
            reference.validate()?;
            if *entity_iri != reference.entity_iri {
                return Err(invalid("Provider returned a different directory identity"));
            }
        }
        (
            ProviderOperation::Recommendations { input },
            ProviderResult::Recommendations { recommendations },
        ) => {
            let mut seen = BTreeSet::new();
            if recommendations.len() > input.candidates.len()
                || recommendations.iter().any(|r| {
                    !seen.insert(&r.book_id)
                        || input.excluded.contains(&r.book_id)
                        || !input.candidates.iter().any(|c| c.book_id == r.book_id)
                        || r.reasons.is_empty()
                        || r.reasons.len() > 100
                        || r.reasons.iter().any(|s| !text(s, 2000))
                })
            {
                return Err(invalid("Invalid provider recommendation or explanation"));
            }
        }
        (
            ProviderOperation::SocialPublish { event },
            ProviderResult::SocialPublish { event_id, relays },
        ) => {
            if event["id"].as_str() != Some(event_id)
                || relays.is_empty()
                || relays.len() > 32
                || !unique_text(relays, 2048)
                || relays.iter().any(|r| {
                    url::Url::parse(r).is_err()
                        || !r.starts_with("wss://") && !r.starts_with("ws://")
                })
            {
                return Err(invalid(
                    "Provider did not acknowledge the exact signed event",
                ));
            }
        }
        (ProviderOperation::SocialFetch { limit, .. }, ProviderResult::SocialFetch { events }) => {
            if events.len() > usize::from(*limit) || events.iter().any(|e| !valid_event(e)) {
                return Err(invalid("Invalid or oversized social provider batch"));
            }
        }
        _ => {
            return Err(invalid(
                "Provider result does not match the requested operation",
            ));
        }
    }
    Ok(ProviderExecution {
        schema: "booknook.provider-execution.v1".into(),
        request: request.clone(),
        response,
    })
}
pub fn validate_execution(
    registry: &ProviderRegistry,
    generation: u64,
    execution_json: &[u8],
    content_bytes: &[u8],
    now: u64,
) -> Result<ProviderExecution> {
    let execution: ProviderExecution = crate::strict_parse(execution_json)?;
    if execution.schema != "booknook.provider-execution.v1" {
        return Err(invalid("Unsupported provider execution schema"));
    }
    validate_reply(
        registry,
        generation,
        &execution.request,
        &crate::canonical(&execution.response)?,
        content_bytes,
        now,
    )
}
fn validate_catalog(descriptor: &RuntimeProviderDescriptor, catalog: &CatalogInput) -> Result<()> {
    match catalog {
        CatalogInput::Kindle { json } => {
            if descriptor.source_namespace != "kindle" || json.len() > 32 * 1024 * 1024 {
                return Err(invalid("Kindle provider namespace or size mismatch"));
            }
            let c: crate::imports::KindleCatalog = crate::strict_parse(json.as_bytes())?;
            if c.schema != "booknook.kindle-catalog.v1" || c.items.len() > 10000 {
                return Err(invalid("Invalid Kindle catalog schema or count"));
            }
            if c.items
                .iter()
                .any(|i| i.position.is_some() || i.observed_at.is_some())
                && (!descriptor
                    .capabilities
                    .contains(&RuntimeCapability::Positions)
                    || descriptor
                        .connection
                        .as_ref()
                        .is_some_and(|c| !c.capabilities.iter().any(|s| s == "kindle.positions")))
            {
                return Err(Error::Denied(
                    "Kindle progress requires a separately advertised positions capability".into(),
                ));
            }
        }
        CatalogInput::Goodreads { csv } => {
            if descriptor.source_namespace != "goodreads" || csv.len() > 32 * 1024 * 1024 {
                return Err(invalid("Goodreads provider namespace or size mismatch"));
            }
        }
        CatalogInput::Normalized { entries } => {
            if ["kindle", "goodreads"].contains(&descriptor.source_namespace.as_str())
                || entries.len() > 10000
            {
                return Err(invalid("Invalid normalized catalog namespace or count"));
            }
            let mut seen = BTreeSet::new();
            if entries.iter().any(|e| {
                !text(&e.source_id, 2000)
                    || !seen.insert(&e.source_id)
                    || !text(&e.title, 4000)
                    || e.authors.len() > 100
                    || e.authors.iter().any(|s| !text(s, 2000))
                    || e.subjects.len() > 100
                    || e.subjects.iter().any(|s| !text(s, 1000))
                    || e.edition_identifiers.len() > 100
                    || e.edition_identifiers
                        .iter()
                        .any(|s| !text(&s.provider, 200) || !text(&s.id, 2000))
                    || e.language.as_ref().is_some_and(|s| !text(s, 100))
            }) {
                return Err(invalid("Invalid normalized source catalog entry"));
            }
        }
    }
    Ok(())
}
pub fn apply_catalog(
    state: &mut State,
    registry: &ProviderRegistry,
    generation: u64,
    execution_json: &[u8],
    now: u64,
) -> Result<ImportReport> {
    let execution = validate_execution(registry, generation, execution_json, &[], now)?;
    let descriptor = registry.descriptor(&execution.request.provider_id)?;
    let ProviderResult::Catalog { catalog } = execution.response.result else {
        return Err(invalid("Expected catalog provider result"));
    };
    let mut staged = state.clone();
    let connection_id = execution.request.connection_id.as_deref();
    let report = match catalog {
        CatalogInput::Kindle { json } => {
            crate::imports::kindle_from_connection(&mut staged, &json, now, connection_id)?
        }
        CatalogInput::Goodreads { csv } => {
            crate::imports::goodreads_from_connection(&mut staged, &csv, now, connection_id)?
        }
        CatalogInput::Normalized { entries } => crate::imports::normalized(
            &mut staged,
            &descriptor.source_namespace,
            &entries,
            now,
            connection_id,
        )?,
    };
    staged.revision = staged
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("Notebook revision exhausted"))?;
    crate::validate(&staged)?;
    *state = staged;
    Ok(report)
}

/// Attach an explicitly selected source observation to a reading pass. Opaque locations are
/// retained unchanged, with unmatched alignment; no ebook percentage or edition is inferred.
pub fn apply_source_position(
    state: &mut State,
    registry: &ProviderRegistry,
    generation: u64,
    execution_json: &[u8],
    observation_json: &[u8],
    pass_id: &str,
    now: u64,
) -> Result<serde_json::Value> {
    let execution = validate_execution(registry, generation, execution_json, &[], now)?;
    let ProviderResult::SourcePositions { observations } = &execution.response.result else {
        return Err(invalid("Expected source position result"));
    };
    let source: SourcePosition = crate::strict_parse(observation_json)?;
    if !observations.contains(&source) {
        return Err(invalid(
            "Selected source position is not in this provider read",
        ));
    }
    let descriptor = registry.descriptor(&execution.request.provider_id)?;
    let id = crate::digest(&crate::canonical(&(
        "booknook.provider-position.v1",
        &descriptor.source_namespace,
        &execution.request.connection_id,
        pass_id,
        &source,
    ))?);
    let observation = crate::model::ProgressObservation {
        id,
        provider_connection_id: execution.request.connection_id,
        pass_id: pass_id.into(),
        location: crate::model::Location::Provider {
            provider: descriptor.source_namespace.clone(),
            source_id: source.source_id,
            value: source.value,
        },
        observed_at: source.observed_at,
        received_at: now,
        alignment: crate::model::Alignment::Unmatched,
        audio_metadata: None,
    };
    crate::apply(state, crate::Command::ObservePosition { observation })
}
