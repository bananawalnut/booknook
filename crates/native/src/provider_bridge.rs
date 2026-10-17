// SPDX-License-Identifier: MIT OR Apache-2.0
//! Async source reads and synchronous, checked commits. Providers never receive signing keys.
use crate::{Desk, NativeError, Result, now};
use booknook_core::{
    Error, canonical, provider_runtime as p,
    providers::{CatalogRecommendations, RecommendationProvider},
    strict_parse,
};
use std::{
    collections::{BTreeMap, VecDeque},
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
};

pub const LOCAL_RECOMMENDATIONS: &str = "booknook.local-recommendations";
pub const NOSTR: &str = "booknook.nostr";
#[derive(Clone, Debug, uniffi::Record)]
pub struct ProviderReply {
    pub response_json: String,
    pub content_bytes: Vec<u8>,
}
#[uniffi::export(callback_interface)]
#[async_trait::async_trait]
pub trait ProviderHost: Send + Sync {
    fn descriptors_json(&self) -> Result<String>;
    async fn execute(&self, request_json: String) -> Result<ProviderReply>;
}
struct PendingRead {
    execution_digest: String,
    settings_digest: String,
}
pub(crate) struct ProviderRuntime {
    host: Option<Arc<dyn ProviderHost>>,
    pub(crate) registry: p::ProviderRegistry,
    pub(crate) generation: u64,
    pending: BTreeMap<String, PendingRead>,
    order: VecDeque<String>,
}
fn builtin(id: &str, capability: p::RuntimeCapability) -> p::RuntimeProviderDescriptor {
    p::RuntimeProviderDescriptor {
        id: id.into(),
        source_namespace: "booknook".into(),
        capabilities: vec![capability],
        available: true,
        reason: None,
        connection: None,
    }
}
impl Default for ProviderRuntime {
    fn default() -> Self {
        Self {
            host: None,
            registry: p::ProviderRegistry {
                schema: "booknook.provider-registry.v1".into(),
                providers: vec![
                    builtin(LOCAL_RECOMMENDATIONS, p::RuntimeCapability::Recommendations),
                    builtin(NOSTR, p::RuntimeCapability::SocialTransport),
                ],
            },
            generation: 0,
            pending: BTreeMap::new(),
            order: VecDeque::new(),
        }
    }
}
impl ProviderRuntime {
    fn with_host(host: Box<dyn ProviderHost>, generation: u64) -> Result<Self> {
        if generation == 0 || generation > p::MAX_GENERATION {
            return Err(Error::Invalid(
                "Provider generation must be a positive safe integer".into(),
            )
            .into());
        }
        let mut registry = p::parse_registry(host.descriptors_json()?.as_bytes())?;
        if registry
            .providers
            .iter()
            .any(|d| [LOCAL_RECOMMENDATIONS, NOSTR].contains(&d.id.as_str()))
        {
            return Err(Error::Invalid("Reserved built-in provider ID".into()).into());
        }
        registry
            .providers
            .extend(Self::default().registry.providers);
        registry.validate()?;
        Ok(Self {
            host: Some(Arc::from(host)),
            registry,
            generation,
            pending: BTreeMap::new(),
            order: VecDeque::new(),
        })
    }
    fn retain(&mut self, execution: &p::ProviderExecution, settings_digest: String) -> Result<()> {
        let id = execution.request.request_id.clone();
        self.pending.insert(
            id.clone(),
            PendingRead {
                execution_digest: booknook_core::digest(&canonical(execution)?),
                settings_digest,
            },
        );
        self.order.push_back(id);
        while self.order.len() > 128 {
            if let Some(id) = self.order.pop_front() {
                self.pending.remove(&id);
            }
        }
        Ok(())
    }
    fn check_read(&self, execution: &p::ProviderExecution, settings_digest: &str) -> Result<()> {
        let read = self
            .pending
            .get(&execution.request.request_id)
            .ok_or_else(|| {
                Error::Conflict("Provider read expired; fetch the source again".into())
            })?;
        if read.execution_digest != booknook_core::digest(&canonical(execution)?)
            || read.settings_digest != settings_digest
        {
            return Err(Error::Conflict(
                "Provider read or connection settings changed before commit".into(),
            )
            .into());
        }
        Ok(())
    }
}
impl Desk {
    pub(crate) fn provider_lock(&self) -> Result<MutexGuard<'_, ProviderRuntime>> {
        self.providers.lock().map_err(|_| NativeError::Failure {
            message: "Provider runtime lock poisoned".into(),
        })
    }
    fn provider_settings_digest(&self) -> Result<String> {
        Ok(booknook_core::digest(
            self.connection_settings()?.as_bytes(),
        ))
    }
    pub(crate) async fn fetch_provider_request(
        &self,
        provider_id: &str,
        operation: &p::ProviderOperation,
    ) -> Result<ProviderReply> {
        self.provider_fetch(
            provider_id.into(),
            String::from_utf8(canonical(operation)?).unwrap(),
        )
        .await
    }
}
#[uniffi::export]
impl Desk {
    #[uniffi::constructor]
    pub fn with_providers(
        directory: String,
        host: Box<dyn ProviderHost>,
        generation: u64,
    ) -> Result<Self> {
        let providers = ProviderRuntime::with_host(host, generation)?;
        Ok(Self {
            store: Mutex::new(crate::store::Store::open(Path::new(&directory))?),
            custody: Mutex::new(crate::custody::Custody::default()),
            providers: Mutex::new(providers),
        })
    }
    pub fn replace_provider_host(
        &self,
        host: Box<dyn ProviderHost>,
        generation: u64,
    ) -> Result<()> {
        let replacement = ProviderRuntime::with_host(host, generation)?;
        let mut current = self.provider_lock()?;
        if generation <= current.generation {
            return Err(
                Error::Conflict("Replacement provider generation must advance".into()).into(),
            );
        }
        *current = replacement;
        Ok(())
    }
    pub fn provider_descriptors(&self) -> Result<String> {
        Ok(serde_json::to_string(&self.provider_lock()?.registry)?)
    }
    /// No state changes. The caller checks cancellation before a separate synchronous commit.
    pub async fn provider_fetch(
        &self,
        provider_id: String,
        operation_json: String,
    ) -> Result<ProviderReply> {
        let (request, host, settings_digest) = {
            let runtime = self.provider_lock()?;
            let request = p::prepare_operation(
                &runtime.registry,
                runtime.generation,
                &provider_id,
                operation_json.as_bytes(),
                &booknook_core::new_id(),
            )?;
            (
                request,
                runtime.host.clone(),
                self.provider_settings_digest()?,
            )
        };
        let reply = if provider_id == LOCAL_RECOMMENDATIONS {
            let p::ProviderOperation::Recommendations { input } = &request.operation else {
                return Err(Error::Invalid("Expected recommendation operation".into()).into());
            };
            let response = p::ProviderResponse {
                schema: "booknook.provider-response.v1".into(),
                request_id: request.request_id.clone(),
                generation: request.generation,
                provider_id: request.provider_id.clone(),
                connection_id: request.connection_id.clone(),
                result: p::ProviderResult::Recommendations {
                    recommendations: CatalogRecommendations.recommend(input)?,
                },
            };
            ProviderReply {
                response_json: serde_json::to_string(&response)?,
                content_bytes: vec![],
            }
        } else if provider_id == NOSTR {
            self.builtin_nostr_request(&request).await?
        } else {
            host.ok_or_else(|| Error::Unavailable("No provider host installed".into()))?
                .execute(serde_json::to_string(&request)?)
                .await?
        };
        let mut runtime = self.provider_lock()?;
        let execution = p::validate_reply(
            &runtime.registry,
            runtime.generation,
            &request,
            reply.response_json.as_bytes(),
            &reply.content_bytes,
            now()?,
        )?;
        if settings_digest != self.provider_settings_digest()? {
            return Err(Error::Conflict(
                "Connection settings changed while reading provider".into(),
            )
            .into());
        }
        runtime.retain(&execution, settings_digest)?;
        Ok(ProviderReply {
            response_json: serde_json::to_string(&execution)?,
            content_bytes: reply.content_bytes,
        })
    }
    pub fn provider_commit_catalog(&self, execution_json: String) -> Result<String> {
        let runtime = self.provider_lock()?;
        let execution = p::validate_execution(
            &runtime.registry,
            runtime.generation,
            execution_json.as_bytes(),
            &[],
            now()?,
        )?;
        let mut store = self.lock()?;
        let settings = crate::connection_bridge::load(&store.connection)?;
        runtime.check_read(
            &execution,
            &booknook_core::digest(serde_json::to_string(&settings)?.as_bytes()),
        )?;
        let mut state = store.load()?;
        let revision = state.revision;
        let result = p::apply_catalog(
            &mut state,
            &runtime.registry,
            runtime.generation,
            execution_json.as_bytes(),
            now()?,
        )?;
        store.save_for_provider(
            revision,
            &state,
            &runtime.pending[&execution.request.request_id].settings_digest,
        )?;
        Ok(serde_json::to_string(
            &booknook_core::providers::CommandResponse {
                state,
                result: serde_json::to_value(result)?,
            },
        )?)
    }
    pub fn provider_commit_content(
        &self,
        execution_json: String,
        content_bytes: Vec<u8>,
        book_id: String,
    ) -> Result<String> {
        let runtime = self.provider_lock()?;
        let execution = p::validate_execution(
            &runtime.registry,
            runtime.generation,
            execution_json.as_bytes(),
            &content_bytes,
            now()?,
        )?;
        let p::ProviderResult::Content {
            media_type,
            source_id,
            sha256,
        } = &execution.response.result
        else {
            return Err(Error::Invalid("Expected content result".into()).into());
        };
        let mut store = self.lock()?;
        let settings = crate::connection_bridge::load(&store.connection)?;
        runtime.check_read(
            &execution,
            &booknook_core::digest(serde_json::to_string(&settings)?.as_bytes()),
        )?;
        let mut state = store.load()?;
        let revision = state.revision;
        if !state.books.contains_key(&book_id) {
            return Err(Error::NotFound("Book/edition missing".into()).into());
        }
        let kind = if media_type == "application/pdf" {
            "pdf"
        } else {
            "epub"
        };
        let asset = crate::content::import(&store.root, &content_bytes, kind)?;
        if !state
            .editions
            .values()
            .any(|e| e.book_id == book_id && e.assets.iter().any(|a| a.id == asset.id))
        {
            let edition_id = format!("{}:content:{}", book_id, asset.sha256);
            state.editions.insert(
                edition_id.clone(),
                booknook_core::model::Edition {
                    id: edition_id.clone(),
                    book_id: book_id.clone(),
                    identifiers: vec![],
                    language: None,
                    assets: vec![asset],
                },
            );
            state
                .books
                .get_mut(&book_id)
                .unwrap()
                .edition_ids
                .push(edition_id);
        }
        let evidence = booknook_core::model::ImportEvidence {
            provider_connection_id: execution.request.connection_id.clone(),
            source: booknook_core::model::SourceRef {
                provider: runtime
                    .registry
                    .descriptor(&execution.request.provider_id)?
                    .source_namespace
                    .clone(),
                id: source_id.clone(),
            },
            imported_at: now()?,
            digest: sha256.clone(),
            fields: BTreeMap::from([
                ("media_type".into(), media_type.clone()),
                ("source_id".into(), source_id.clone()),
                ("sha256".into(), sha256.clone()),
            ]),
        };
        let book = state.books.get_mut(&book_id).unwrap();
        if !book.provenance.iter().any(|old| {
            old.digest == evidence.digest
                && old.source == evidence.source
                && old.provider_connection_id == evidence.provider_connection_id
        }) {
            book.provenance.push(evidence);
        }
        state.revision += 1;
        store.save_for_provider(
            revision,
            &state,
            &runtime.pending[&execution.request.request_id].settings_digest,
        )?;
        Ok(serde_json::to_string(&state)?)
    }
    pub fn provider_attach_audio(
        &self,
        execution_json: String,
        observation_json: String,
        pass_id: String,
        confirmed_edition_id: Option<String>,
    ) -> Result<String> {
        let runtime = self.provider_lock()?;
        let execution = p::validate_execution(
            &runtime.registry,
            runtime.generation,
            execution_json.as_bytes(),
            &[],
            now()?,
        )?;
        let p::ProviderResult::Positions { observations } = &execution.response.result else {
            return Err(Error::Invalid("Expected audio positions result".into()).into());
        };
        let source: booknook_core::audio::AudioObservation =
            strict_parse(observation_json.as_bytes())?;
        if !observations.contains(&source) {
            return Err(Error::Invalid(
                "Selected audio observation is not in this provider read".into(),
            )
            .into());
        }
        let mut store = self.lock()?;
        let settings = crate::connection_bridge::load(&store.connection)?;
        runtime.check_read(
            &execution,
            &booknook_core::digest(serde_json::to_string(&settings)?.as_bytes()),
        )?;
        let mut observation = booknook_core::audio::reviewed_observation(
            &store.load()?,
            &source,
            &pass_id,
            confirmed_edition_id.as_deref(),
            now()?,
        )?;
        if let Some(connection) = execution.request.connection_id.clone() {
            observation.id = booknook_core::digest(&canonical(&(
                "booknook.source-bound-position.v1",
                &connection,
                &observation.id,
            ))?);
            observation.provider_connection_id = Some(connection);
        }
        let mut state = store.load()?;
        let revision = state.revision;
        let result = booknook_core::apply(
            &mut state,
            booknook_core::Command::ObservePosition { observation },
        )?;
        store.save_for_provider(
            revision,
            &state,
            &runtime.pending[&execution.request.request_id].settings_digest,
        )?;
        Ok(serde_json::to_string(
            &booknook_core::providers::CommandResponse { state, result },
        )?)
    }
}

