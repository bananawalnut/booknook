// SPDX-License-Identifier: MIT OR Apache-2.0
type NativeResult<T> = std::result::Result<T, booknook_native::NativeError>;
use booknook_core::feed_source::{FeedSourceRequest, FeedSourceResponse};
use booknook_native::{
    FeedCacheHostV1, FeedCacheStageV1, FeedSourceCancellationV1, FeedSourceHostV1,
    FeedSourceReplyV1, FeedSourceSessionV1,
};
use serde_json::{Value, json};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
};
fn bytes(value: &impl serde::Serialize) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}
fn config() -> Vec<u8> {
    bytes(
        &json!({"schema":"booknook.feed-configuration.v1","revision":0,"feeds":serde_json::from_slice::<Value>(include_bytes!("fixtures/feeds-v1/section.json")).unwrap()}),
    )
}
fn source() -> String {
    serde_json::from_slice::<Value>(&config()).unwrap()["feeds"]["subscriptions"][0]["id"]
        .as_str()
        .unwrap()
        .into()
}
fn descriptor() -> String {
    json!({"schema":"booknook.feed-source-provider.v1","version":1,"id":"reader.public","available":true,"protocols":["public-xml-bytes.v1"]}).to_string()
}
fn poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}
struct Host {
    pending: Arc<Mutex<bool>>,
    calls: Arc<AtomicUsize>,
}
#[async_trait::async_trait]
impl FeedSourceHostV1 for Host {
    fn descriptor_json(&self) -> NativeResult<String> {
        Ok(descriptor())
    }
    async fn fetch(
        &self,
        request_json: String,
        cancellation: Arc<FeedSourceCancellationV1>,
    ) -> NativeResult<FeedSourceReplyV1> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let pending = self.pending.clone();
        std::future::poll_fn(move |_| {
            if *pending.lock().unwrap() {
                Poll::Pending
            } else {
                Poll::Ready(())
            }
        })
        .await;
        let _ = cancellation.is_cancelled();
        let request: FeedSourceRequest = serde_json::from_str(&request_json).unwrap();
        let body = if request.conditional.is_some() {
            vec![]
        } else {
            b"<rss/>".to_vec()
        };
        let response = if request.conditional.is_some() {
            FeedSourceResponse::not_modified(
                &request,
                &request.url,
                booknook_core::feed_source::FeedValidators::empty(),
            )
            .unwrap()
        } else {
            FeedSourceResponse::for_request(&request, &request.url, "application/rss+xml", &body)
                .unwrap()
        };
        Ok(FeedSourceReplyV1 {
            response_bytes: bytes(&response),
            body,
        })
    }
}
struct Cache {
    session: Arc<FeedSourceSessionV1>,
    invalidate: bool,
    commits: Arc<AtomicUsize>,
    retained: Option<Vec<u8>>,
}
#[async_trait::async_trait]
impl FeedCacheHostV1 for Cache {
    async fn stage(
        &self,
        _condition_json: String,
        _body: Vec<u8>,
        _cancellation: Arc<FeedSourceCancellationV1>,
    ) -> NativeResult<FeedCacheStageV1> {
        std::future::ready(()).await;
        if self.invalidate {
            self.session.replace_configuration(config())?;
        }
        Ok(FeedCacheStageV1 {
            reference: "staged-1".into(),
            existing_body: self.retained.clone(),
        })
    }
    fn compare_and_store(
        &self,
        staged: String,
        condition_json: String,
        body: Vec<u8>,
    ) -> NativeResult<()> {
        assert_eq!(staged, "staged-1");
        if self.retained.is_some() {
            assert!(body.is_empty());
        } else {
            assert_eq!(body, b"<rss/>");
        }
        let condition: Value = serde_json::from_str(&condition_json).unwrap();
        assert_eq!(condition["schema"], "booknook.feed-cache-condition.v1");
        assert!(self.session.disable(source()).is_err()); // no foreign callback deadlock
        self.commits.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
#[test]
fn actual_native_async_source_callback_requires_activation_and_rejects_inflight_replacements() {
    for stale in [false, true] {
        let pending = Arc::new(Mutex::new(true));
        let calls = Arc::new(AtomicUsize::new(0));
        let session = FeedSourceSessionV1::new(
            config(),
            Box::new(Host {
                pending: pending.clone(),
                calls: calls.clone(),
            }),
        )
        .unwrap();
        assert!(session.begin(source(), None).is_err());
        session.activate(source()).unwrap();
        let attempt = session.begin(source(), None).unwrap();
        let mut future = Box::pin(attempt.execute());
        assert!(poll(future.as_mut()).is_pending());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        if stale {
            session.replace_configuration(config()).unwrap();
            session.activate(source()).unwrap();
        }
        *pending.lock().unwrap() = false;
        let result = poll(future.as_mut());
        if stale {
            assert!(matches!(result, Poll::Ready(Err(_))));
        } else {
            assert!(matches!(result, Poll::Ready(Ok(_))));
        }
    }
}
#[test]
fn actual_native_async_storage_uses_guarded_synchronous_final_cas_and_rechecks_staging() {
    for stale in [false, true] {
        let session = Arc::new(
            FeedSourceSessionV1::new(
                config(),
                Box::new(Host {
                    pending: Arc::new(Mutex::new(false)),
                    calls: Arc::new(AtomicUsize::new(0)),
                }),
            )
            .unwrap(),
        );
        session.activate(source()).unwrap();
        let attempt = session.begin(source(), None).unwrap();
        let mut fetch = Box::pin(attempt.execute());
        let Poll::Ready(Ok(read)) = poll(fetch.as_mut()) else {
            panic!("fetch")
        };
        let commits = Arc::new(AtomicUsize::new(0));
        let mut store = Box::pin(read.store(Box::new(Cache {
            session: session.clone(),
            invalidate: stale,
            commits: commits.clone(),
            retained: None,
        })));
        let result = poll(store.as_mut());
        if stale {
            assert!(matches!(result, Poll::Ready(Err(_))));
        } else {
            assert!(matches!(result, Poll::Ready(Ok(()))));
        }
        assert_eq!(commits.load(Ordering::SeqCst), usize::from(!stale));
    }
}

fn hint() -> Vec<u8> {
    let configuration: Value = serde_json::from_slice(&config()).unwrap();
    bytes(
        &json!({"schema":"booknook.feed-conditional-hint.v1","source_id":source(),"provider_id":"reader.public","url":configuration["feeds"]["subscriptions"][0]["url"],"body_sha256":booknook_core::digest(b"<rss/>"),"validators":{"etag":"\"v1\"","last_modified":null}}),
    )
}
#[test]
fn actual_native_304_requires_matching_retained_body_and_current_generation_at_final_cas() {
    for retained in [None, Some(b"wrong".to_vec()), Some(b"<rss/>".to_vec())] {
        let session = Arc::new(
            FeedSourceSessionV1::new(
                config(),
                Box::new(Host {
                    pending: Arc::new(Mutex::new(false)),
                    calls: Arc::new(AtomicUsize::new(0)),
                }),
            )
            .unwrap(),
        );
        session.activate(source()).unwrap();
        let attempt = session.begin(source(), Some(hint())).unwrap();
        let mut future = Box::pin(attempt.execute());
        let Poll::Ready(Ok(read)) = poll(future.as_mut()) else {
            panic!("fetch304")
        };
        assert!(read.body().is_empty());
        let good = retained.as_deref() == Some(b"<rss/>".as_slice());
        let writes = Arc::new(AtomicUsize::new(0));
        let mut store = Box::pin(read.store(Box::new(Cache {
            session: session.clone(),
            invalidate: false,
            commits: writes.clone(),
            retained,
        })));
        let result = poll(store.as_mut());
        if good {
            assert!(matches!(result, Poll::Ready(Ok(()))));
        } else {
            assert!(matches!(result, Poll::Ready(Err(_))));
        }
        assert_eq!(writes.load(Ordering::SeqCst), usize::from(good));
    }
    for field in ["provider_id", "source_id", "url"] {
        let session = FeedSourceSessionV1::new(
            config(),
            Box::new(Host {
                pending: Arc::new(Mutex::new(false)),
                calls: Arc::new(AtomicUsize::new(0)),
            }),
        )
        .unwrap();
        session.activate(source()).unwrap();
        let mut value: Value = serde_json::from_slice(&hint()).unwrap();
        value[field] = json!("wrong");
        assert!(session.begin(source(), Some(bytes(&value))).is_err());
    }
    let pending = Arc::new(Mutex::new(true));
    let session = Arc::new(
        FeedSourceSessionV1::new(
            config(),
            Box::new(Host {
                pending: pending.clone(),
                calls: Arc::new(AtomicUsize::new(0)),
            }),
        )
        .unwrap(),
    );
    session.activate(source()).unwrap();
    let attempt = session.begin(source(), Some(hint())).unwrap();
    let mut future = Box::pin(attempt.execute());
    assert!(poll(future.as_mut()).is_pending());
    session.replace_configuration(config()).unwrap();
    *pending.lock().unwrap() = false;
    assert!(matches!(poll(future.as_mut()), Poll::Ready(Err(_))));
}
