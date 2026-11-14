// SPDX-License-Identifier: MIT OR Apache-2.0
//! Native-controller acceptance uses real SQLite and a deterministic installed
//! provider. This does not qualify any external network transport.
use booknook_core::feed_source::{FeedSourceRequest, FeedSourceResponse};
use booknook_native::{
    FeedCacheController, FeedSourceCancellationV1, FeedSourceHostV1, FeedSourceReplyV1,
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
fn configuration() -> Vec<u8> {
    let mut section =
        serde_json::from_slice::<Value>(include_bytes!("fixtures/feeds-v1/section.json")).unwrap();
    section["subscriptions"][0]["format"] = json!("rss");
    serde_json::to_vec(
        &json!({"schema":"booknook.feed-configuration.v1","revision":0,"feeds":section}),
    )
    .unwrap()
}
fn source() -> String {
    serde_json::from_slice::<Value>(&configuration()).unwrap()["feeds"]["subscriptions"][0]["id"]
        .as_str()
        .unwrap()
        .into()
}
fn directory() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "booknook-cache-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
struct Host(Arc<AtomicUsize>);
#[async_trait::async_trait]
impl FeedSourceHostV1 for Host {
    fn descriptor_json(&self) -> Result<String, booknook_native::NativeError> {
        Ok(json!({"schema":"booknook.feed-source-provider.v1","version":1,"id":"reader.public","available":true,"protocols":["public-xml-bytes.v1"]}).to_string())
    }
    async fn fetch(
        &self,
        request: String,
        _: Arc<FeedSourceCancellationV1>,
    ) -> Result<FeedSourceReplyV1, booknook_native::NativeError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        let r: FeedSourceRequest = serde_json::from_str(&request).unwrap();
        let raw=br#"<rss version="2.0"><channel><title>Local</title><item><guid>same</guid><title>A</title></item><item><guid>same</guid><title>B</title></item></channel></rss>"#.to_vec();
        let mut response =
            FeedSourceResponse::for_request(&r, &r.url, "application/rss+xml", &raw).unwrap();
        response.validators.etag = Some("\"one\"".into());
        Ok(FeedSourceReplyV1 {
            response_bytes: serde_json::to_vec(&response).unwrap(),
            body: raw,
        })
    }
}
fn host() -> Box<dyn FeedSourceHostV1> {
    Box::new(Host(Arc::new(AtomicUsize::new(0))))
}
fn run<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(f)
}
#[test]
fn authoritative_open_owner_lock_and_inactive_restart() {
    run(async {
        let dir = directory();
        let raw = configuration();
        let c = FeedCacheController::open(&dir, Some(raw.clone()), host()).unwrap();
        assert_eq!(c.configuration().await.unwrap().raw, raw);
        assert!(FeedCacheController::open(&dir, None, host()).is_err());
        c.activate(&source()).await.unwrap();
        c.refresh(&source()).await.unwrap();
        c.close().await.unwrap();
        let mut wrong = configuration();
        wrong.push(b' ');
        assert!(FeedCacheController::open(&dir, Some(wrong), host()).is_err());
        let reopened = FeedCacheController::open(&dir, None, host()).unwrap();
        assert!(reopened.refresh(&source()).await.is_err());
        let view = reopened.offline(&source()).await.unwrap();
        assert_eq!(view.entries.len(), 2);
        assert_ne!(view.entries[0].variant_key, view.entries[1].variant_key);
        assert_eq!(reopened.configuration().await.unwrap().raw, raw);
        reopened.close().await.unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    });
}
#[test]
fn exact_configuration_cas_revokes_activation_without_losing_raw() {
    run(async {
        let dir = directory();
        let c = FeedCacheController::open(&dir, Some(configuration()), host()).unwrap();
        c.activate(&source()).await.unwrap();
        let before = c.refresh(&source()).await.unwrap();
        let snapshot = c.configuration().await.unwrap();
        assert!(
            c.replace_configuration("wrong", configuration())
                .await
                .is_err()
        );
        let mut changed = configuration();
        changed.push(b' ');
        c.replace_configuration(&snapshot.configuration_token, changed.clone())
            .await
            .unwrap();
        assert!(c.refresh(&source()).await.is_err());
        let after = c.offline(&source()).await.unwrap();
        assert_eq!(before.raw_sha256, after.raw_sha256);
        assert_eq!(c.configuration().await.unwrap().raw, changed);
        c.close().await.unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    });
}

// Helper is invoked only by the parent acceptance test in a real child process.
#[test]
fn cache_owner_child() {
    let Some(path) = std::env::var_os("BOOKNOOK_CACHE_CHILD_DIRECTORY") else {
        return;
    };
    let dir = PathBuf::from(path);
    let c = FeedCacheController::open(&dir, Some(configuration()), host()).unwrap();
    run(async {
        c.activate(&source()).await.unwrap();
        c.refresh(&source()).await.unwrap();
    });
    std::fs::write(dir.join("child-ready"), b"committed").unwrap();
    loop {
        std::thread::park_timeout(std::time::Duration::from_secs(1));
    }
}
#[test]
fn forced_child_termination_preserves_committed_raw_and_restarts_inactive() {
    let dir = directory();
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Child(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "cache_owner_child", "--nocapture"])
            .env("BOOKNOOK_CACHE_CHILD_DIRECTORY", &dir)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let until = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !dir.join("child-ready").exists() {
        assert!(
            std::time::Instant::now() < until,
            "child did not commit before deadline"
        );
        assert!(child.0.try_wait().unwrap().is_none());
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(FeedCacheController::open(&dir, None, host()).is_err());
    let db = rusqlite::Connection::open(dir.join("feeds.sqlite")).unwrap();
    let prior_owner: String = db
        .query_row("SELECT owner FROM controller", [], |r| r.get(0))
        .unwrap();
    let raw: Vec<u8> = db
        .query_row("SELECT raw FROM cache", [], |r| r.get(0))
        .unwrap();
    drop(db);
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    let reopened = FeedCacheController::open(&dir, None, host()).unwrap();
    run(async {
        assert!(
            !reopened
                .read_status(&source())
                .await
                .unwrap()
                .locally_active
        );
        assert!(reopened.refresh(&source()).await.is_err());
        let offline = reopened.offline(&source()).await.unwrap();
        assert_eq!(offline.raw_sha256, booknook_core::digest(&raw));
        assert_eq!(offline.entries.len(), 2);
        let new_owner: String = rusqlite::Connection::open(dir.join("feeds.sqlite"))
            .unwrap()
            .query_row("SELECT owner FROM controller", [], |r| r.get(0))
            .unwrap();
        assert_ne!(prior_owner, new_owner);
        reopened.close().await.unwrap();
    });
    std::fs::remove_dir_all(dir).unwrap();
}