impl Desk {
    async fn builtin_nostr_request(&self, request: &p::ProviderRequest) -> Result<ProviderReply> {
        let config: booknook_core::settings::ConnectionSettings =
            strict_parse(self.connection_settings()?.as_bytes())?;
        let request = request.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let result = Self::run_builtin_nostr_request(config, request);
            let _ = sender.send(result);
        });
        receiver.await.map_err(|_| NativeError::Failure {
            message: "Provider network worker stopped".into(),
        })?
    }
    fn run_builtin_nostr_request(
        config: booknook_core::settings::ConnectionSettings,
        request: p::ProviderRequest,
    ) -> Result<ProviderReply> {
        use nostr_sdk::prelude::{Event, JsonUtil, PublicKey};
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| NativeError::Failure {
                message: e.to_string(),
            })?;
        // UniFFI's executor does not provide a Tokio reactor. Run network IO on this runtime;
        // no notebook/store lock is held while waiting for actual acknowledgments.
        let result = runtime.block_on(async {
            let transport = booknook_connectors::nostr_transport::SocialRelayTransport::connect(
                &config.nostr_relays,
            )
            .await?;
            let result = match &request.operation {
                p::ProviderOperation::SocialPublish { event } => {
                    let event = Event::from_json(serde_json::to_vec(event)?)
                        .map_err(|_| Error::Invalid("Malformed signed Nostr event".into()))?;
                    let relays = transport.publish_signed_event(&event).await?;
                    Ok(p::ProviderResult::SocialPublish {
                        event_id: event.id.to_hex(),
                        relays: relays.into_iter().collect(),
                    })
                }
                p::ProviderOperation::SocialFetch {
                    authors,
                    recipient,
                    since,
                    before,
                    limit,
                } => {
                    let authors = authors
                        .iter()
                        .map(|s| {
                            PublicKey::from_hex(s)
                                .map_err(|_| Error::Invalid("Invalid transport key".into()))
                        })
                        .collect::<booknook_core::Result<Vec<_>>>()?;
                    let recipient = recipient
                        .as_ref()
                        .map(|s| {
                            PublicKey::from_hex(s)
                                .map_err(|_| Error::Invalid("Invalid recipient".into()))
                        })
                        .transpose()?;
                    let mut events = transport
                        .fetch_page(authors, recipient, *since, *before)
                        .await?;
                    events.truncate(usize::from(*limit));
                    Ok(p::ProviderResult::SocialFetch {
                        events: events
                            .into_iter()
                            .map(|e| serde_json::to_value(e).map_err(booknook_core::Error::from))
                            .collect::<booknook_core::Result<Vec<_>>>()?,
                    })
                }
                _ => Err(Error::Invalid("Expected social transport operation".into())),
            };
            transport.disconnect().await;
            result
        })?;
        let response = p::ProviderResponse {
            schema: "booknook.provider-response.v1".into(),
            request_id: request.request_id.clone(),
            generation: request.generation,
            provider_id: request.provider_id.clone(),
            connection_id: request.connection_id.clone(),
            result,
        };
        Ok(ProviderReply {
            response_json: serde_json::to_string(&response)?,
            content_bytes: vec![],
        })
    }
}

#[uniffi::export]
impl Desk {
    pub fn provider_attach_position(
        &self,
        execution_json: String,
        observation_json: String,
        pass_id: String,
    ) -> Result<String> {
        let runtime = self.provider_lock()?;
        let execution = p::validate_execution(
            &runtime.registry,
            runtime.generation,
            execution_json.as_bytes(),
            &[],
            now()?,
        )?;
        let mut store = self.lock()?;
        let settings = crate::connection_bridge::load(&store.connection)?;
        runtime.check_read(
            &execution,
            &booknook_core::digest(serde_json::to_string(&settings)?.as_bytes()),
        )?;
        let mut state = store.load()?;
        let revision = state.revision;
        let result = p::apply_source_position(
            &mut state,
            &runtime.registry,
            runtime.generation,
            execution_json.as_bytes(),
            observation_json.as_bytes(),
            &pass_id,
            now()?,
        )?;
        store.save_for_provider(
            revision,
            &state,
            &runtime.pending[&execution.request.request_id].settings_digest,
        )?;
        Ok(serde_json::to_string(
            &booknook_core::providers::CommandResponse { state, result },
        )?)
    }
}
