// SPDX-License-Identifier: MIT OR Apache-2.0
//! Standalone host-local native feed cache (Rust SDK, not a portable export).
//!
//! `open` is synchronous local I/O and MUST be called off the UI thread. All
//! subsequent operations use a dedicated bounded worker, never the network
//! executor/blocking pool. Drop never joins: the worker owns its DB/OS lock until
//! queued work is discarded and the worker exits; a concurrent reopen refuses.
//! An imported enabled preference never activates acquisition. No externally
//! constructed read, JSON condition or cache permit can be admitted here.
use crate::{FeedSourceHostV1, NativeError, Result};
use booknook_core::{
    feed_source as f, feed_xml as x,
    feeds::{FeedConfiguration, FeedSubscription},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    future::Future,
    path::Path,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    task::{Poll, Waker},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::oneshot;

/// Logical retained raw/metadata budget; NOT a physical SQLite/WAL disk cap.
/// Eviction uses admission/refresh recency, never an offline read-access write.
pub const FEED_CACHE_LOGICAL_QUOTA: u64 = 64 * 1024 * 1024;
pub const FEED_CACHE_MAX_ROWS: usize = 128;
pub const FEED_CACHE_QUEUE_CAPACITY: usize = 16;
const CONFIG_LIMIT: usize = booknook_core::feeds::MAX_SECTION_BYTES + 4096;
const META_LIMIT: usize = f::MAX_SOURCE_METADATA_BYTES;
fn fail(message: impl Into<String>) -> NativeError {
    NativeError::Failure {
        message: message.into(),
    }
}
fn core_error(error: NativeError) -> booknook_core::Error {
    booknook_core::Error::Unavailable(error.to_string())
}
fn token() -> String {
    let id = booknook_core::new_id();
    format!(
        "{}-{}-{}-{}-{}",
        &id[..8],
        &id[8..12],
        &id[12..16],
        &id[16..20],
        &id[20..]
    )
}
fn bound_token(raw: &[u8]) -> String {
    format!("{}:{}", token(), booknook_core::digest(raw))
}
fn check_bound_token(value: &str, raw: &[u8]) -> Result<()> {
    if !value.is_ascii()
        || value.len() != 101
        || value[37..] != booknook_core::digest(raw)
        || value.as_bytes()[36] != b':'
    {
        return Err(fail("Corrupt authoritative feed bytes/token binding"));
    }
    valid_token(&value[..36])
}
fn now() -> Result<i64> {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| fail("Local clock predates Unix epoch"))?
            .as_secs(),
    )
    .map_err(|_| fail("Local clock overflow"))
}
fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::SeqCst) {
        Err(fail("Feed operation cancelled before durable commit"))
    } else {
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FeedCacheConfiguration {
    pub raw: Vec<u8>,
    pub configuration_token: String,
    pub provider_token: String,
    pub provider_descriptor: f::FeedSourceDescriptor,
}
#[derive(Debug, Serialize)]
pub struct FeedCacheEntry {
    /// Stable per-source content variant key. Excludes source ordinal, duplicate
    /// count and conflict marker; does not collapse conflicting native IDs.
    pub variant_key: String,
    pub entry: x::NormalizedEntry,
}
#[derive(Debug, Serialize)]
pub struct FeedCacheView {
    pub schema: String,
    pub source_id: String,
    pub configuration_token: String,
    pub cache_token: String,
    pub context_sha256: String,
    pub raw_sha256: String,
    pub normalizer_profile: String,
    pub provenance: x::FeedProvenance,
    pub title: Option<x::InertText>,
    pub entries: Vec<FeedCacheEntry>,
    /// Local cache observation only; publisher dates remain in each entry.
    pub last_check_unix_seconds: i64,
    pub last_body_success_unix_seconds: i64,
    pub offline: bool,
}
#[derive(Debug, Serialize)]
pub struct FeedCacheStatus {
    pub source_id: String,
    pub locally_active: bool,
    pub cached: bool,
    pub last_check_unix_seconds: Option<i64>,
    pub last_body_success_unix_seconds: Option<i64>,
    pub last_error: Option<String>,
    /// Local error/inactive state, not a claim about publisher freshness.
    pub stale: bool,
}

/// Clone this coordinator across windows; a second independent open is refused.
#[derive(Clone)]
pub struct FeedCacheController {
    sender: Arc<mpsc::SyncSender<Message>>,
    operation: Option<Arc<OperationCancellation>>,
}
// One foreign call owns this scope. Synchronous cancellation fences queued or
// running storage BEFORE waking the foreign future; wake/drop alone races SQLite.
#[derive(Default)]
pub(crate) struct OperationCancellation {
    cancelled: AtomicBool,
    claimed: AtomicBool,
    state: Mutex<OperationState>,
}
#[derive(Default)]
struct OperationState {
    job: Option<Weak<AtomicBool>>,
    read: Option<f::FeedCancellation>,
    waker: Option<Waker>,
}
struct OperationWaiter<'a>(&'a OperationCancellation);
impl Drop for OperationWaiter<'_> {
    fn drop(&mut self) {
        // A future's waker can own the future itself. Never retain it after
        // completion/drop and create a scope -> waker -> future ownership cycle.
        let waker = self
            .0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .waker
            .take();
        drop(waker);
    }
}
impl OperationCancellation {
    pub(crate) fn claim(&self) -> Result<()> {
        self.claimed
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| ())
            .map_err(|_| fail("Feed cache operation is one-use"))
    }
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        let waker = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(job) = state.job.as_ref().and_then(Weak::upgrade) {
                job.store(true, Ordering::SeqCst);
            }
            if let Some(read) = &state.read {
                read.cancel();
            }
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
    fn register_job(&self, job: &Arc<AtomicBool>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.job = Some(Arc::downgrade(job));
        if self.cancelled.load(Ordering::SeqCst) {
            job.store(true, Ordering::SeqCst);
        }
    }
    fn register_read(&self, read: &f::FeedCancellation) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.read = Some(read.clone());
        if self.cancelled.load(Ordering::SeqCst) {
            read.cancel();
        }
    }
    pub(crate) async fn run<T>(&self, future: impl Future<Output = Result<T>>) -> Result<T> {
        let _waiter = OperationWaiter(self);
        let mut future = std::pin::pin!(future);
        let mut started = false;
        std::future::poll_fn(|cx| {
            {
                let mut state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                state.waker = Some(cx.waker().clone());
            }
            // Pre-cancel must not poll/enqueue even a lifecycle close. Once
            // started, preserve an already-ready successful acknowledgement.
            if !started && self.cancelled.load(Ordering::SeqCst) {
                return Poll::Ready(Err(fail("Feed operation cancelled before starting")));
            }
            started = true;
            if let Poll::Ready(result) = future.as_mut().poll(cx) { return Poll::Ready(result); }
            if self.cancelled.load(Ordering::SeqCst) {
                Poll::Ready(Err(fail("Feed operation cancelled; outcome may already be committed; inspect durable state")))
            } else { Poll::Pending }
        }).await
    }
}
type Job = Box<dyn FnOnce(&mut Worker) + Send>;
enum Message {
    Run(Job),
    Close(oneshot::Sender<Result<()>>),
}
struct CancelJob(Arc<AtomicBool>);
impl Drop for CancelJob {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
struct CancelRead(f::FeedCancellation);
impl Drop for CancelRead {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
impl FeedCacheController {
    /// Local filesystem I/O only; call off UI. `expected` initializes a new store
    /// or must EXACTLY match existing authoritative bytes. None requires an
    /// existing store. The supplied executable must match its persisted provider;
    /// changing provider is a separate compare-and-swap operation.
    pub fn open(
        directory: &Path,
        expected: Option<Vec<u8>>,
        host: Box<dyn FeedSourceHostV1>,
    ) -> Result<Self> {
        // Foreign descriptor callback is outside storage worker/gate/transaction.
        let provider = crate::feed_source::cache_installed_provider(host)?;
        Self::open_provider(directory, expected, provider)
    }
    /// Concrete provider construction is offline; refresh still needs activation.
    pub fn public_https(directory: &Path, expected: Option<Vec<u8>>) -> Result<Self> {
        Self::open_provider(
            directory,
            expected,
            crate::feed_source::cache_public_https_provider()?,
        )
    }
    fn open_provider(
        directory: &Path,
        expected: Option<Vec<u8>>,
        provider: Arc<dyn f::FeedSourceProvider>,
    ) -> Result<Self> {
        let descriptor = provider.descriptor()?; // cached adapter, no foreign code
        descriptor.validate()?;
        if let Some(raw) = &expected {
            FeedConfiguration::parse(raw)?;
        }
        let path = directory.to_path_buf();
        let (sender, receiver) = mpsc::sync_channel(FEED_CACHE_QUEUE_CAPACITY);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("booknook-feed-storage".into())
            .spawn(move || {
                let mut worker = match Worker::open(&path, expected, provider, descriptor) {
                    Ok(worker) => worker,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                if ready_tx.send(Ok(())).is_err() {
                    return;
                }
                while let Ok(message) = receiver.recv() {
                    match message {
                        Message::Run(job) => job(&mut worker),
                        Message::Close(reply) => {
                            let result = worker.close();
                            drop(worker); // release DB and owner lock BEFORE acknowledging
                            let _ = reply.send(result);
                            return;
                        }
                    }
                }
                let _ = worker.close();
            })?;
        ready_rx
            .recv()
            .map_err(|_| fail("Feed storage worker stopped during open"))??;
        Ok(Self {
            sender: Arc::new(sender),
            operation: None,
        })
    }
    pub(crate) fn scoped(&self, operation: Arc<OperationCancellation>) -> Self {
        Self {
            sender: self.sender.clone(),
            operation: Some(operation),
        }
    }
    async fn call<T: Send + 'static>(
        &self,
        action: impl FnOnce(&mut Worker, &AtomicBool) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let _cancel_on_drop = CancelJob(cancelled.clone());
        if let Some(scope) = &self.operation {
            scope.register_job(&cancelled);
        }
        check_cancel(&cancelled)?;
        let (reply, response) = oneshot::channel();
        // Capacity belongs to the queued/running closure, not the awaiting caller.
        self.sender
            .try_send(Message::Run(Box::new(move |worker| {
                let result = if cancelled.load(Ordering::SeqCst) {
                    Err(fail("Queued feed work cancelled"))
                } else if worker.poisoned {
                    Err(fail(
                        "Feed cache controller is closed or uncertain; reopen explicitly",
                    ))
                } else {
                    action(worker, &cancelled)
                };
                let _ = reply.send(result);
            })))
            .map_err(|_| fail("Feed storage queue is full or closed; retry explicitly"))?;
        response
            .await
            .map_err(|_| fail("Feed storage worker closed"))?
    }
    pub async fn configuration(&self) -> Result<FeedCacheConfiguration> {
        self.call(|w, _| {
            w.fence()?;
            Ok(w.snapshot())
        })
        .await
    }
    /// Cancellation is checked after SQLite waits, before COMMIT starts. Once
    /// COMMIT starts a dropped future cannot promise rollback.
    /// Exact expected token prevents stale UI replacement. This is data-only
    /// configuration admission, not approval to fetch or publish.
    pub async fn replace_configuration(&self, expected: &str, raw: Vec<u8>) -> Result<()> {
        let configuration = FeedConfiguration::parse(&raw)?;
        let expected = expected.to_owned();
        self.call(move |w, cancel| w.replace_configuration(&expected, raw, configuration, cancel))
            .await
    }
    pub async fn replace_provider(
        &self,
        expected: &str,
        host: Box<dyn FeedSourceHostV1>,
    ) -> Result<()> {
        let provider = crate::feed_source::cache_installed_provider(host)?;
        let descriptor = provider.descriptor()?;
        let expected = expected.to_owned();
        self.call(move |w, cancel| w.replace_provider(&expected, provider, descriptor, cancel))
            .await
    }
    pub async fn activate(&self, source: &str) -> Result<()> {
        let source = source.to_owned();
        self.call(move |w, cancel| w.activation(&source, true, cancel))
            .await
    }
    /// Revokes local network activation; cached text is not deleted.
    pub async fn disable(&self, source: &str) -> Result<()> {
        let source = source.to_owned();
        self.call(move |w, cancel| w.activation(&source, false, cancel))
            .await
    }
    pub async fn close(&self) -> Result<()> {
        let (reply, response) = oneshot::channel();
        self.sender
            .try_send(Message::Close(reply))
            .map_err(|_| fail("Feed storage queue is full or closed"))?;
        response
            .await
            .map_err(|_| fail("Feed worker closed without acknowledgement"))?
    }
    pub async fn read_status(&self, source: &str) -> Result<FeedCacheStatus> {
        let source = source.to_owned();
        self.call(move |w, _| w.status(&source)).await
    }
    /// No implicit fetch, activation, fallback or trusted normalized JSON decode.
    pub async fn offline(&self, source: &str) -> Result<FeedCacheView> {
        let source = source.to_owned();
        let staged = self
            .call(move |w, _| {
                w.fence()?;
                w.source(&source)?;
                let row = w
                    .load(&source)?
                    .ok_or_else(|| fail("Feed has no retained body"))?;
                w.check_row(&source, &row)?;
                Ok((row, w.configuration_token.clone()))
            })
            .await?;
        let (row, configuration_token) = staged;
        let raw_token = row.cache_token.clone();
        let view = self
            .call(move |_, cancel| {
                let feed = x::normalize(&row.context, &row.raw)?;
                if cancel.load(Ordering::SeqCst) {
                    return Err(fail("Offline normalization cancelled"));
                }
                view(feed, &row, &configuration_token, true)
            })
            .await?;
        let source = view.source_id.clone();
        let expected_configuration = view.configuration_token.clone();
        self.call(move |w, _| {
            if w.configuration_token != expected_configuration {
                return Err(fail("Stale offline configuration"));
            }
            // One READ transaction snapshot revalidates authority and exact row.
            // Offline display must not require a recency write under low disk.
            let tx = w.db.unchecked_transaction()?;
            check_owner(&tx, &w.owner, &w.configuration_token, &w.provider_token)?;
            let current = load_row(&tx, &source)?.ok_or_else(|| fail("Offline row was evicted"))?;
            w.check_row(&source, &current)?;
            if current.cache_token != raw_token {
                return Err(fail("Stale offline cache view"));
            }
            tx.commit()?;
            Ok(())
        })
        .await?;
        Ok(view)
    }
    pub async fn refresh(&self, source: &str) -> Result<FeedCacheView> {
        let source = source.to_owned();
        let start = self.call(move |w, cancel| w.begin(&source, cancel)).await?;
        let cancellation = start.attempt.cancellation();
        let _cancel_read = CancelRead(cancellation.clone());
        if let Some(scope) = &self.operation {
            scope.register_read(&cancellation);
            if scope.cancelled.load(Ordering::SeqCst) {
                return Err(fail("Feed refresh cancelled before transport"));
            }
        }
        // No storage gate, worker, transaction, or UI blocking during transport.
        let reply = match start
            .provider
            .fetch(start.request.clone(), cancellation)
            .await
        {
            Ok(reply) => reply,
            Err(error) => {
                self.record_error(&start.binding, &error.to_string()).await;
                return Err(error.into());
            }
        };
        let read = match start.attempt.complete(&reply.response_bytes, reply.body) {
            Ok(read) => read,
            Err(error) => {
                self.record_error(&start.binding, &error.to_string()).await;
                return Err(error.into());
            }
        };
        let binding = start.binding;
        let source = binding.source.clone();
        let outcome = read.response().outcome;
        let context = if outcome == f::FeedSourceOutcome::NotModified {
            start
                .retained
                .as_ref()
                .ok_or_else(|| fail("Conditional response has no retained snapshot"))?
                .context
                .clone()
        } else {
            serde_json::to_vec(&x::NormalizationContext {
                schema: "booknook.feed-normalization-context.v1".into(),
                source_id: source.clone(),
                provider_id: read.request().provider_id.clone(),
                requested_url: read.request().url.clone(),
                final_url: read.response().final_url.clone(),
                expected_format: read.request().format,
            })?
        };
        let raw = if outcome == f::FeedSourceOutcome::NotModified {
            start
                .retained
                .ok_or_else(|| fail("Missing conditional body"))?
                .raw
        } else {
            read.body().to_vec()
        };
        let normalized = self
            .call(move |_, cancel| {
                let feed = x::normalize(&context, &raw)?;
                if cancel.load(Ordering::SeqCst) {
                    return Err(fail("Feed normalization cancelled"));
                }
                Ok((feed, context, raw))
            })
            .await;
        let (feed, context, raw) = match normalized {
            Ok(value) => value,
            Err(error) => {
                self.record_error(&binding, &error.to_string()).await;
                return Err(error);
            }
        };
        let error_binding = binding.clone();
        match self
            .call(move |w, cancel| {
                if cancel.load(Ordering::SeqCst) {
                    read.cancellation().cancel();
                    return Err(fail("Feed admission cancelled"));
                }
                w.admit(binding, read, feed, context, raw, cancel)
            })
            .await
        {
            Ok(view) => Ok(view),
            Err(error) => {
                self.record_error(&error_binding, &error.to_string()).await;
                Err(error)
            }
        }
    }
    async fn record_error(&self, binding: &Binding, message: &str) {
        let binding = binding.clone();
        // Never persist arbitrary provider text (which can contain source URLs or
        // secrets). Bounded local classification only; callers receive the error.
        let _ = message;
        let _ = self
            .call(move |w, cancel| {
                let tx =
                    w.db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                check_binding(&tx, &w.owner, &binding)?;
                tx.execute(
                    "UPDATE sources SET last_error='Feed refresh failed' WHERE source_id=?1",
                    [&binding.source],
                )?;
                check_cancel(cancel)?;
                tx.commit()?;
                Ok(())
            })
            .await;
    }
}

struct OwnerLock(File);
impl OwnerLock {
    fn acquire(directory: &Path) -> Result<Self> {
        use std::os::unix::{
            fs::{OpenOptionsExt, PermissionsExt},
            io::AsRawFd,
        };
        fs::create_dir_all(directory)?;
        let meta = fs::symlink_metadata(directory)?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(fail("Feed cache requires a real host-controlled directory"));
        }
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(directory.join("feeds.owner-lock"))?;
        if !file.metadata()?.is_file() {
            return Err(fail("Feed cache owner lock is not a regular file"));
        }
        // SAFETY: live owned descriptor, nonblocking exclusive process lock.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(fail("Feed cache already has a live independent owner"));
        }
        Ok(Self(file))
    }
}
impl Drop for OwnerLock {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
struct Worker {
    db: Connection,
    path: std::path::PathBuf,
    _lock: OwnerLock,
    owner: String,
    raw_configuration: Vec<u8>,
    configuration: FeedConfiguration,
    configuration_token: String,
    provider_token: String,
    provider: Arc<dyn f::FeedSourceProvider>,
    descriptor: f::FeedSourceDescriptor,
    session: f::FeedSourceSession,
    poisoned: bool,
    quota: u64,
    max_rows: usize,
    #[cfg(test)]
    faults: Faults,
}
#[derive(Clone)]
struct Binding {
    source: String,
    configuration: String,
    provider: String,
    activation: String,
    request: String,
    cache: Option<String>,
}
struct Start {
    attempt: f::FeedSourceAttempt,
    request: f::FeedSourceRequest,
    provider: Arc<dyn f::FeedSourceProvider>,
    binding: Binding,
    retained: Option<Row>,
}
#[derive(PartialEq, Eq)]
struct Row {
    raw: Vec<u8>,
    sha: String,
    context: Vec<u8>,
    context_sha: String,
    profile: String,
    cache_token: String,
    request: String,
    validators: f::FeedValidators,
    checked: i64,
    body_success: i64,
}
impl Row {
    fn logical(&self) -> Result<u64> {
        u64::try_from(
            self.raw.len()
                + self.context.len()
                + serde_json::to_vec(&self.validators)?.len()
                + self.sha.len()
                + self.context_sha.len()
                + self.profile.len()
                + self.cache_token.len()
                + self.request.len()
                + 1024,
        )
        .map_err(|_| fail("Logical cache size overflow"))
    }
}
impl Worker {
    fn open(
        directory: &Path,
        expected: Option<Vec<u8>>,
        provider: Arc<dyn f::FeedSourceProvider>,
        descriptor: f::FeedSourceDescriptor,
    ) -> Result<Self> {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let lock = OwnerLock::acquire(directory)?;
        let directory = fs::canonicalize(directory)?;
        let path = directory.join("feeds.sqlite");
        let existed = path.try_exists()?;
        if !existed && expected.is_none() {
            return Err(fail("A new feed cache requires explicit configuration"));
        }
        // O_NOFOLLOW verifies DB path before SQLite opens it. Host-controlled private
        // directory is required; SQLite is not a defense against hostile directory owners.
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(!existed)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)?;
        if !file.metadata()?.is_file() {
            return Err(fail("Feed database is not a regular file"));
        }
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        drop(file);
        let mut db = Connection::open(&path)?;
        db.busy_timeout(std::time::Duration::from_millis(250))?;
        db.pragma_update(None, "foreign_keys", true)?;
        db.pragma_update(None, "synchronous", "FULL")?;
        // Existing DB must validate BEFORE any schema construction/journal changes.
        let descriptor_raw = serde_json::to_vec(&descriptor)?;
        if !existed {
            let raw = expected
                .as_ref()
                .ok_or_else(|| fail("Missing initial configuration"))?;
            FeedConfiguration::parse(raw)?;
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            tx.execute_batch("CREATE TABLE controller (id INTEGER PRIMARY KEY CHECK(id=1),schema_version INTEGER NOT NULL,owner TEXT NOT NULL,configuration BLOB NOT NULL,configuration_token TEXT NOT NULL,provider BLOB NOT NULL,provider_token TEXT NOT NULL,counter INTEGER NOT NULL); CREATE TABLE sources (source_id TEXT PRIMARY KEY,activation TEXT,latest_request TEXT,last_error TEXT); CREATE TABLE cache (source_id TEXT PRIMARY KEY,raw BLOB NOT NULL,sha TEXT NOT NULL,context BLOB NOT NULL,context_sha TEXT NOT NULL,profile TEXT NOT NULL,cache_token TEXT NOT NULL,request_id TEXT NOT NULL,validators BLOB NOT NULL,checked INTEGER NOT NULL,body_success INTEGER NOT NULL,lru INTEGER NOT NULL,logical INTEGER NOT NULL); CREATE TABLE receipts (source_id TEXT PRIMARY KEY,request_id TEXT NOT NULL,cache_token TEXT NOT NULL,context_sha TEXT NOT NULL,profile TEXT NOT NULL,raw_sha TEXT NOT NULL);")?;
            tx.execute(
                "INSERT INTO controller VALUES(1,1,'',?1,?2,?3,?4,0)",
                params![
                    raw,
                    bound_token(raw),
                    descriptor_raw,
                    bound_token(&descriptor_raw)
                ],
            )?;
            let config = FeedConfiguration::parse(raw)?;
            for source in &config.feeds.subscriptions {
                tx.execute("INSERT INTO sources(source_id) VALUES(?1)", [&source.id])?;
            }
            tx.commit()?;
        }
        let (schema,raw_configuration,configuration_token,stored_descriptor,provider_token):(i64,Vec<u8>,String,Vec<u8>,String)=db.query_row("SELECT schema_version,CASE WHEN typeof(configuration)='blob' AND length(configuration)<=?1 THEN configuration ELSE NULL END,CASE WHEN typeof(configuration_token)='text' AND length(CAST(configuration_token AS BLOB))=101 THEN configuration_token ELSE NULL END,CASE WHEN typeof(provider)='blob' AND length(provider)<=?2 THEN provider ELSE NULL END,CASE WHEN typeof(provider_token)='text' AND length(CAST(provider_token AS BLOB))=101 THEN provider_token ELSE NULL END FROM controller WHERE id=1",params![i64::try_from(CONFIG_LIMIT).map_err(|_|fail("Configuration bound overflow"))?,i64::try_from(META_LIMIT).map_err(|_|fail("Metadata bound overflow"))?],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
        if schema != 1 {
            return Err(fail(
                "Unsupported feed cache storage schema; no reset performed",
            ));
        }
        let configuration = FeedConfiguration::parse(&raw_configuration)?;
        if expected
            .as_ref()
            .is_some_and(|raw| raw != &raw_configuration)
        {
            return Err(fail(
                "Expected configuration does not exactly match authoritative feed cache",
            ));
        }
        if f::FeedSourceDescriptor::parse(&stored_descriptor)? != descriptor {
            return Err(fail(
                "Installed provider does not match persisted descriptor; replace through the live controller",
            ));
        }
        check_bound_token(&configuration_token, &raw_configuration)?;
        check_bound_token(&provider_token, &stored_descriptor)?;
        let session = f::FeedSourceSession::new(configuration.clone(), vec![descriptor.clone()])?;
        let counter: i64 = db.query_row("SELECT counter FROM controller WHERE id=1", [], |r| {
            r.get(0)
        })?;
        let row_count: i64 = db.query_row("SELECT COUNT(*) FROM cache", [], |r| r.get(0))?;
        if counter < 0 || row_count < 0 || row_count > FEED_CACHE_MAX_ROWS as i64 {
            return Err(fail("Invalid bounded cache ordering/row count"));
        }
        let owner = token();
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("UPDATE controller SET owner=?1 WHERE id=1", [&owner])?;
        // Strict authoritative source set; corruption is not repaired by deleting rows.
        let ids = {
            let mut q=tx.prepare("SELECT CASE WHEN typeof(source_id)='text' AND length(CAST(source_id AS BLOB))=36 THEN source_id ELSE NULL END FROM sources ORDER BY source_id LIMIT 257")?;
            q.query_map([], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        let mut expected_ids = configuration
            .feeds
            .subscriptions
            .iter()
            .map(|s| s.id.clone())
            .collect::<Vec<_>>();
        expected_ids.sort();
        if ids.len() > booknook_core::feeds::MAX_SUBSCRIPTIONS || ids != expected_ids {
            return Err(fail("Corrupt authoritative feed source set"));
        }
        tx.execute("UPDATE sources SET activation=NULL,latest_request=NULL", [])?;
        tx.commit()?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.pragma_update(None, "wal_autocheckpoint", 0)?;
        Ok(Self {
            db,
            path,
            _lock: lock,
            owner,
            raw_configuration,
            configuration,
            configuration_token,
            provider_token,
            provider,
            descriptor,
            session,
            poisoned: false,
            quota: FEED_CACHE_LOGICAL_QUOTA,
            max_rows: FEED_CACHE_MAX_ROWS,
            #[cfg(test)]
            faults: Faults::default(),
        })
    }
    fn snapshot(&self) -> FeedCacheConfiguration {
        FeedCacheConfiguration {
            raw: self.raw_configuration.clone(),
            configuration_token: self.configuration_token.clone(),
            provider_token: self.provider_token.clone(),
            provider_descriptor: self.descriptor.clone(),
        }
    }
    fn fence(&self) -> Result<()> {
        check_owner(
            &self.db,
            &self.owner,
            &self.configuration_token,
            &self.provider_token,
        )
    }
    fn source(&self, id: &str) -> Result<&FeedSubscription> {
        self.configuration
            .feeds
            .subscriptions
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(|| fail("Unknown configured feed source"))
    }
    fn replace_configuration(
        &mut self,
        expected: &str,
        raw: Vec<u8>,
        configuration: FeedConfiguration,
        cancel: &AtomicBool,
    ) -> Result<()> {
        self.fence()?;
        if expected != self.configuration_token {
            return Err(fail("Stale feed configuration token"));
        }
        let session =
            f::FeedSourceSession::new(configuration.clone(), vec![self.descriptor.clone()])?;
        let invalidated = self
            .configuration
            .feeds
            .subscriptions
            .iter()
            .filter(|old| {
                configuration
                    .feeds
                    .subscriptions
                    .iter()
                    .find(|new| new.id == old.id)
                    .is_none_or(|new| new.url != old.url || new.format != old.format)
            })
            .map(|s| s.id.clone())
            .collect::<Vec<_>>();
        let next = bound_token(&raw);
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        check_owner(&tx, &self.owner, expected, &self.provider_token)?;
        tx.execute(
            "UPDATE controller SET configuration=?1,configuration_token=?2 WHERE id=1",
            params![raw, next],
        )?;
        for source in invalidated {
            tx.execute("DELETE FROM cache WHERE source_id=?1", [&source])?;
            tx.execute("DELETE FROM receipts WHERE source_id=?1", [source])?;
        }
        tx.execute("DELETE FROM sources", [])?;
        for s in &configuration.feeds.subscriptions {
            tx.execute("INSERT INTO sources(source_id) VALUES(?1)", [&s.id])?;
        }
        check_cancel(cancel)?;
        #[cfg(test)]
        if self.faults.before_lifecycle_commit {
            return Err(fail("Injected lifecycle transaction failure before COMMIT"));
        }
        if tx.commit().is_err() {
            self.poisoned = true;
            let _ = self.session.close();
            return Err(fail(
                "Lifecycle COMMIT outcome uncertain; controller closed, reopen and inspect",
            ));
        }
        if self.session.close().is_err() {
            self.poisoned = true;
            return Err(fail(
                "Configuration committed but runtime reconciliation failed; reopen",
            ));
        }
        self.session = session;
        self.configuration = configuration;
        self.raw_configuration = raw;
        self.configuration_token = next;
        Ok(())
    }
    fn replace_provider(
        &mut self,
        expected: &str,
        provider: Arc<dyn f::FeedSourceProvider>,
        descriptor: f::FeedSourceDescriptor,
        cancel: &AtomicBool,
    ) -> Result<()> {
        descriptor.validate()?;
        self.fence()?;
        if expected != self.provider_token {
            return Err(fail("Stale feed provider token"));
        }
        let session =
            f::FeedSourceSession::new(self.configuration.clone(), vec![descriptor.clone()])?;
        let raw = serde_json::to_vec(&descriptor)?;
        let next = bound_token(&raw);
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        check_owner(&tx, &self.owner, &self.configuration_token, expected)?;
        if self.descriptor.id != descriptor.id {
            tx.execute("DELETE FROM cache", [])?;
            tx.execute("DELETE FROM receipts", [])?;
        }
        tx.execute(
            "UPDATE controller SET provider=?1,provider_token=?2 WHERE id=1",
            params![raw, next],
        )?;
        tx.execute("UPDATE sources SET activation=NULL,latest_request=NULL", [])?;
        check_cancel(cancel)?;
        #[cfg(test)]
        if self.faults.before_lifecycle_commit {
            return Err(fail("Injected lifecycle transaction failure before COMMIT"));
        }
        if tx.commit().is_err() {
            self.poisoned = true;
            let _ = self.session.close();
            return Err(fail(
                "Lifecycle COMMIT outcome uncertain; controller closed, reopen and inspect",
            ));
        }
        if self.session.close().is_err() {
            self.poisoned = true;
            return Err(fail(
                "Provider committed but runtime reconciliation failed; reopen",
            ));
        }
        self.session = session;
        self.provider = provider;
        self.descriptor = descriptor;
        self.provider_token = next;
        Ok(())
    }
    fn activation(&mut self, source: &str, active: bool, cancel: &AtomicBool) -> Result<()> {
        self.fence()?;
        let s = self.source(source)?;
        if active && (!s.enabled || !self.descriptor.available) {
            return Err(fail(
                "Feed preference/provider unavailable; explicit activation refused",
            ));
        }
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        check_owner(
            &tx,
            &self.owner,
            &self.configuration_token,
            &self.provider_token,
        )?;
        tx.execute(
            "UPDATE sources SET activation=?1,latest_request=NULL WHERE source_id=?2",
            params![active.then(token), source],
        )?;
        check_cancel(cancel)?;
        #[cfg(test)]
        if self.faults.before_lifecycle_commit {
            return Err(fail("Injected lifecycle transaction failure before COMMIT"));
        }
        if tx.commit().is_err() {
            self.poisoned = true;
            let _ = self.session.close();
            return Err(fail(
                "Lifecycle COMMIT outcome uncertain; controller closed, reopen and inspect",
            ));
        }
        let result = if active {
            self.session.activate(source, &self.descriptor.id)
        } else {
            self.session.disable(source)
        };
        if let Err(error) = result {
            self.poisoned = true;
            let _ = self.session.close();
            return Err(error.into());
        }
        Ok(())
    }
    fn close(&mut self) -> Result<()> {
        self.poisoned = true;
        let _ = self.session.close();
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE controller SET owner='' WHERE id=1 AND owner=?1",
            [&self.owner],
        )?;
        if changed == 1 {
            tx.execute("UPDATE sources SET activation=NULL,latest_request=NULL", [])?;
        }
        tx.commit()?;
        Ok(())
    }
    fn load(&self, source: &str) -> Result<Option<Row>> {
        load_row(&self.db, source)
    }
    fn check_row(&self, source: &str, row: &Row) -> Result<()> {
        let subscription = self.source(source)?;
        let context = x::NormalizationContext::parse(&row.context)?;
        if row.profile != x::NORMALIZER_PROFILE
            || row.context_sha != booknook_core::digest(&row.context)
            || row.sha != booknook_core::digest(&row.raw)
            || context.source_id != source
            || context.provider_id != self.descriptor.id
            || context.requested_url != subscription.url
            || context.expected_format != subscription.format
        {
            return Err(fail(
                "Retained feed binding, profile or raw digest is invalid",
            ));
        }
        if row.checked < 0 || row.body_success < 0 || row.body_success > row.checked {
            return Err(fail("Corrupt local feed timestamps"));
        }
        row.validators.validate()?;
        valid_token(&row.cache_token)?;
        valid_request(&row.request)?;
        Ok(())
    }
    fn begin(&mut self, source: &str, cancel: &AtomicBool) -> Result<Start> {
        self.fence()?;
        self.source(source)?;
        let activation:String=self.db.query_row("SELECT CASE WHEN activation IS NULL THEN NULL WHEN typeof(activation)='text' AND length(CAST(activation AS BLOB))=36 THEN activation ELSE zeroblob(1) END FROM sources WHERE source_id=?1",[source],|r|r.get(0)).map_err(|_|fail("Feed is locally inactive; explicitly activate"))?;
        valid_token(&activation)?;
        // Corrupt cached row is an isolated error, never an automatic network fallback.
        let retained = self.load(source)?;
        if let Some(row) = &retained {
            self.check_row(source, row)?;
        }
        let conditional = retained.as_ref().and_then(|row| {
            let context = x::NormalizationContext::parse(&row.context).ok()?;
            (context.final_url == context.requested_url
                && (row.validators.etag.is_some() || row.validators.last_modified.is_some()))
            .then(|| f::FeedConditionalHint {
                schema: "booknook.feed-conditional-hint.v1".into(),
                source_id: source.into(),
                provider_id: self.descriptor.id.clone(),
                url: context.requested_url,
                body_sha256: row.sha.clone(),
                validators: row.validators.clone(),
            })
        });
        let attempt = self.session.begin_with_conditional(source, conditional)?;
        let request = attempt.dispatch()?;
        let binding = Binding {
            source: source.into(),
            configuration: self.configuration_token.clone(),
            provider: self.provider_token.clone(),
            activation,
            request: request.request_id.clone(),
            cache: retained.as_ref().map(|r| r.cache_token.clone()),
        };
        let registered = (|| {
            let tx = self
                .db
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            check_owner(&tx, &self.owner, &binding.configuration, &binding.provider)?;
            let active:Option<String>=tx.query_row("SELECT CASE WHEN activation IS NULL THEN NULL WHEN typeof(activation)='text' AND length(CAST(activation AS BLOB))=36 THEN activation ELSE zeroblob(1) END FROM sources WHERE source_id=?1",[source],|r|r.get(0))?;
            if active.as_deref() != Some(&binding.activation) {
                return Err(fail("Stale activation before dispatch"));
            }
            tx.execute(
                "UPDATE sources SET latest_request=?1,last_error=NULL WHERE source_id=?2",
                params![binding.request, source],
            )?;
            check_cancel(cancel)?;
            if tx.commit().is_err() {
                self.poisoned = true;
                let _ = self.session.close();
                return Err(fail(
                    "Lifecycle COMMIT outcome uncertain; controller closed, reopen and inspect",
                ));
            }
            Ok(())
        })();
        if let Err(error) = registered {
            attempt.cancel();
            return Err(error);
        }
        Ok(Start {
            attempt,
            request,
            provider: self.provider.clone(),
            binding,
            retained,
        })
    }
    fn status(&self, source: &str) -> Result<FeedCacheStatus> {
        self.fence()?;
        self.source(source)?;
        let (active,error):(Option<String>,Option<String>)=self.db.query_row("SELECT CASE WHEN activation IS NULL THEN NULL WHEN typeof(activation)='text' AND length(CAST(activation AS BLOB))=36 THEN activation ELSE zeroblob(1) END,CASE WHEN last_error IS NULL THEN NULL WHEN typeof(last_error)='text' AND length(CAST(last_error AS BLOB))<=128 THEN last_error ELSE zeroblob(1) END FROM sources WHERE source_id=?1",[source],|r|Ok((r.get(0)?,r.get(1)?)))?;
        let row = self.load(source);
        let corrupt = row.is_err();
        let row = row.ok().flatten();
        let invalid = row
            .as_ref()
            .is_some_and(|r| self.check_row(source, r).is_err());
        let error = if corrupt || invalid {
            Some("Retained feed is unavailable or corrupt".into())
        } else {
            error
        };
        let row = if invalid { None } else { row };
        Ok(FeedCacheStatus {
            source_id: source.into(),
            locally_active: active.is_some(),
            cached: row.is_some() && !invalid,
            last_check_unix_seconds: row.as_ref().map(|r| r.checked),
            last_body_success_unix_seconds: row.as_ref().map(|r| r.body_success),
            stale: active.is_none() || error.is_some(),
            last_error: error,
        })
    }
    fn admit(
        &mut self,
        binding: Binding,
        read: f::FeedSourceRead,
        feed: x::NormalizedFeed,
        context: Vec<u8>,
        raw: Vec<u8>,
        cancel: &AtomicBool,
    ) -> Result<FeedCacheView> {
        self.fence()?;
        let condition = read.cache_condition()?;
        let checked = now()?;
        let outcome = condition.outcome;
        let cache_token = token();
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        check_binding(&tx, &self.owner, &binding)?;
        let previous = load_row(&tx, &binding.source)?;
        let actual = if outcome == f::FeedSourceOutcome::NotModified {
            let previous = previous
                .as_ref()
                .ok_or_else(|| fail("304 retained body is absent or evicted"))?;
            // These are actual transaction bytes, not the pre-await retained snapshot.
            if previous.cache_token != binding.cache.clone().unwrap_or_default()
                || previous.context != context
                || previous.profile != x::NORMALIZER_PROFILE
                || previous.context_sha != booknook_core::digest(&context)
                || previous.sha != booknook_core::digest(&previous.raw)
                || previous.raw != raw
                || condition.body_sha256 != previous.sha
            {
                return Err(fail("304 actual retained context/body/revision changed"));
            }
            Some(previous.raw.as_slice())
        } else {
            None
        };
        if previous
            .as_ref()
            .is_some_and(|row| row.checked > checked || row.body_success > checked)
        {
            return Err(fail(
                "Local clock moved backwards; prior feed cache retained",
            ));
        }
        let row = Row {
            raw,
            sha: condition.body_sha256.clone(),
            context_sha: booknook_core::digest(&context),
            context,
            profile: x::NORMALIZER_PROFILE.into(),
            cache_token: cache_token.clone(),
            request: binding.request.clone(),
            validators: condition.validators.clone(),
            checked,
            body_success: if outcome == f::FeedSourceOutcome::NotModified {
                previous
                    .as_ref()
                    .ok_or_else(|| fail("Missing 304 row"))?
                    .body_success
            } else {
                checked
            },
        };
        let logical = row.logical()?;
        if logical > self.quota {
            return Err(fail("Candidate exceeds logical feed cache quota"));
        }
        let result_view = view(feed, &row, &binding.configuration, false)?;
        if cancel.load(Ordering::SeqCst) {
            read.cancellation().cancel();
            return Err(fail("Cancelled before durable admission"));
        }
        let owner = &self.owner;
        let quota = self.quota;
        let max_rows = self.max_rows;
        #[cfg(test)]
        let fault = self.faults;
        let result=read.with_current_cache_commit(actual,|_| {
        let result=(|| ->Result<()> {
            check_binding(&tx,owner,&binding)?;
            let count:i64=tx.query_row("SELECT counter FROM controller WHERE id=1",[],|r|r.get(0))?;
            let next=count.checked_add(1).filter(|n|*n>0).ok_or_else(||fail("Feed LRU counter exhausted"))?;
            validate_accounting(&tx)?;
            tx.execute("UPDATE controller SET counter=?1 WHERE id=1",[next])?;
            tx.execute("INSERT OR REPLACE INTO cache VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",params![binding.source,row.raw,row.sha,row.context,row.context_sha,row.profile,row.cache_token,row.request,serde_json::to_vec(&row.validators)?,row.checked,row.body_success,next,i64::try_from(logical).map_err(|_|fail("Quota integer overflow"))?])?;
            tx.execute("INSERT OR REPLACE INTO receipts VALUES(?1,?2,?3,?4,?5,?6)",params![binding.source,row.request,row.cache_token,row.context_sha,row.profile,row.sha])?;
            tx.execute("UPDATE sources SET last_error=NULL WHERE source_id=?1",[&binding.source])?;
            loop {
                let (bytes,rows):(i64,i64)=tx.query_row("SELECT COALESCE(SUM(length(raw)+length(context)+length(validators)+length(CAST(sha AS BLOB))+length(CAST(context_sha AS BLOB))+length(CAST(profile AS BLOB))+length(CAST(cache_token AS BLOB))+length(CAST(request_id AS BLOB))+1024),0),COUNT(*) FROM cache",[],|r|Ok((r.get(0)?,r.get(1)?)))?;
                if bytes>=0 && u64::try_from(bytes).is_ok_and(|n|n<=quota) && usize::try_from(rows).is_ok_and(|n|n<=max_rows){break;}
                let evict:Option<String>=tx.query_row("SELECT CASE WHEN typeof(source_id)='text' AND length(CAST(source_id AS BLOB))=36 THEN source_id ELSE NULL END FROM cache WHERE source_id<>?1 ORDER BY lru,source_id LIMIT 1",[&binding.source],|r|r.get(0)).optional()?;
                let evict=evict.ok_or_else(||fail("Cannot satisfy logical feed quota"))?;
                valid_token(&evict)?;
                tx.execute("DELETE FROM cache WHERE source_id=?1",[&evict])?;tx.execute("DELETE FROM receipts WHERE source_id=?1",[evict])?;
            }
            #[cfg(test)] if fault.before_commit {return Err(fail("Injected pre-commit failure"));}
            tx.commit()?;
            #[cfg(test)] if fault.after_commit {return Err(fail("Injected uncertain COMMIT acknowledgement"));}
            Ok(())
        })();
        result.map_err(core_error)
    });
        if let Err(error) = result {
            #[cfg(test)]
            if self.faults.corrupt_after_commit {
                self.db.execute(
                    "UPDATE cache SET profile='changed.after.commit' WHERE source_id=?1",
                    [&binding.source],
                )?;
            }
            // Do not replay the transaction. Reconcile only an EXACT receipt and row
            // while this owner/config/provider/request remains current. An uncertain
            // or changed authority poisons this controller; caller must reopen.
            match self.reconcile(&binding, &row) {
                Ok(true) => return Ok(result_view),
                Ok(false) => return Err(error.into()),
                Err(_) => {
                    self.poisoned = true;
                    let _ = self.session.close();
                    return Err(fail(
                        "Feed COMMIT outcome is uncertain; controller closed, reopen and inspect",
                    ));
                }
            }
        }
        // PASSIVE never waits for readers; physical WAL size is not the logical
        // quota. A pinned external reader can retain WAL pages. Maintenance is
        // outside core guard/transaction and never changes an accepted outcome.
        let _ = self.db.execute_batch("PRAGMA wal_checkpoint(PASSIVE)");
        Ok(result_view)
    }
    fn reconcile(&self, binding: &Binding, row: &Row) -> Result<bool> {
        #[cfg(test)]
        if self.faults.reconcile_unavailable {
            return Err(fail("Injected reconciliation unavailable"));
        }
        let fresh = Connection::open_with_flags(
            &self.path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        fresh.busy_timeout(std::time::Duration::from_millis(250))?;
        let fresh = fresh.unchecked_transaction()?;
        check_request(&fresh, &self.owner, binding)?;
        let receipt:Option<(String,String,String,String,String)>=fresh.query_row("SELECT CASE WHEN typeof(request_id)='text' AND length(CAST(request_id AS BLOB))=32 THEN request_id ELSE NULL END,CASE WHEN typeof(cache_token)='text' AND length(CAST(cache_token AS BLOB))=36 THEN cache_token ELSE NULL END,CASE WHEN typeof(context_sha)='text' AND length(CAST(context_sha AS BLOB))=64 THEN context_sha ELSE NULL END,CASE WHEN typeof(profile)='text' AND length(CAST(profile AS BLOB))<=128 THEN profile ELSE NULL END,CASE WHEN typeof(raw_sha)='text' AND length(CAST(raw_sha AS BLOB))=64 THEN raw_sha ELSE NULL END FROM receipts WHERE source_id=?1",[&binding.source],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        if let Some((request, cache, context, profile, sha)) = receipt
            && request == row.request
            && cache == row.cache_token
            && context == row.context_sha
            && profile == row.profile
            && sha == row.sha
        {
            let actual = load_row(&fresh, &binding.source)?
                .ok_or_else(|| fail("Receipt without actual retained row"))?;
            if actual == *row && booknook_core::digest(&actual.raw) == row.sha {
                return Ok(true);
            }
            return Err(fail("Receipt and retained row disagree"));
        }
        check_binding(&fresh, &self.owner, binding)?;
        Ok(false)
    }
}
fn valid_token(value: &str) -> Result<()> {
    if value.len() != 36
        || !value.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
    {
        Err(fail("Corrupt feed cache token"))
    } else {
        Ok(())
    }
}
fn valid_request(value: &str) -> Result<()> {
    if value.len() != 32
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Err(fail("Invalid retained core request ID"))
    } else {
        Ok(())
    }
}
fn check_owner(db: &Connection, owner: &str, configuration: &str, provider: &str) -> Result<()> {
    let (actual_owner,actual_configuration,actual_provider,schema,raw,descriptor):(String,String,String,i64,Vec<u8>,Vec<u8>)=db.query_row("SELECT CASE WHEN typeof(owner)='text' AND length(CAST(owner AS BLOB))=36 THEN owner ELSE NULL END,CASE WHEN typeof(configuration_token)='text' AND length(CAST(configuration_token AS BLOB))=101 THEN configuration_token ELSE NULL END,CASE WHEN typeof(provider_token)='text' AND length(CAST(provider_token AS BLOB))=101 THEN provider_token ELSE NULL END,schema_version,CASE WHEN typeof(configuration)='blob' AND length(configuration)<=266240 THEN configuration ELSE NULL END,CASE WHEN typeof(provider)='blob' AND length(provider)<=16384 THEN provider ELSE NULL END FROM controller WHERE id=1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
    if actual_owner != owner
        || actual_configuration != configuration
        || actual_provider != provider
        || schema != 1
    {
        return Err(fail(
            "Stale durable feed owner/configuration/provider fence",
        ));
    }
    check_bound_token(&actual_configuration, &raw)?;
    check_bound_token(&actual_provider, &descriptor)?;
    Ok(())
}
fn check_request(db: &Connection, owner: &str, binding: &Binding) -> Result<()> {
    check_owner(db, owner, &binding.configuration, &binding.provider)?;
    let (activation,request):(Option<String>,Option<String>)=db.query_row("SELECT CASE WHEN activation IS NULL THEN NULL WHEN typeof(activation)='text' AND length(CAST(activation AS BLOB))=36 THEN activation ELSE zeroblob(1) END,CASE WHEN latest_request IS NULL THEN NULL WHEN typeof(latest_request)='text' AND length(CAST(latest_request AS BLOB))=32 THEN latest_request ELSE zeroblob(1) END FROM sources WHERE source_id=?1",[&binding.source],|r|Ok((r.get(0)?,r.get(1)?)))?;
    if activation.as_deref() != Some(&binding.activation)
        || request.as_deref() != Some(&binding.request)
    {
        return Err(fail("Stale durable activation or source request"));
    }
    Ok(())
}
fn check_binding(db: &Connection, owner: &str, binding: &Binding) -> Result<()> {
    check_request(db, owner, binding)?;
    let cache:Option<String>=db.query_row("SELECT CASE WHEN typeof(cache_token)='text' AND length(CAST(cache_token AS BLOB))=36 THEN cache_token ELSE NULL END FROM cache WHERE source_id=?1",[&binding.source],|r|r.get(0)).optional()?;
    if cache != binding.cache {
        return Err(fail("Stale durable retained cache token"));
    }
    Ok(())
}
fn validate_accounting(db: &Connection) -> Result<()> {
    let count: i64 = db.query_row("SELECT COUNT(*) FROM cache", [], |r| r.get(0))?;
    if !(0..=FEED_CACHE_MAX_ROWS as i64).contains(&count) {
        return Err(fail("Corrupt bounded cache row count"));
    }
    let bad:i64=db.query_row("SELECT COUNT(*) FROM cache WHERE typeof(source_id)<>'text' OR length(CAST(source_id AS BLOB))<>36 OR typeof(raw)<>'blob' OR length(raw) NOT BETWEEN 1 AND 2097152 OR typeof(context)<>'blob' OR length(context) NOT BETWEEN 1 AND 16384 OR typeof(validators)<>'blob' OR length(validators)>2048 OR typeof(sha)<>'text' OR length(CAST(sha AS BLOB))<>64 OR typeof(context_sha)<>'text' OR length(CAST(context_sha AS BLOB))<>64 OR typeof(profile)<>'text' OR length(CAST(profile AS BLOB))>128 OR typeof(cache_token)<>'text' OR length(CAST(cache_token AS BLOB))<>36 OR typeof(request_id)<>'text' OR length(CAST(request_id AS BLOB))<>32 OR typeof(logical)<>'integer' OR logical<0 OR logical<>length(raw)+length(context)+length(validators)+length(CAST(sha AS BLOB))+length(CAST(context_sha AS BLOB))+length(CAST(profile AS BLOB))+length(CAST(cache_token AS BLOB))+length(CAST(request_id AS BLOB))+1024 OR typeof(lru)<>'integer' OR lru<0",[],|r|r.get(0))?;
    if bad != 0 {
        return Err(fail("Corrupt bounded feed quota accounting"));
    }
    let mut ids = db.prepare("SELECT source_id FROM cache LIMIT 129")?;
    for id in ids.query_map([], |r| r.get::<_, String>(0))? {
        valid_token(&id?)?;
    }
    Ok(())
}
fn load_row(db: &Connection, source: &str) -> Result<Option<Row>> {
    // CASE bounds BEFORE allocation, including corrupted databases. Other metadata
    // uses fixed small columns; no unbounded string/blob decode from a bad row.
    Ok(db.query_row("SELECT CASE WHEN typeof(raw)='blob' AND length(raw)>0 AND length(raw)<=2097152 THEN raw ELSE NULL END,CASE WHEN typeof(sha)='text' AND length(CAST(sha AS BLOB))=64 THEN sha ELSE NULL END,CASE WHEN typeof(context)='blob' AND length(context)>0 AND length(context)<=16384 THEN context ELSE NULL END,CASE WHEN typeof(context_sha)='text' AND length(CAST(context_sha AS BLOB))=64 THEN context_sha ELSE NULL END,CASE WHEN typeof(profile)='text' AND length(CAST(profile AS BLOB))<=128 THEN profile ELSE NULL END,CASE WHEN typeof(cache_token)='text' AND length(CAST(cache_token AS BLOB))=36 THEN cache_token ELSE NULL END,CASE WHEN typeof(request_id)='text' AND length(CAST(request_id AS BLOB))=32 THEN request_id ELSE NULL END,CASE WHEN typeof(validators)='blob' AND length(validators)<=2048 THEN validators ELSE NULL END,checked,body_success FROM cache WHERE source_id=?1",[source],|r| {
    let validators:Vec<u8>=r.get(7)?;
    let validators=booknook_core::strict_parse(&validators).map_err(|e|rusqlite::Error::FromSqlConversionFailure(7,rusqlite::types::Type::Blob,Box::new(e)))?;
    Ok(Row{raw:r.get(0)?,sha:r.get(1)?,context:r.get(2)?,context_sha:r.get(3)?,profile:r.get(4)?,cache_token:r.get(5)?,request:r.get(6)?,validators,checked:r.get(8)?,body_success:r.get(9)?})
 }).optional()?)
}
fn view(
    feed: x::NormalizedFeed,
    row: &Row,
    configuration: &str,
    offline: bool,
) -> Result<FeedCacheView> {
    let source = feed.provenance.source_id.clone();
    let mut entries = Vec::with_capacity(feed.entries.len());
    for entry in feed.entries {
        let mut content = serde_json::to_value(&entry)?;
        let fields = content
            .as_object_mut()
            .ok_or_else(|| fail("Invalid produced entry"))?;
        fields.remove("first_source_ordinal");
        fields.remove("duplicate_count");
        fields.remove("conflicting_identity");
        let key = booknook_core::digest(&booknook_core::canonical(
            &serde_json::json!({"schema":"booknook.feed-cache-entry-key.v1","profile":x::NORMALIZER_PROFILE,"source_id":source,"content":content}),
        )?);
        entries.push(FeedCacheEntry {
            variant_key: key,
            entry,
        });
    }
    // Published and updated remain independent. No local check/fetch timestamp is
    // ever a sort key. Parsed publication dates sort lexicographically (seconds,
    // nanos, including Chrono leap-second representation); undated ties keep source
    // order. The variant key remains independent of that order.
    entries.sort_by(|a, b| {
        let a_date = a
            .entry
            .published
            .unix_seconds
            .zip(a.entry.published.nanoseconds);
        let b_date = b
            .entry
            .published
            .unix_seconds
            .zip(b.entry.published.nanoseconds);
        b_date
            .cmp(&a_date)
            .then_with(|| {
                a.entry
                    .first_source_ordinal
                    .cmp(&b.entry.first_source_ordinal)
            })
            .then_with(|| a.variant_key.cmp(&b.variant_key))
    });
    Ok(FeedCacheView {
        schema: "booknook.native-feed-cache-view.v1".into(),
        source_id: source,
        configuration_token: configuration.into(),
        cache_token: row.cache_token.clone(),
        context_sha256: row.context_sha.clone(),
        raw_sha256: row.sha.clone(),
        normalizer_profile: feed.normalizer_profile,
        provenance: feed.provenance,
        title: feed.title,
        entries,
        last_check_unix_seconds: row.checked,
        last_body_success_unix_seconds: row.body_success,
        offline,
    })
}
#[cfg(test)]
#[derive(Default, Clone, Copy)]
struct Faults {
    before_commit: bool,
    after_commit: bool,
    reconcile_unavailable: bool,
    corrupt_after_commit: bool,
    before_lifecycle_commit: bool,
}

#[cfg(test)]
#[path = "feed_cache_tests.rs"]
mod tests;
