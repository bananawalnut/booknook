// SPDX-License-Identifier: MIT OR Apache-2.0
//! Independent v1 host source port. Imported settings never install transport or
//! activate a source. Advertisements are negotiation data, not network authority.
//!
//! Bytes are bounded but explicitly UNTRUSTED: no DNS/redirect/credential policy,
//! XML safety, sanitization, normalization or durable cache implementation is
//! supplied here. Those remain separate acquisition, parsing and storage stages.
//! A newer explicit refresh supersedes older pending requests/candidates for that
//! source; this is local request ordering, never remote timestamp-wins merging.
//! Async storage may stage outside the runtime guard; its final synchronous CAS
//! must run through `with_current_cache_commit`, atomically compare the supplied
//! condition in its own durable store, and not reenter this session. An adapter
//! requiring an async final commit needs a separately qualified shared-generation
//! transaction; pre/post await checks alone cannot make that transaction atomic.
use crate::{
    Error, Result,
    feeds::{FeedConfiguration, FeedFormat, MAX_REVISION, endpoint, uuid},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex, MutexGuard, Weak,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
};
pub const MAX_FEED_BODY_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_SOURCE_METADATA_BYTES: usize = 16 * 1024;
pub const MAX_PENDING_FEED_READS: usize = 128;
pub type FeedFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;
fn invalid(message: &str) -> Error {
    Error::Invalid(message.into())
}
fn conflict() -> Error {
    Error::Conflict(
        "Feed attempt is cancelled, consumed or stale; explicitly activate and request again"
            .into(),
    )
}
fn text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}
pub(crate) fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
}
fn parse<T: for<'de> Deserialize<'de>>(raw: &[u8]) -> Result<T> {
    if raw.is_empty() || raw.len() > MAX_SOURCE_METADATA_BYTES {
        return Err(invalid("Feed source metadata exceeds 16 KiB or is empty"));
    }
    crate::strict_parse(raw)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum FeedSourceProtocol {
    #[serde(rename = "public-xml-bytes.v1")]
    PublicXmlBytesV1,
}
crate::feeds::deserialize_feed_string_enum!(FeedSourceProtocol { PublicXmlBytesV1 => "public-xml-bytes.v1" });
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedSourceDescriptor {
    pub schema: String,
    pub version: u8,
    pub id: String,
    pub available: bool,
    pub protocols: Vec<FeedSourceProtocol>,
}
impl FeedSourceDescriptor {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        let result: Self = parse(raw)?;
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<()> {
        if self.schema != "booknook.feed-source-provider.v1"
            || self.version != 1
            || !safe_id(&self.id)
            || self.protocols != [FeedSourceProtocol::PublicXmlBytesV1]
        {
            return Err(invalid("Unsupported or invalid feed source descriptor"));
        }
        Ok(())
    }
}
fn nullable<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> std::result::Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}
fn http_date(value: &str) -> bool {
    if value.len() != 29 || !value.is_ascii() {
        return false;
    }
    let b = value.as_bytes();
    if &b[3..5] != b", "
        || b[7] != b' '
        || b[11] != b' '
        || b[16] != b' '
        || b[19] != b':'
        || b[22] != b':'
        || &b[25..] != b" GMT"
        || !["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].contains(&&value[..3])
    {
        return false;
    }
    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let Some(month) = months.iter().position(|m| *m == &value[8..11]) else {
        return false;
    };
    let number = |part: &str| -> Option<u16> {
        if !part.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let (Some(day), Some(year), Some(hour), Some(minute), Some(second)) = (
        number(&value[5..7]),
        number(&value[12..16]),
        number(&value[17..19]),
        number(&value[20..22]),
        number(&value[23..25]),
    ) else {
        return false;
    };
    let days = if month == 1 && year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
        29
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][month]
    };
    year >= 1601 && day >= 1 && day <= days && hour < 24 && minute < 60 && second < 60
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedValidators {
    #[serde(deserialize_with = "nullable")]
    pub etag: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub last_modified: Option<String>,
}
impl FeedValidators {
    pub fn empty() -> Self {
        Self {
            etag: None,
            last_modified: None,
        }
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(tag) = &self.etag {
            let quoted = tag.strip_prefix("W/").unwrap_or(tag);
            if tag.len() > 512
                || quoted.len() < 2
                || !quoted.starts_with('"')
                || !quoted.ends_with('"')
                || !quoted.as_bytes()[1..quoted.len() - 1]
                    .iter()
                    .all(|b| *b == 0x21 || (0x23..=0x7e).contains(b))
            {
                return Err(invalid(
                    "ETag must be a bounded ASCII entity-tag without controls or header injection",
                ));
            }
        }
        if self
            .last_modified
            .as_ref()
            .is_some_and(|date| !http_date(date))
        {
            return Err(invalid(
                "Last-Modified must be a valid bounded IMF-fixdate without controls",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedConditionalHint {
    pub schema: String,
    pub source_id: String,
    pub provider_id: String,
    pub url: String,
    pub body_sha256: String,
    pub validators: FeedValidators,
}
impl FeedConditionalHint {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        let hint: Self = parse(raw)?;
        hint.validate()?;
        Ok(hint)
    }
    pub fn validate(&self) -> Result<()> {
        if self.schema != "booknook.feed-conditional-hint.v1"
            || !safe_id(&self.provider_id)
            || self.body_sha256.len() != 64
            || !self
                .body_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || (self.validators.etag.is_none() && self.validators.last_modified.is_none())
        {
            return Err(invalid(
                "Invalid conditional hint binding, digest or absent validators",
            ));
        }
        uuid(&self.source_id)?;
        endpoint(&self.url)?;
        self.validators.validate()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FeedSourceOutcome {
    Body,
    NotModified,
}
crate::feeds::deserialize_feed_string_enum!(FeedSourceOutcome { Body => "body", NotModified => "not-modified" });

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedSourceRequest {
    pub schema: String,
    pub runtime_id: String,
    pub request_id: String,
    pub source_id: String,
    pub provider_id: String,
    pub configuration_generation: u64,
    pub provider_generation: u64,
    pub activation_generation: u64,
    pub url: String,
    pub format: FeedFormat,
    pub protocol: FeedSourceProtocol,
    #[serde(deserialize_with = "nullable")]
    pub conditional: Option<FeedConditionalHint>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedSourceResponse {
    pub schema: String,
    pub runtime_id: String,
    pub request_id: String,
    pub source_id: String,
    pub provider_id: String,
    pub configuration_generation: u64,
    pub provider_generation: u64,
    pub activation_generation: u64,
    pub final_url: String,
    #[serde(deserialize_with = "nullable")]
    pub content_type: Option<String>,
    pub body_sha256: String,
    pub outcome: FeedSourceOutcome,
    pub validators: FeedValidators,
}
impl FeedSourceResponse {
    pub fn for_request(
        request: &FeedSourceRequest,
        final_url: &str,
        content_type: &str,
        body: &[u8],
    ) -> Result<Self> {
        let response = Self {
            schema: "booknook.feed-source-response.v1".into(),
            runtime_id: request.runtime_id.clone(),
            request_id: request.request_id.clone(),
            source_id: request.source_id.clone(),
            provider_id: request.provider_id.clone(),
            configuration_generation: request.configuration_generation,
            provider_generation: request.provider_generation,
            activation_generation: request.activation_generation,
            final_url: final_url.into(),
            content_type: Some(content_type.into()),
            body_sha256: crate::digest(body),
            outcome: FeedSourceOutcome::Body,
            validators: FeedValidators::empty(),
        };
        response.validate(request, body)?;
        Ok(response)
    }
    pub fn not_modified(
        request: &FeedSourceRequest,
        final_url: &str,
        validators: FeedValidators,
    ) -> Result<Self> {
        let hint = request
            .conditional
            .as_ref()
            .ok_or_else(|| invalid("No conditional hint was issued"))?;
        let response = Self {
            schema: "booknook.feed-source-response.v1".into(),
            runtime_id: request.runtime_id.clone(),
            request_id: request.request_id.clone(),
            source_id: request.source_id.clone(),
            provider_id: request.provider_id.clone(),
            configuration_generation: request.configuration_generation,
            provider_generation: request.provider_generation,
            activation_generation: request.activation_generation,
            final_url: final_url.into(),
            content_type: None,
            body_sha256: hint.body_sha256.clone(),
            outcome: FeedSourceOutcome::NotModified,
            validators,
        };
        response.validate(request, &[])?;
        Ok(response)
    }
    fn validate(&self, request: &FeedSourceRequest, body: &[u8]) -> Result<()> {
        if self.schema != "booknook.feed-source-response.v1"
            || self.runtime_id != request.runtime_id
            || self.request_id != request.request_id
            || self.source_id != request.source_id
            || self.provider_id != request.provider_id
            || self.configuration_generation != request.configuration_generation
            || self.provider_generation != request.provider_generation
            || self.activation_generation != request.activation_generation
        {
            return Err(conflict());
        }
        endpoint(&self.final_url)?;
        self.validators.validate()?;
        match self.outcome {
            FeedSourceOutcome::Body => {
                if body.is_empty()
                    || body.len() > MAX_FEED_BODY_BYTES
                    || !self.content_type.as_ref().is_some_and(|s| text(s, 128))
                    || self.body_sha256 != crate::digest(body)
                {
                    return Err(invalid(
                        "Invalid untrusted feed body, content type or digest",
                    ));
                }
            }
            FeedSourceOutcome::NotModified => {
                let hint = request.conditional.as_ref().ok_or_else(|| {
                    invalid("Unsolicited not-modified outcome without issued conditional hint")
                })?;
                if !body.is_empty()
                    || self.content_type.is_some()
                    || self.final_url != request.url
                    || self.body_sha256 != hint.body_sha256
                {
                    return Err(invalid(
                        "Not-modified must match the issued exact endpoint and prior body, without body or invented content type",
                    ));
                }
            }
        }
        Ok(())
    }
}
pub struct FeedSourceReply {
    pub response_bytes: Vec<u8>,
    pub body: Vec<u8>,
}
/// An installed host implementation, not a downloaded descriptor. The provider
/// owns safe I/O and must observe cancellation; late replies are rejected anyway.
pub trait FeedSourceProvider: Send + Sync {
    fn descriptor(&self) -> Result<FeedSourceDescriptor>;
    fn fetch<'a>(
        &'a self,
        request: FeedSourceRequest,
        cancellation: FeedCancellation,
    ) -> FeedFuture<'a, FeedSourceReply>;
}
#[derive(Clone)]
pub struct FeedCancellation {
    cancelled: Arc<AtomicU8>,
}
impl FeedCancellation {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst) == 1
    }
    /// False once the final synchronous commit has started; cancellation cannot
    /// retract a transaction already accepted at that linearization boundary.
    pub fn cancel(&self) -> bool {
        match self
            .cancelled
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
        {
            Ok(_) => true,
            Err(value) => value == 1,
        }
    }
}
struct Activation {
    provider_id: String,
    generation: u64,
}
struct Pending {
    source_id: String,
    cancelled: Weak<AtomicU8>,
}
struct State {
    closed: bool,
    runtime_id: String,
    configuration: FeedConfiguration,
    providers: BTreeMap<String, FeedSourceDescriptor>,
    configuration_generation: u64,
    provider_generation: u64,
    activation_generation: u64,
    active: BTreeMap<String, Activation>,
    pending: BTreeMap<String, Pending>,
}
impl State {
    fn revoke(&mut self) {
        for flag in self.pending.values().filter_map(|p| p.cancelled.upgrade()) {
            let _ = flag.compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst);
        }
        self.pending.clear();
        self.active.clear();
    }
    fn next(value: &mut u64) -> Result<()> {
        if *value >= MAX_REVISION {
            return Err(invalid("Feed generation exhausted; create a new session"));
        }
        *value += 1;
        Ok(())
    }
    fn check(&self, request: &FeedSourceRequest, cancellation: &FeedCancellation) -> Result<()> {
        if self.closed
            || cancellation.is_cancelled()
            || request.runtime_id != self.runtime_id
            || request.configuration_generation != self.configuration_generation
            || request.provider_generation != self.provider_generation
        {
            return Err(conflict());
        }
        let source = self
            .configuration
            .feeds
            .subscriptions
            .iter()
            .find(|s| s.id == request.source_id)
            .ok_or_else(conflict)?;
        let active = self.active.get(&request.source_id).ok_or_else(conflict)?;
        let provider = self
            .providers
            .get(&request.provider_id)
            .ok_or_else(conflict)?;
        if !source.enabled
            || source.url != request.url
            || source.format != request.format
            || !provider.available
            || active.provider_id != request.provider_id
            || active.generation != request.activation_generation
        {
            return Err(conflict());
        }
        Ok(())
    }
}
fn providers(values: Vec<FeedSourceDescriptor>) -> Result<BTreeMap<String, FeedSourceDescriptor>> {
    if values.len() > 32 {
        return Err(invalid("At most 32 installed feed providers are supported"));
    }
    let mut result = BTreeMap::new();
    for value in values {
        value.validate()?;
        if result.insert(value.id.clone(), value).is_some() {
            return Err(invalid("Duplicate installed feed provider"));
        }
    }
    Ok(result)
}
#[derive(Clone)]
pub struct FeedSourceSession {
    state: Arc<Mutex<State>>,
}
impl FeedSourceSession {
    pub fn new(
        configuration: FeedConfiguration,
        installed: Vec<FeedSourceDescriptor>,
    ) -> Result<Self> {
        configuration.validate()?;
        Ok(Self {
            state: Arc::new(Mutex::new(State {
                closed: false,
                runtime_id: crate::new_id(),
                configuration,
                providers: providers(installed)?,
                configuration_generation: 1,
                provider_generation: 1,
                activation_generation: 0,
                active: BTreeMap::new(),
                pending: BTreeMap::new(),
            })),
        })
    }
    // Never block on host reentrancy: a final CAS callback must not mutate this
    // runtime while holding its guard. Concurrent hosts may retry after conflict.
    fn lock(&self) -> Result<MutexGuard<'_, State>> {
        self.state.try_lock().map_err(|_| {
            Error::Conflict(
                "Feed runtime busy or poisoned; retry without reentrant callbacks".into(),
            )
        })
    }
    pub fn close(&self) -> Result<()> {
        let mut state = self.lock()?;
        state.revoke();
        state.closed = true;
        Ok(())
    }
    pub fn replace_configuration(&self, configuration: FeedConfiguration) -> Result<()> {
        configuration.validate()?;
        let mut state = self.lock()?;
        if state.closed {
            return Err(conflict());
        }
        State::next(&mut state.configuration_generation)?;
        state.revoke();
        state.configuration = configuration;
        Ok(())
    }
    pub fn replace_providers(&self, installed: Vec<FeedSourceDescriptor>) -> Result<()> {
        let installed = providers(installed)?;
        let mut state = self.lock()?;
        if state.closed {
            return Err(conflict());
        }
        State::next(&mut state.provider_generation)?;
        state.revoke();
        state.providers = installed;
        Ok(())
    }
    pub fn activate(&self, source_id: &str, provider_id: &str) -> Result<()> {
        let mut state = self.lock()?;
        if state.closed {
            return Err(conflict());
        }
        let source = state
            .configuration
            .feeds
            .subscriptions
            .iter()
            .find(|s| s.id == source_id)
            .ok_or_else(|| invalid("Unknown feed source"))?;
        if !source.enabled
            || !state
                .providers
                .get(provider_id)
                .is_some_and(|p| p.available)
        {
            return Err(Error::Denied(
                "Source must be enabled and an available host provider explicitly installed".into(),
            ));
        }
        State::next(&mut state.activation_generation)?;
        let generation = state.activation_generation;
        for pending in state.pending.values().filter(|p| p.source_id == source_id) {
            if let Some(flag) = pending.cancelled.upgrade() {
                let _ = flag.compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst);
            }
        }
        state.pending.retain(|_, p| p.source_id != source_id);
        state.active.insert(
            source_id.into(),
            Activation {
                provider_id: provider_id.into(),
                generation,
            },
        );
        Ok(())
    }
    pub fn disable(&self, source_id: &str) -> Result<()> {
        let mut state = self.lock()?;
        if !state
            .configuration
            .feeds
            .subscriptions
            .iter()
            .any(|s| s.id == source_id)
        {
            return Err(invalid("Unknown feed source"));
        }
        state.active.remove(source_id);
        for pending in state.pending.values().filter(|p| p.source_id == source_id) {
            if let Some(flag) = pending.cancelled.upgrade() {
                let _ = flag.compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst);
            }
        }
        state.pending.retain(|_, p| p.source_id != source_id);
        Ok(())
    }
    pub fn begin(&self, source_id: &str) -> Result<FeedSourceAttempt> {
        self.begin_with_conditional(source_id, None)
    }
    pub fn begin_with_conditional(
        &self,
        source_id: &str,
        conditional: Option<FeedConditionalHint>,
    ) -> Result<FeedSourceAttempt> {
        if let Some(hint) = &conditional {
            hint.validate()?;
        }
        let mut state = self.lock()?;
        let active = state.active.get(source_id).ok_or_else(|| {
            Error::Denied("Feed source is locally inactive; explicit activation required".into())
        })?;
        let source = state
            .configuration
            .feeds
            .subscriptions
            .iter()
            .find(|s| s.id == source_id)
            .ok_or_else(conflict)?;
        let request = FeedSourceRequest {
            schema: "booknook.feed-source-request.v1".into(),
            runtime_id: state.runtime_id.clone(),
            request_id: crate::new_id(),
            source_id: source_id.into(),
            provider_id: active.provider_id.clone(),
            configuration_generation: state.configuration_generation,
            provider_generation: state.provider_generation,
            activation_generation: active.generation,
            url: source.url.clone(),
            format: source.format,
            protocol: FeedSourceProtocol::PublicXmlBytesV1,
            conditional,
        };
        if request.conditional.as_ref().is_some_and(|hint| {
            hint.source_id != request.source_id
                || hint.provider_id != request.provider_id
                || hint.url != request.url
        }) {
            return Err(invalid(
                "Conditional hint is not bound to this source/provider/exact endpoint",
            ));
        }
        let cancellation = FeedCancellation {
            cancelled: Arc::new(AtomicU8::new(0)),
        };
        state.check(&request, &cancellation)?;
        for pending in state.pending.values().filter(|p| p.source_id == source_id) {
            if let Some(flag) = pending.cancelled.upgrade() {
                let _ = flag.compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst);
            }
        }
        state.pending.retain(|_, pending| {
            pending.source_id != source_id
                && pending
                    .cancelled
                    .upgrade()
                    .is_some_and(|flag| flag.load(Ordering::SeqCst) == 0)
        });
        if state.pending.len() >= MAX_PENDING_FEED_READS {
            return Err(invalid(
                "Too many pending feed reads; release or cancel old reads",
            ));
        }
        let descriptor = state.providers.get(&request.provider_id).unwrap().clone();
        state.pending.insert(
            request.request_id.clone(),
            Pending {
                source_id: source_id.into(),
                cancelled: Arc::downgrade(&cancellation.cancelled),
            },
        );
        Ok(FeedSourceAttempt {
            session: self.clone(),
            request,
            descriptor,
            cancellation,
            phase: AtomicU8::new(0),
        })
    }
}
pub struct FeedSourceAttempt {
    session: FeedSourceSession,
    request: FeedSourceRequest,
    descriptor: FeedSourceDescriptor,
    cancellation: FeedCancellation,
    phase: AtomicU8,
}
impl FeedSourceAttempt {
    pub fn cancellation(&self) -> FeedCancellation {
        self.cancellation.clone()
    }
    pub fn cancel(&self) -> bool {
        self.cancellation.cancel()
    }
    /// Single dispatch; this returns minimal request data, never a private config.
    pub fn dispatch(&self) -> Result<FeedSourceRequest> {
        self.session
            .lock()?
            .check(&self.request, &self.cancellation)?;
        self.phase
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| conflict())?;
        Ok(self.request.clone())
    }
    pub async fn execute(&self, provider: &dyn FeedSourceProvider) -> Result<FeedSourceRead> {
        if provider.descriptor()? != self.descriptor {
            return Err(conflict());
        }
        let request = self.dispatch()?;
        let reply = provider.fetch(request, self.cancellation()).await?;
        self.complete(&reply.response_bytes, reply.body)
    }
    pub fn complete(&self, response_bytes: &[u8], body: Vec<u8>) -> Result<FeedSourceRead> {
        if self
            .phase
            .compare_exchange(1, 2, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(conflict());
        }
        self.session
            .lock()?
            .check(&self.request, &self.cancellation)?;
        if body.len() > MAX_FEED_BODY_BYTES {
            return Err(invalid("Untrusted feed body exceeds 2 MiB"));
        }
        let response: FeedSourceResponse = parse(response_bytes)?;
        response.validate(&self.request, &body)?;
        // Validation may be expensive; never treat its start as the commit check.
        self.session
            .lock()?
            .check(&self.request, &self.cancellation)?;
        Ok(FeedSourceRead {
            session: self.session.clone(),
            request: self.request.clone(),
            response,
            body,
            cancellation: self.cancellation(),
            committed: AtomicBool::new(false),
        })
    }
}
/// A data-only expected CAS condition. Its serialization cannot recreate an
/// opaque read and grants no authority. Hosts must compare their stored condition
/// atomically in the final callback, never use this snapshot as a later permit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FeedCacheCondition {
    pub schema: String,
    pub runtime_id: String,
    pub request_id: String,
    pub source_id: String,
    pub provider_id: String,
    pub configuration_generation: u64,
    pub provider_generation: u64,
    pub activation_generation: u64,
    pub body_sha256: String,
    pub url: String,
    pub final_url: String,
    pub outcome: FeedSourceOutcome,
    pub expected_body_sha256: Option<String>,
    pub validators: FeedValidators,
}
pub struct FeedSourceRead {
    session: FeedSourceSession,
    request: FeedSourceRequest,
    response: FeedSourceResponse,
    body: Vec<u8>,
    cancellation: FeedCancellation,
    committed: AtomicBool,
}
impl FeedSourceRead {
    pub fn request(&self) -> &FeedSourceRequest {
        &self.request
    }
    pub fn response(&self) -> &FeedSourceResponse {
        &self.response
    }
    pub fn body(&self) -> &[u8] {
        &self.body
    }
    pub fn cancellation(&self) -> FeedCancellation {
        self.cancellation.clone()
    }
    fn cache_validators(&self) -> FeedValidators {
        if self.response.outcome == FeedSourceOutcome::NotModified {
            let previous = &self
                .request
                .conditional
                .as_ref()
                .expect("validated not-modified hint")
                .validators;
            FeedValidators {
                etag: self
                    .response
                    .validators
                    .etag
                    .clone()
                    .or_else(|| previous.etag.clone()),
                last_modified: self
                    .response
                    .validators
                    .last_modified
                    .clone()
                    .or_else(|| previous.last_modified.clone()),
            }
        } else if self.response.final_url == self.request.url {
            self.response.validators.clone()
        } else {
            // Observed response validators belong to the final endpoint. Do not
            // make them reusable conditional headers for a different redirector.
            FeedValidators::empty()
        }
    }
    fn condition(&self) -> FeedCacheCondition {
        FeedCacheCondition {
            schema: "booknook.feed-cache-condition.v1".into(),
            runtime_id: self.request.runtime_id.clone(),
            request_id: self.request.request_id.clone(),
            source_id: self.request.source_id.clone(),
            provider_id: self.request.provider_id.clone(),
            configuration_generation: self.request.configuration_generation,
            provider_generation: self.request.provider_generation,
            activation_generation: self.request.activation_generation,
            body_sha256: self.response.body_sha256.clone(),
            url: self.request.url.clone(),
            final_url: self.response.final_url.clone(),
            outcome: self.response.outcome,
            expected_body_sha256: if self.response.outcome == FeedSourceOutcome::NotModified {
                Some(self.response.body_sha256.clone())
            } else {
                None
            },
            validators: self.cache_validators(),
        }
    }
    pub fn cache_condition(&self) -> Result<FeedCacheCondition> {
        let state = self.session.lock()?;
        state.check(&self.request, &self.cancellation)?;
        if self.committed.load(Ordering::SeqCst) {
            return Err(conflict());
        }
        Ok(self.condition())
    }
    /// Final synchronous transaction boundary. The callback must atomically CAS
    /// its own durable generation/source key before storing untrusted bytes, and
    /// must not reenter this runtime or return before its durable commit finishes.
    /// Async staging belongs before this call; no async final transaction is implied.
    pub fn with_current_cache_commit(
        &self,
        existing_body: Option<&[u8]>,
        commit: impl FnOnce(&FeedCacheCondition) -> Result<()>,
    ) -> Result<()> {
        let state = self.session.lock()?;
        state.check(&self.request, &self.cancellation)?;
        match (self.response.outcome, existing_body) {
            (FeedSourceOutcome::Body, None) => {}
            (FeedSourceOutcome::NotModified, Some(body))
                if !body.is_empty()
                    && body.len() <= MAX_FEED_BODY_BYTES
                    && crate::digest(body) == self.response.body_sha256 => {}
            _ => {
                return Err(invalid(
                    "Not-modified cache admission requires actual matching retained bytes; body outcomes accept no retained-body substitute",
                ));
            }
        }

        if self.committed.load(Ordering::SeqCst) {
            return Err(conflict());
        }
        self.cancellation
            .cancelled
            .compare_exchange(0, 2, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| conflict())?;
        match commit(&self.condition()) {
            Ok(result) => {
                self.committed.store(true, Ordering::SeqCst);
                self.cancellation.cancelled.store(3, Ordering::SeqCst);
                Ok(result)
            }
            Err(error) => {
                self.cancellation.cancelled.store(0, Ordering::SeqCst);
                Err(error)
            }
        }
    }
    pub async fn store(&self, store: &dyn FeedCacheStore) -> Result<()> {
        self.cache_condition()?;
        store.compare_and_store(self).await?;
        // A later source change can reject this acknowledgement; it does not
        // roll back a transaction already validly accepted inside the guard.
        self.session
            .lock()?
            .check(&self.request, &self.cancellation)?;
        if !self.committed.load(Ordering::SeqCst) {
            return Err(invalid(
                "Cache adapter returned without a guarded atomic commit",
            ));
        }
        Ok(())
    }
}
/// Replaceable async staging + guarded final synchronous CAS. Implementations
/// must use the opaque candidate's commit callback after every await, not merely
/// call `cache_condition` before I/O. No backend is supplied or qualified here.
pub trait FeedCacheStore: Send + Sync {
    fn compare_and_store<'a>(&'a self, candidate: &'a FeedSourceRead) -> FeedFuture<'a, ()>;
}
