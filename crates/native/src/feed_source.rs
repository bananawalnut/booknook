// SPDX-License-Identifier: MIT OR Apache-2.0
//! Independent installed-source callbacks and explicit native public HTTPS factory.
//! No Desk or custody authority. Async staging stays separate from guarded cache CAS.
use crate::{NativeError, Result};
use booknook_core::{Error, feed_source as f, feeds::FeedConfiguration};
use std::sync::{Arc, Mutex, OnceLock};
fn encode(value: &impl serde::Serialize) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}
fn foreign(error: NativeError) -> Error {
    Error::Unavailable(error.to_string())
}
#[uniffi::export]
pub fn validate_feed_source_descriptor_v1(raw: Vec<u8>) -> Result<String> {
    encode(&f::FeedSourceDescriptor::parse(&raw)?)
}
#[derive(uniffi::Object)]
pub struct FeedSourceCancellationV1 {
    inner: f::FeedCancellation,
}
#[uniffi::export]
impl FeedSourceCancellationV1 {
    pub fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }
    /// False when final synchronous commit has already started; no rollback.
    pub fn cancel(&self) -> bool {
        self.inner.cancel()
    }
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct FeedSourceReplyV1 {
    pub response_bytes: Vec<u8>,
    pub body: Vec<u8>,
}
#[uniffi::export(callback_interface)]
#[async_trait::async_trait]
pub trait FeedSourceHostV1: Send + Sync {
    fn descriptor_json(&self) -> Result<String>;
    async fn fetch(
        &self,
        request_json: String,
        cancellation: Arc<FeedSourceCancellationV1>,
    ) -> Result<FeedSourceReplyV1>;
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct FeedCacheStageV1 {
    pub reference: String,
    pub existing_body: Option<Vec<u8>>,
}
#[uniffi::export(callback_interface)]
#[async_trait::async_trait]
pub trait FeedCacheHostV1: Send + Sync {
    async fn stage(
        &self,
        condition_json: String,
        body: Vec<u8>,
        cancellation: Arc<FeedSourceCancellationV1>,
    ) -> Result<FeedCacheStageV1>;
    /// Must atomically compare the expected generation/source key and finish its
    /// durable transaction before returning. Do not reenter the source session.
    fn compare_and_store(
        &self,
        staged: String,
        condition_json: String,
        body: Vec<u8>,
    ) -> Result<()>;
}

// UniFFI futures are polled by the foreign executor, which has no Tokio reactor.
// Lazily own one bounded executor for explicitly requested native HTTP work;
// neither constructing this host nor constructing a source session starts it.
fn public_feed_runtime() -> Result<&'static tokio::runtime::Runtime> {
    static RUNTIME: OnceLock<std::result::Result<tokio::runtime::Runtime, std::io::Error>> =
        OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .max_blocking_threads(4)
                .thread_name("booknook-public-feed")
                .enable_all()
                .build()
        })
        .as_ref()
        .map_err(|_| NativeError::Failure {
            message: "Cannot start the native public feed executor".into(),
        })
}

struct PublicFeedTask {
    handle: tokio::task::JoinHandle<booknook_core::Result<f::FeedSourceReply>>,
    cancellation: f::FeedCancellation,
    complete: bool,
}
impl PublicFeedTask {
    async fn finish(mut self) -> booknook_core::Result<f::FeedSourceReply> {
        // JoinHandle can be polled by a foreign executor; only its spawned work
        // needs the Tokio reactor. No block_on or caller-thread network work.
        let result = (&mut self.handle).await;
        self.complete = true;
        result.map_err(|_| Error::Unavailable("Native public feed worker stopped".into()))?
    }
}
impl Drop for PublicFeedTask {
    fn drop(&mut self) {
        if !self.complete {
            self.cancellation.cancel();
            self.handle.abort();
        }
    }
}

