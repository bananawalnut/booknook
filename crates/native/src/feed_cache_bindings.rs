// SPDX-License-Identifier: MIT OR Apache-2.0
//! Additive foreign facade over the coordinated native cache, not a cache permit.
use crate::feed_cache::OperationCancellation;
use crate::{FeedCacheController, FeedSourceHostV1, NativeError, Result};
use std::{future::Future, path::Path, sync::Arc};

const MAX_VIEW_BYTES: usize = 5 * 1024 * 1024;
fn payload(value: &impl serde::Serialize, limit: usize) -> Result<Vec<u8>> {
    let raw = serde_json::to_vec(value)?;
    if raw.len() > limit {
        return Err(NativeError::Failure {
            message: "Produced feed cache payload exceeds binding limit".into(),
        });
    }
    Ok(raw)
}
/// One-use cancellation only, never permission to fetch or admit an external read.
/// cancel fences registered storage/core work synchronously before waking await.
#[derive(uniffi::Object)]
pub struct FeedCacheOperationV1 {
    scope: Arc<OperationCancellation>,
}
#[uniffi::export]
impl FeedCacheOperationV1 {
    #[uniffi::constructor]
    pub fn new() -> Self {
        Self {
            scope: Arc::new(OperationCancellation::default()),
        }
    }
    /// A request, not a rollback promise: a synchronous commit may have started.
    pub fn cancel(&self) {
        self.scope.cancel();
    }
}
impl Default for FeedCacheOperationV1 {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(uniffi::Record)]
pub struct FeedCacheConfigurationV1 {
    pub raw: Vec<u8>,
    pub configuration_token: String,
    pub provider_token: String,
    pub provider_descriptor_json: String,
}
/// Share this same opaque coordinator across windows. Constructors perform local
/// blocking filesystem work: foreign callers MUST construct off the UI thread.
#[derive(uniffi::Object)]
pub struct FeedCacheControllerV1 {
    inner: FeedCacheController,
}
impl FeedCacheControllerV1 {
    async fn execute<T, F: Future<Output = Result<T>>>(
        &self,
        operation: Arc<FeedCacheOperationV1>,
        action: impl FnOnce(FeedCacheController) -> F,
    ) -> Result<T> {
        operation.scope.claim()?;
        let controller = self.inner.scoped(operation.scope.clone());
        operation.scope.run(action(controller)).await
    }
}
#[uniffi::export]
impl FeedCacheControllerV1 {
    #[uniffi::constructor]
    pub fn new(
        directory: String,
        expected_configuration: Option<Vec<u8>>,
        host: Box<dyn FeedSourceHostV1>,
    ) -> Result<Self> {
        Ok(Self {
            inner: FeedCacheController::open(Path::new(&directory), expected_configuration, host)?,
        })
    }
    /// Offline construction; no activation and no caller Tokio reactor required.
    #[uniffi::constructor]
    pub fn public_https(
        directory: String,
        expected_configuration: Option<Vec<u8>>,
    ) -> Result<Self> {
        Ok(Self {
            inner: FeedCacheController::public_https(
                Path::new(&directory),
                expected_configuration,
            )?,
        })
    }
    pub async fn configuration(
        &self,
        operation: Arc<FeedCacheOperationV1>,
    ) -> Result<FeedCacheConfigurationV1> {
        self.execute(operation, |controller| async move {
            let value = controller.configuration().await?;
            Ok(FeedCacheConfigurationV1 {
                raw: value.raw,
                configuration_token: value.configuration_token,
                provider_token: value.provider_token,
                provider_descriptor_json: serde_json::to_string(&value.provider_descriptor)?,
            })
        })
        .await
    }
    pub async fn replace_configuration(
        &self,
        expected_token: String,
        raw: Vec<u8>,
        operation: Arc<FeedCacheOperationV1>,
    ) -> Result<()> {
        self.execute(operation, |controller| async move {
            controller.replace_configuration(&expected_token, raw).await
        })
        .await
    }
    pub async fn replace_provider(
        &self,
        expected_token: String,
        host: Box<dyn FeedSourceHostV1>,
        operation: Arc<FeedCacheOperationV1>,
    ) -> Result<()> {
        self.execute(operation, |controller| async move {
            controller.replace_provider(&expected_token, host).await
        })
        .await
    }
    pub async fn activate(
        &self,
        source_id: String,
        operation: Arc<FeedCacheOperationV1>,
    ) -> Result<()> {
        self.execute(operation, |controller| async move {
            controller.activate(&source_id).await
        })
        .await
    }
    pub async fn pause(
        &self,
        source_id: String,
        operation: Arc<FeedCacheOperationV1>,
    ) -> Result<()> {
        self.execute(operation, |controller| async move {
            controller.disable(&source_id).await
        })
        .await
    }
    /// Produced-only inert display data, bounded to 5 MiB. Not an import format.
    pub async fn refresh(
        &self,
        source_id: String,
        operation: Arc<FeedCacheOperationV1>,
    ) -> Result<Vec<u8>> {
        self.execute(operation, |controller| async move {
            payload(&controller.refresh(&source_id).await?, MAX_VIEW_BYTES)
        })
        .await
    }
    /// Re-normalizes original retained XML; never decodes normalized cache JSON.
    pub async fn offline(
        &self,
        source_id: String,
        operation: Arc<FeedCacheOperationV1>,
    ) -> Result<Vec<u8>> {
        self.execute(operation, |controller| async move {
            payload(&controller.offline(&source_id).await?, MAX_VIEW_BYTES)
        })
        .await
    }
    pub async fn status(
        &self,
        source_id: String,
        operation: Arc<FeedCacheOperationV1>,
    ) -> Result<Vec<u8>> {
        self.execute(operation, |controller| async move {
            payload(&controller.read_status(&source_id).await?, 16 * 1024)
        })
        .await
    }
    /// Once enqueued cleanup still finishes if the awaiting caller is cancelled.
    pub async fn close(&self, operation: Arc<FeedCacheOperationV1>) -> Result<()> {
        self.execute(
            operation,
            |controller| async move { controller.close().await },
        )
        .await
    }
}
