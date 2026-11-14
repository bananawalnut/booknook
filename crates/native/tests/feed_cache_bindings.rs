// SPDX-License-Identifier: MIT OR Apache-2.0
//! Real exported Rust objects; synthetic provider, no external transport claim.
use booknook_native::{FeedCacheControllerV1, FeedCacheOperationV1};
use std::sync::Arc;

#[test]
fn public_https_constructor_stays_offline_and_operation_is_one_use() {
    let dir = std::env::temp_dir().join(format!(
        "booknook-cache-binding-{}",
        booknook_core::new_id()
    ));
    let raw = br#"{"schema":"booknook.feed-configuration.v1","revision":0,"feeds":{"schema":"castalia.feeds-section.v1","version":1,"subscriptions":[],"collections":[],"preferences":{"order":"newest-first"}}}"#.to_vec();
    let cache =
        FeedCacheControllerV1::public_https(dir.to_str().unwrap().into(), Some(raw.clone()))
            .unwrap();
    // Deliberately no Tokio reactor: this future only needs the storage worker.
    fn run<F: std::future::Future>(future: F) -> F::Output {
        use std::task::{Context, Poll, Wake, Waker};
        struct Thread(std::thread::Thread);
        impl Wake for Thread {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = Waker::from(Arc::new(Thread(std::thread::current())));
        let mut future = std::pin::pin!(future);
        loop {
            match future.as_mut().poll(&mut Context::from_waker(&waker)) {
                Poll::Ready(value) => return value,
                Poll::Pending => std::thread::park(),
            }
        }
    }
    let operation = Arc::new(FeedCacheOperationV1::new());
    let snapshot = run(cache.configuration(operation.clone())).unwrap();
    assert_eq!(snapshot.raw, raw);
    assert!(run(cache.configuration(operation)).is_err());
    let canceled = Arc::new(FeedCacheOperationV1::new());
    canceled.cancel();
    assert!(run(cache.configuration(canceled)).is_err());
    run(cache.close(Arc::new(FeedCacheOperationV1::new()))).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}