struct PublicHttpsHost;
#[async_trait::async_trait]
impl FeedSourceHostV1 for PublicHttpsHost {
    fn descriptor_json(&self) -> Result<String> {
        let provider = booknook_connectors::public_https_feed::PublicHttpsFeedProvider::new();
        encode(&f::FeedSourceProvider::descriptor(&provider)?)
    }
    async fn fetch(
        &self,
        request_json: String,
        cancellation: Arc<FeedSourceCancellationV1>,
    ) -> Result<FeedSourceReplyV1> {
        if request_json.len() > f::MAX_SOURCE_METADATA_BYTES {
            return Err(Error::Invalid("Oversized native feed source request".into()).into());
        }
        let request: f::FeedSourceRequest = booknook_core::strict_parse(request_json.as_bytes())?;
        let cancellation = cancellation.inner.clone();
        let worker_cancellation = cancellation.clone();
        let handle = public_feed_runtime()?.spawn(async move {
            let provider = booknook_connectors::public_https_feed::PublicHttpsFeedProvider::new();
            f::FeedSourceProvider::fetch(&provider, request, worker_cancellation).await
        });
        let reply = PublicFeedTask {
            handle,
            cancellation,
            complete: false,
        }
        .finish()
        .await?;
        Ok(FeedSourceReplyV1 {
            response_bytes: reply.response_bytes,
            body: reply.body,
        })
    }
}
struct Installed {
    host: Arc<dyn FeedSourceHostV1>,
    descriptor: f::FeedSourceDescriptor,
}
impl Installed {
    fn new(host: Box<dyn FeedSourceHostV1>) -> Result<Self> {
        let descriptor = f::FeedSourceDescriptor::parse(host.descriptor_json()?.as_bytes())?;
        Ok(Self {
            host: Arc::from(host),
            descriptor,
        })
    }
}
impl f::FeedSourceProvider for Installed {
    fn descriptor(&self) -> booknook_core::Result<f::FeedSourceDescriptor> {
        Ok(self.descriptor.clone())
    }
    fn fetch<'a>(
        &'a self,
        request: f::FeedSourceRequest,
        cancellation: f::FeedCancellation,
    ) -> f::FeedFuture<'a, f::FeedSourceReply> {
        Box::pin(async move {
            let reply = self
                .host
                .fetch(
                    serde_json::to_string(&request)?,
                    Arc::new(FeedSourceCancellationV1 {
                        inner: cancellation,
                    }),
                )
                .await
                .map_err(foreign)?;
            Ok(f::FeedSourceReply {
                response_bytes: reply.response_bytes,
                body: reply.body,
            })
        })
    }
}
#[derive(uniffi::Object)]
pub struct FeedSourceSessionV1 {
    inner: f::FeedSourceSession,
    installed: Mutex<Arc<Installed>>,
}
#[uniffi::export]
impl FeedSourceSessionV1 {
    /// Explicit concrete native transport; construction is offline and inactive.
    /// Fetch still requires local activation and returns untrusted XML bytes.
    #[uniffi::constructor]
    pub fn public_https(configuration_bytes: Vec<u8>) -> Result<Self> {
        Self::new(configuration_bytes, Box::new(PublicHttpsHost))
    }

    #[uniffi::constructor]
    pub fn new(configuration_bytes: Vec<u8>, host: Box<dyn FeedSourceHostV1>) -> Result<Self> {
        let configuration = FeedConfiguration::parse(&configuration_bytes)?;
        let installed = Arc::new(Installed::new(host)?);
        Ok(Self {
            inner: f::FeedSourceSession::new(configuration, vec![installed.descriptor.clone()])?,
            installed: Mutex::new(installed),
        })
    }
    pub fn close(&self) -> Result<()> {
        Ok(self.inner.close()?)
    }
    pub fn replace_configuration(&self, configuration_bytes: Vec<u8>) -> Result<()> {
        Ok(self
            .inner
            .replace_configuration(FeedConfiguration::parse(&configuration_bytes)?)?)
    }
    pub fn replace_provider(&self, host: Box<dyn FeedSourceHostV1>) -> Result<()> {
        let replacement = Arc::new(Installed::new(host)?);
        let mut installed = self
            .installed
            .try_lock()
            .map_err(|_| Error::Conflict("Feed provider registration is busy".into()))?;
        self.inner
            .replace_providers(vec![replacement.descriptor.clone()])?;
        *installed = replacement;
        Ok(())
    }
    pub fn activate(&self, source_id: String) -> Result<()> {
        let installed = self
            .installed
            .try_lock()
            .map_err(|_| Error::Conflict("Feed provider registration is busy".into()))?;
        Ok(self.inner.activate(&source_id, &installed.descriptor.id)?)
    }
    pub fn disable(&self, source_id: String) -> Result<()> {
        Ok(self.inner.disable(&source_id)?)
    }
    pub fn begin(
        &self,
        source_id: String,
        conditional_bytes: Option<Vec<u8>>,
    ) -> Result<Arc<FeedSourceAttemptV1>> {
        let hint = conditional_bytes
            .as_deref()
            .map(f::FeedConditionalHint::parse)
            .transpose()?;
        let installed = self
            .installed
            .try_lock()
            .map_err(|_| Error::Conflict("Feed provider registration is busy".into()))?;
        Ok(Arc::new(FeedSourceAttemptV1 {
            inner: self.inner.begin_with_conditional(&source_id, hint)?,
            installed: installed.clone(),
        }))
    }
}

#[cfg(test)]
mod public_https_factory_tests {
    use super::*;
    use std::{
        future::Future,
        sync::atomic::{AtomicBool, Ordering},
        task::{Context, Poll, Waker},
        time::{Duration, Instant},
    };

    fn cancellation() -> f::FeedCancellation {
        let section = booknook_core::feeds::FeedSection::parse(include_bytes!(
            "../tests/fixtures/feeds-v1/section.json"
        ))
        .unwrap();
        let source = section.subscriptions[0].id.clone();
        let provider = booknook_connectors::public_https_feed::PublicHttpsFeedProvider::new();
        let descriptor = f::FeedSourceProvider::descriptor(&provider).unwrap();
        let id = descriptor.id.clone();
        let session = f::FeedSourceSession::new(
            FeedConfiguration::new(0, section).unwrap(),
            vec![descriptor],
        )
        .unwrap();
        session.activate(&source, &id).unwrap();
        session.begin(&source).unwrap().cancellation()
    }
    fn wait<F: Future>(future: F) -> F::Output {
        assert!(tokio::runtime::Handle::try_current().is_err());
        let mut future = Box::pin(future);
        let end = Instant::now() + Duration::from_secs(2);
        loop {
            if let Poll::Ready(value) = future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
            {
                return value;
            }
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn concrete_public_https_foreign_future_drop_cancels_and_aborts_owned_work() {
        let cancellation = cancellation();
        let started = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicBool::new(false));
        struct Dropped(Arc<AtomicBool>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let worker_started = started.clone();
        let worker_dropped = dropped.clone();
        let handle = public_feed_runtime().unwrap().spawn(async move {
            let _drop = Dropped(worker_dropped);
            worker_started.store(true, Ordering::SeqCst);
            std::future::pending::<booknook_core::Result<f::FeedSourceReply>>().await
        });
        let mut future = Box::pin(
            PublicFeedTask {
                handle,
                cancellation: cancellation.clone(),
                complete: false,
            }
            .finish(),
        );
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        let end = Instant::now() + Duration::from_secs(2);
        while !started.load(Ordering::SeqCst) {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(1));
        }
        drop(future);
        assert!(cancellation.is_cancelled());
        while !dropped.load(Ordering::SeqCst) {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn concrete_public_https_completed_worker_does_not_cancel_successful_read() {
        let cancellation = cancellation();
        let handle = public_feed_runtime().unwrap().spawn(async {
            Ok(f::FeedSourceReply {
                response_bytes: vec![],
                body: b"raw fixture".to_vec(),
            })
        });
        let result = wait(
            PublicFeedTask {
                handle,
                cancellation: cancellation.clone(),
                complete: false,
            }
            .finish(),
        )
        .unwrap();
        assert_eq!(result.body, b"raw fixture");
        assert!(!cancellation.is_cancelled());
    }
}
#[derive(uniffi::Object)]
pub struct FeedSourceAttemptV1 {
    inner: f::FeedSourceAttempt,
    installed: Arc<Installed>,
}
#[uniffi::export]
impl FeedSourceAttemptV1 {
    pub fn cancel(&self) -> bool {
        self.inner.cancel()
    }
    pub async fn execute(&self) -> Result<Arc<FeedSourceReadV1>> {
        Ok(Arc::new(FeedSourceReadV1 {
            inner: self.inner.execute(self.installed.as_ref()).await?,
        }))
    }
}
#[derive(uniffi::Object)]
pub struct FeedSourceReadV1 {
    inner: f::FeedSourceRead,
}
struct Cache {
    host: Box<dyn FeedCacheHostV1>,
}
impl f::FeedCacheStore for Cache {
    fn compare_and_store<'a>(&'a self, candidate: &'a f::FeedSourceRead) -> f::FeedFuture<'a, ()> {
        Box::pin(async move {
            let condition = candidate.cache_condition()?;
            let cancellation = Arc::new(FeedSourceCancellationV1 {
                inner: candidate.cancellation(),
            });
            let staged = self
                .host
                .stage(
                    serde_json::to_string(&condition)?,
                    candidate.body().to_vec(),
                    cancellation,
                )
                .await
                .map_err(foreign)?;
            if staged.reference.is_empty()
                || staged.reference.len() > 512
                || staged.reference.chars().any(char::is_control)
            {
                return Err(Error::Invalid("Invalid cache staging reference".into()));
            }
            candidate.with_current_cache_commit(staged.existing_body.as_deref(), |condition| {
                self.host
                    .compare_and_store(
                        staged.reference,
                        serde_json::to_string(condition)?,
                        candidate.body().to_vec(),
                    )
                    .map_err(foreign)
            })
        })
    }
}
#[uniffi::export]
impl FeedSourceReadV1 {
    pub fn request(&self) -> Result<String> {
        encode(self.inner.request())
    }
    pub fn response(&self) -> Result<String> {
        encode(self.inner.response())
    }
    /// Untrusted bytes, not parsed or safe to render.
    pub fn body(&self) -> Vec<u8> {
        self.inner.body().to_vec()
    }
    pub fn cache_condition(&self) -> Result<String> {
        encode(&self.inner.cache_condition()?)
    }
    pub fn cancel(&self) -> bool {
        self.inner.cancellation().cancel()
    }
    pub async fn store(&self, host: Box<dyn FeedCacheHostV1>) -> Result<()> {
        Ok(self.inner.store(&Cache { host }).await?)
    }
}
