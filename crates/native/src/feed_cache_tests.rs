// SPDX-License-Identifier: MIT OR Apache-2.0
use super::*;
use serde_json::json;
use std::{
    future::Future,
    sync::{Mutex, atomic::AtomicUsize},
    task::{Context, Poll, Waker},
};
const SOURCE: &str = "b41ef5e8-0e06-43e0-aa47-19f382e00c08";
const SECOND: &str = "b41ef5e8-0e06-43e0-aa47-19f382e00c09";
fn config(two: bool) -> Vec<u8> {
    let sub = |id: &str| json!({"id":id,"url":format!("https://example.org/{id}.xml"),"format":"rss","title":"Synthetic","enabled":true,"collectionIds":[],"disclosure":"private"});
    let subscriptions = if two {
        vec![sub(SOURCE), sub(SECOND)]
    } else {
        vec![sub(SOURCE)]
    };
    serde_json::to_vec(&json!({"schema":"booknook.feed-configuration.v1","revision":0,"feeds":{"schema":"castalia.feeds-section.v1","version":1,"subscriptions":subscriptions,"collections":[],"preferences":{"order":"newest-first"}}})).unwrap()
}
fn raw(items: &str) -> Vec<u8> {
    format!("<rss version=\"2.0\"><channel><title>Synthetic</title>{items}</channel></rss>")
        .into_bytes()
}
fn temp() -> std::path::PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let p = std::env::temp_dir().join(format!(
        "booknook-feed-cache-unit-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&p).unwrap();
    p
}
fn run<T>(f: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(f)
}
// This signal is emitted by SQLite itself after BEGIN IMMEDIATE encounters the
// external write lock, not merely when the worker dequeues the lifecycle call.
thread_local! {
    static BUSY_SIGNAL: std::cell::RefCell<Option<mpsc::SyncSender<()>>> = const { std::cell::RefCell::new(None) };
}
fn sqlite_busy_signal(attempt: i32) -> bool {
    BUSY_SIGNAL.with(|signal| {
        if let Some(sender) = signal.borrow_mut().take() {
            let _ = sender.try_send(());
        }
    });
    if attempt >= 250 {
        return false;
    }
    std::thread::sleep(std::time::Duration::from_millis(1));
    true
}
fn poll<F: Future + ?Sized>(f: std::pin::Pin<&mut F>) -> Poll<F::Output> {
    f.poll(&mut Context::from_waker(Waker::noop()))
}
#[derive(Default)]
struct State {
    raw: Mutex<Vec<u8>>,
    calls: AtomicUsize,
    conditional: AtomicBool,
    pending: AtomicBool,
    notify: tokio::sync::Notify,
    requests: Mutex<Vec<f::FeedSourceRequest>>,
    reenter: Mutex<Option<(FeedCacheController, bool)>>,
}
struct Host(Arc<State>);
#[async_trait::async_trait]
impl FeedSourceHostV1 for Host {
    fn descriptor_json(&self) -> Result<String> {
        Ok(json!({"schema":"booknook.feed-source-provider.v1","version":1,"id":"reader.test","available":true,"protocols":["public-xml-bytes.v1"]}).to_string())
    }
    async fn fetch(
        &self,
        request: String,
        _: Arc<crate::FeedSourceCancellationV1>,
    ) -> Result<crate::FeedSourceReplyV1> {
        let r: f::FeedSourceRequest = serde_json::from_str(&request).unwrap();
        self.0.calls.fetch_add(1, Ordering::SeqCst);
        self.0.requests.lock().unwrap().push(r.clone());
        while self.0.pending.load(Ordering::SeqCst) {
            let n = self.0.notify.notified();
            if !self.0.pending.load(Ordering::SeqCst) {
                break;
            }
            n.await;
        }
        let reenter = self.0.reenter.lock().unwrap().clone();
        if let Some((controller, disable)) = reenter {
            controller.configuration().await?;
            if disable {
                controller.disable(&r.source_id).await?;
            }
        }
        let body = self.0.raw.lock().unwrap().clone();
        let (response, body) =
            if r.conditional.is_some() && self.0.conditional.load(Ordering::SeqCst) {
                (
                    f::FeedSourceResponse::not_modified(&r, &r.url, f::FeedValidators::empty())
                        .unwrap(),
                    vec![],
                )
            } else {
                let mut response =
                    f::FeedSourceResponse::for_request(&r, &r.url, "application/rss+xml", &body)
                        .unwrap();
                response.validators.etag = Some("\"synthetic\"".into());
                (response, body)
            };
        Ok(crate::FeedSourceReplyV1 {
            response_bytes: serde_json::to_vec(&response).unwrap(),
            body,
        })
    }
}
fn setup(two: bool) -> (FeedCacheController, Arc<State>, std::path::PathBuf) {
    let p = temp();
    let state = Arc::new(State::default());
    *state.raw.lock().unwrap() = raw("<item><guid>one</guid><title>One</title></item>");
    let c =
        FeedCacheController::open(&p, Some(config(two)), Box::new(Host(state.clone()))).unwrap();
    (c, state, p)
}
async fn finish(c: &FeedCacheController, p: &Path) {
    c.close().await.unwrap();
    fs::remove_dir_all(p).unwrap();
}
#[test]
fn committed_receipt_reconciles_absent_and_existing_rows() {
    run(async {
        let (c, state, p) = setup(false);
        c.activate(SOURCE).await.unwrap();
        c.call(|w, _| {
            w.faults.after_commit = true;
            Ok(())
        })
        .await
        .unwrap();
        let first = c.refresh(SOURCE).await.unwrap();
        *state.raw.lock().unwrap() = raw("<item><guid>two</guid><title>Two</title></item>");
        let second = c.refresh(SOURCE).await.unwrap();
        assert_ne!(first.raw_sha256, second.raw_sha256);
        assert_eq!(
            c.offline(SOURCE).await.unwrap().raw_sha256,
            second.raw_sha256
        );
        let count: i64 = Connection::open(p.join("feeds.sqlite"))
            .unwrap()
            .query_row("SELECT counter FROM controller", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);
        finish(&c, &p).await;
    });
}
#[test]
fn precommit_failure_rolls_back_and_unreconcilable_ack_is_honestly_closed() {
    run(async {
        let (c, state, p) = setup(false);
        c.activate(SOURCE).await.unwrap();
        let first = c.refresh(SOURCE).await.unwrap();
        *state.raw.lock().unwrap() = raw("<item><guid>two</guid></item>");
        c.call(|w, _| {
            w.faults.before_commit = true;
            Ok(())
        })
        .await
        .unwrap();
        assert!(c.refresh(SOURCE).await.is_err());
        assert_eq!(
            c.offline(SOURCE).await.unwrap().raw_sha256,
            first.raw_sha256
        );
        c.call(|w, _| {
            w.faults = Faults {
                after_commit: true,
                reconcile_unavailable: true,
                before_commit: false,
                ..Faults::default()
            };
            Ok(())
        })
        .await
        .unwrap();
        let error = c.refresh(SOURCE).await.unwrap_err();
        assert!(error.to_string().contains("uncertain"));
        assert!(c.configuration().await.is_err());
        c.close().await.unwrap();
        let reopen = FeedCacheController::open(&p, None, Box::new(Host(state))).unwrap();
        let retained = reopen.offline(SOURCE).await.unwrap();
        assert_ne!(retained.raw_sha256, first.raw_sha256);
        assert!(reopen.refresh(SOURCE).await.is_err());
        finish(&reopen, &p).await;
    });
}
#[test]
fn conditional_304_keeps_raw_and_body_success_but_actual_row_changes_refuse() {
    run(async {
        let (c, state, p) = setup(false);
        c.activate(SOURCE).await.unwrap();
        let first = c.refresh(SOURCE).await.unwrap();
        state.conditional.store(true, Ordering::SeqCst);
        let second = c.refresh(SOURCE).await.unwrap();
        assert_eq!(first.raw_sha256, second.raw_sha256);
        assert_eq!(
            first.last_body_success_unix_seconds,
            second.last_body_success_unix_seconds
        );
        assert_ne!(first.cache_token, second.cache_token);
        for sql in [
            "UPDATE cache SET raw=x'626164'",
            "UPDATE cache SET context=x'7b7d'",
            "UPDATE cache SET profile='future'",
            "UPDATE cache SET context_sha=printf('%064d',0)",
            "UPDATE cache SET cache_token='b41ef5e8-0e06-43e0-aa47-19f382e00c09'",
            "DELETE FROM cache",
        ] {
            let before = state.calls.load(Ordering::SeqCst);
            state.pending.store(true, Ordering::SeqCst);
            let mut refresh = Box::pin(c.refresh(SOURCE));
            while state.calls.load(Ordering::SeqCst) == before {
                assert!(poll(refresh.as_mut()).is_pending());
                tokio::task::yield_now().await;
            }
            Connection::open(p.join("feeds.sqlite"))
                .unwrap()
                .execute_batch(sql)
                .unwrap();
            state.pending.store(false, Ordering::SeqCst);
            state.notify.notify_waiters();
            assert!(refresh.await.is_err());
            // Explicitly restore original bytes via a NEW unconditional controller test
            // row, not an automatic production fallback from corrupt state.
            Connection::open(p.join("feeds.sqlite"))
                .unwrap()
                .execute("DELETE FROM cache", [])
                .unwrap();
            state.conditional.store(false, Ordering::SeqCst);
            c.refresh(SOURCE).await.unwrap();
            state.conditional.store(true, Ordering::SeqCst);
        }
        finish(&c, &p).await;
    });
}
#[test]
fn disable_and_configuration_replacement_do_not_admit_old_reads() {
    run(async {
        let (c, state, p) = setup(true);
        c.activate(SOURCE).await.unwrap();
        c.activate(SECOND).await.unwrap();
        state.pending.store(true, Ordering::SeqCst);
        let mut old = Box::pin(c.refresh(SOURCE));
        while state.calls.load(Ordering::SeqCst) < 1 {
            assert!(poll(old.as_mut()).is_pending());
            tokio::task::yield_now().await;
        }
        let mut newer = Box::pin(c.refresh(SOURCE));
        while state.calls.load(Ordering::SeqCst) < 2 {
            assert!(poll(newer.as_mut()).is_pending());
            tokio::task::yield_now().await;
        }
        c.disable(SOURCE).await.unwrap();
        state.pending.store(false, Ordering::SeqCst);
        state.notify.notify_waiters();
        assert!(old.await.is_err());
        assert!(newer.await.is_err());
        assert!(c.offline(SOURCE).await.is_err());
        let other = c.refresh(SECOND).await.unwrap();
        assert_eq!(other.source_id, SECOND);
        c.activate(SOURCE).await.unwrap();
        state.pending.store(true, Ordering::SeqCst);
        let before = state.calls.load(Ordering::SeqCst);
        let mut pending = Box::pin(c.refresh(SOURCE));
        while state.calls.load(Ordering::SeqCst) == before {
            assert!(poll(pending.as_mut()).is_pending());
            tokio::task::yield_now().await;
        }
        let snapshot = c.configuration().await.unwrap();
        c.replace_configuration(&snapshot.configuration_token, config(true))
            .await
            .unwrap();
        state.pending.store(false, Ordering::SeqCst);
        state.notify.notify_waiters();
        assert!(pending.await.is_err());
        assert!(c.refresh(SECOND).await.is_err());
        assert!(c.offline(SECOND).await.is_ok());
        finish(&c, &p).await;
    });
}
#[test]
fn queue_capacity_survives_dropped_callers_and_cancelled_jobs_never_execute() {
    run(async {
        let (c, _, p) = setup(false);
        let (entered_tx, entered) = mpsc::sync_channel(1);
        let (release, wait) = mpsc::sync_channel(1);
        let mut running = Box::pin(c.call(move |_, _| {
            entered_tx.send(()).unwrap();
            wait.recv().unwrap();
            Ok(())
        }));
        assert!(poll(running.as_mut()).is_pending());
        entered.recv().unwrap();
        let executed = Arc::new(AtomicUsize::new(0));
        for _ in 0..FEED_CACHE_QUEUE_CAPACITY {
            let count = executed.clone();
            let mut queued = Box::pin(c.call(move |_, _| {
                count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }));
            assert!(poll(queued.as_mut()).is_pending());
            drop(queued);
        }
        assert!(
            c.configuration()
                .await
                .unwrap_err()
                .to_string()
                .contains("queue")
        );
        release.send(()).unwrap();
        running.await.unwrap();
        loop {
            match c.configuration().await {
                Ok(_) => break,
                Err(_) => tokio::task::yield_now().await,
            }
        }
        assert_eq!(executed.load(Ordering::SeqCst), 0);
        finish(&c, &p).await;
    });
}
#[test]
fn lifecycle_cancellation_after_real_sqlite_wait_never_activates_or_replaces() {
    run(async {
        let (c, state, p) = setup(false);
        for operation in 0..3 {
            let snapshot = c.configuration().await.unwrap();
            let (entered_tx, entered) = mpsc::sync_channel(1);
            c.call(move |w, _| {
                BUSY_SIGNAL.with(|signal| *signal.borrow_mut() = Some(entered_tx));
                w.db.busy_handler(Some(sqlite_busy_signal))?;
                Ok(())
            })
            .await
            .unwrap();
            let external = Connection::open(p.join("feeds.sqlite")).unwrap();
            external.execute_batch("BEGIN IMMEDIATE").unwrap();
            let mut future: std::pin::Pin<Box<dyn Future<Output = Result<()>>>> = match operation {
                0 => Box::pin(c.activate(SOURCE)),
                1 => {
                    let mut raw = config(false);
                    raw.push(b' ');
                    Box::pin(c.replace_configuration(&snapshot.configuration_token, raw))
                }
                _ => Box::pin(
                    c.replace_provider(&snapshot.provider_token, Box::new(Host(state.clone()))),
                ),
            };
            assert!(poll(future.as_mut()).is_pending());
            // SQLite itself has reported BUSY on the external write lock.
            entered
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
            drop(future);
            external.execute_batch("ROLLBACK").unwrap();
            let current = c.configuration().await.unwrap();
            assert_eq!(current.configuration_token, snapshot.configuration_token);
            assert_eq!(current.provider_token, snapshot.provider_token);
            assert!(!c.read_status(SOURCE).await.unwrap().locally_active);
        }
        finish(&c, &p).await;
    });
}
#[test]
fn saturated_network_blocking_slots_do_not_strand_offline_storage() {
    run(async {
        struct Release(Vec<mpsc::SyncSender<()>>);
        impl Drop for Release {
            fn drop(&mut self) {
                for tx in self.0.drain(..) {
                    let _ = tx.send(());
                }
            }
        }
        let (c, _, p) = setup(false);
        c.activate(SOURCE).await.unwrap();
        c.refresh(SOURCE).await.unwrap();
        let (entered_tx, entered) = mpsc::sync_channel(4);
        let mut release = Release(vec![]);
        let mut handles = vec![];
        for _ in 0..4 {
            let (tx, rx) = mpsc::sync_channel(1);
            release.0.push(tx);
            let entered_tx = entered_tx.clone();
            handles.push(
                crate::feed_source::cache_test_block_network_pool(move || {
                    entered_tx.send(()).unwrap();
                    let _ = rx.recv();
                })
                .unwrap(),
            );
        }
        for _ in 0..4 {
            entered
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        }
        // DNS-equivalent executor saturation only; no actual resolver/network claim.
        tokio::time::timeout(std::time::Duration::from_secs(3), c.offline(SOURCE))
            .await
            .unwrap()
            .unwrap();
        let snapshot = c.configuration().await.unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            c.replace_configuration(&snapshot.configuration_token, config(false)),
        )
        .await
        .unwrap()
        .unwrap();
        drop(release);
        for handle in handles {
            handle.await.unwrap();
        }
        finish(&c, &p).await;
    });
}
#[test]
fn quota_eviction_and_real_sqlite_full_preserve_prior_committed_rows() {
    run(async {
        let (c, state, p) = setup(true);
        c.activate(SOURCE).await.unwrap();
        c.activate(SECOND).await.unwrap();
        let first = c.refresh(SOURCE).await.unwrap();
        c.call(|w, _| {
            w.max_rows = 1;
            Ok(())
        })
        .await
        .unwrap();
        c.refresh(SECOND).await.unwrap();
        assert!(c.offline(SOURCE).await.is_err());
        assert!(c.offline(SECOND).await.is_ok());
        c.call(|w, _| {
            w.max_rows = 128;
            let pages: i64 = w.db.pragma_query_value(None, "page_count", |r| r.get(0))?;
            w.db.pragma_update(None, "max_page_count", pages)?;
            Ok(())
        })
        .await
        .unwrap();
        *state.raw.lock().unwrap() = raw(&(0..80)
            .map(|i| {
                format!(
                    "<item><guid>{i}</guid><description>{}</description></item>",
                    "x".repeat(2000)
                )
            })
            .collect::<String>());
        assert!(c.refresh(SECOND).await.is_err());
        // Prove offline display uses no hidden SQL write, beyond the actual
        // SQLITE_FULL exercise above. This connection now rejects every write.
        c.call(|w, _| {
            w.db.pragma_update(None, "query_only", true)?;
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(
            c.offline(SECOND).await.unwrap().raw_sha256,
            first.raw_sha256
        );
        c.call(|w, _| {
            w.db.pragma_update(None, "query_only", false)?;
            Ok(())
        })
        .await
        .unwrap();
        finish(&c, &p).await;
    });
}
#[test]
fn corruption_is_per_source_and_authoritative_bytes_cannot_drift_under_same_token() {
    run(async {
        let (c, _, p) = setup(true);
        c.activate(SOURCE).await.unwrap();
        c.activate(SECOND).await.unwrap();
        c.refresh(SOURCE).await.unwrap();
        c.refresh(SECOND).await.unwrap();
        let db = Connection::open(p.join("feeds.sqlite")).unwrap();
        db.execute("UPDATE cache SET checked=-1 WHERE source_id=?1", [SOURCE])
            .unwrap();
        assert!(c.offline(SOURCE).await.is_err());
        assert!(c.read_status(SOURCE).await.unwrap().last_error.is_some());
        assert!(c.offline(SECOND).await.is_ok());
        db.execute(
            "UPDATE controller SET configuration=CAST(configuration||' ' AS BLOB)",
            [],
        )
        .unwrap();
        assert!(c.configuration().await.is_err());
        c.close().await.unwrap();
        assert!(
            FeedCacheController::open(&p, None, Box::new(Host(Arc::new(State::default()))))
                .is_err()
        );
        fs::remove_dir_all(&p).unwrap();
    });
}
#[test]
fn variant_keys_survive_reordering_duplicates_and_date_sort_is_exact() {
    run(async {
        let (c, state, p) = setup(false);
        c.activate(SOURCE).await.unwrap();
        let a = "<item><guid>same</guid><title>A</title><pubDate>Thu, 01 Jan 1970 00:00:00 +0000</pubDate></item>";
        let b = "<item><guid>same</guid><title>B</title><pubDate>Wed, 31 Dec 1969 23:59:59 +0000</pubDate></item>";
        let leap =
            "<item><guid>leap</guid><pubDate>Sat, 31 Dec 2016 23:59:60 +0000</pubDate></item>";
        let invalid = "<item><guid>bad</guid><pubDate>invalid</pubDate></item>";
        let missing = "<item><guid>missing</guid></item>";
        *state.raw.lock().unwrap() = raw(&format!("{a}{b}{leap}{invalid}{missing}"));
        let first = c.refresh(SOURCE).await.unwrap();
        let keys = first
            .entries
            .iter()
            .map(|e| {
                (
                    e.entry.identity.clone(),
                    e.entry.title.clone(),
                    e.variant_key.clone(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            first.entries[0].entry.published.nanoseconds,
            Some(1_000_000_000)
        );
        assert_eq!(first.entries[1].entry.published.unix_seconds, Some(0));
        assert_eq!(first.entries[2].entry.published.unix_seconds, Some(-1));
        assert_eq!(
            first.entries[3].entry.published.status,
            x::DateStatus::Invalid
        );
        assert_eq!(
            first.entries[4].entry.published.status,
            x::DateStatus::Missing
        );
        *state.raw.lock().unwrap() = raw(&format!("{missing}{b}{b}{invalid}{leap}{a}"));
        let second = c.refresh(SOURCE).await.unwrap();
        for e in second.entries {
            assert!(keys.iter().any(|(id, title, key)| *id == e.entry.identity
                && *title == e.entry.title
                && *key == e.variant_key));
        }
        finish(&c, &p).await;
    });
}
#[test]
fn dropped_last_handle_never_joins_and_old_worker_keeps_owner_lock() {
    let (c, state, p) = setup(false);
    let (entered_tx, entered) = mpsc::sync_channel(1);
    let (release, wait) = mpsc::sync_channel(1);
    c.sender
        .try_send(Message::Run(Box::new(move |_| {
            entered_tx.send(()).unwrap();
            wait.recv().unwrap();
        })))
        .unwrap();
    entered.recv().unwrap();
    let start = std::time::Instant::now();
    drop(c);
    assert!(start.elapsed() < std::time::Duration::from_millis(100));
    assert!(FeedCacheController::open(&p, None, Box::new(Host(state.clone()))).is_err());
    release.send(()).unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let reopened = loop {
        match FeedCacheController::open(&p, None, Box::new(Host(state.clone()))) {
            Ok(c) => break c,
            Err(_) => {
                assert!(std::time::Instant::now() < until);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    };
    run(finish(&reopened, &p));
}
#[test]
fn corrupt_unicode_tokens_are_errors_without_worker_panic_or_reset() {
    run(async {
        for offset in [35, 36] {
            let (c, state, p) = setup(false);
            let raw = config(false);
            let token = format!("{}é{}", "a".repeat(offset), "b".repeat(101 - offset - 2));
            assert_eq!(token.len(), 101);
            let db = Connection::open(p.join("feeds.sqlite")).unwrap();
            db.execute("UPDATE controller SET configuration_token=?1", [token])
                .unwrap();
            assert!(c.configuration().await.is_err());
            // The worker remains alive: close replies after releasing actual resources.
            c.close().await.unwrap();
            assert!(FeedCacheController::open(&p, None, Box::new(Host(state))).is_err());
            let retained: Vec<u8> = db
                .query_row("SELECT configuration FROM controller", [], |r| r.get(0))
                .unwrap();
            assert_eq!(retained, raw);
            drop(db);
            fs::remove_dir_all(p).unwrap();
        }
    });
}
#[test]
fn forged_quota_accounting_is_rejected_without_old_row_loss() {
    run(async {
        for logical in [-1i64, 0, i64::MAX] {
            let (c, state, p) = setup(false);
            c.activate(SOURCE).await.unwrap();
            let old = c.refresh(SOURCE).await.unwrap();
            Connection::open(p.join("feeds.sqlite"))
                .unwrap()
                .execute("UPDATE cache SET logical=?1", [logical])
                .unwrap();
            *state.raw.lock().unwrap() = raw("<item><guid>replacement</guid></item>");
            assert!(c.refresh(SOURCE).await.is_err());
            assert_eq!(c.offline(SOURCE).await.unwrap().raw_sha256, old.raw_sha256);
            finish(&c, &p).await;
        }
    });
}
#[test]
fn full_context_variants_changed_during_304_never_gain_admission() {
    run(async {
        for field in ["expected_format", "final_url", "provider_id"] {
            let (c, state, p) = setup(false);
            c.activate(SOURCE).await.unwrap();
            c.refresh(SOURCE).await.unwrap();
            state.conditional.store(true, Ordering::SeqCst);
            state.pending.store(true, Ordering::SeqCst);
            let before = state.calls.load(Ordering::SeqCst);
            let mut refresh = Box::pin(c.refresh(SOURCE));
            while state.calls.load(Ordering::SeqCst) == before {
                assert!(poll(refresh.as_mut()).is_pending());
                tokio::task::yield_now().await;
            }
            let db = Connection::open(p.join("feeds.sqlite")).unwrap();
            let old: Vec<u8> = db
                .query_row("SELECT context FROM cache", [], |r| r.get(0))
                .unwrap();
            let mut context: serde_json::Value = serde_json::from_slice(&old).unwrap();
            context[field] = json!(match field {
                "expected_format" => "auto",
                "final_url" => "https://example.net/final.xml",
                _ => "different.provider",
            });
            let changed = serde_json::to_vec(&context).unwrap();
            db.execute(
                "UPDATE cache SET context=?1,context_sha=?2",
                params![changed, booknook_core::digest(&changed)],
            )
            .unwrap();
            state.pending.store(false, Ordering::SeqCst);
            state.notify.notify_waiters();
            assert!(refresh.await.is_err());
            let actual: Vec<u8> = db
                .query_row("SELECT context FROM cache", [], |r| r.get(0))
                .unwrap();
            assert_eq!(actual, changed);
            drop(db);
            finish(&c, &p).await;
        }
    });
}
#[test]
fn actual_foreign_callback_can_reenter_and_newer_refresh_alone_supersedes() {
    run(async {
        let (c, state, p) = setup(false);
        c.activate(SOURCE).await.unwrap();
        *state.reenter.lock().unwrap() = Some((c.clone(), false));
        c.refresh(SOURCE).await.unwrap();
        *state.reenter.lock().unwrap() = Some((c.clone(), true));
        assert!(c.refresh(SOURCE).await.is_err());
        *state.reenter.lock().unwrap() = None;
        c.activate(SOURCE).await.unwrap();
        state.pending.store(true, Ordering::SeqCst);
        let before = state.calls.load(Ordering::SeqCst);
        let mut old = Box::pin(c.refresh(SOURCE));
        while state.calls.load(Ordering::SeqCst) == before {
            assert!(poll(old.as_mut()).is_pending());
            tokio::task::yield_now().await;
        }
        let mut newer = Box::pin(c.refresh(SOURCE));
        while state.calls.load(Ordering::SeqCst) == before + 1 {
            assert!(poll(newer.as_mut()).is_pending());
            tokio::task::yield_now().await;
        }
        state.pending.store(false, Ordering::SeqCst);
        state.notify.notify_waiters();
        assert!(old.await.is_err());
        assert!(newer.await.is_ok());
        finish(&c, &p).await;
    });
}
#[test]
fn exact_receipt_refuses_changed_metadata_after_real_commit() {
    run(async {
        let (c, _, p) = setup(false);
        c.activate(SOURCE).await.unwrap();
        c.call(|w, _| {
            w.faults = Faults {
                after_commit: true,
                corrupt_after_commit: true,
                ..Faults::default()
            };
            Ok(())
        })
        .await
        .unwrap();
        let error = c.refresh(SOURCE).await.unwrap_err();
        assert!(error.to_string().contains("uncertain"));
        assert!(c.offline(SOURCE).await.is_err());
        c.close().await.unwrap();
        fs::remove_dir_all(p).unwrap();
    });
}
#[test]
fn explicit_endpoint_and_provider_edit_invalidation_is_atomic() {
    run(async {
        let (c, state, p) = setup(false);
        c.activate(SOURCE).await.unwrap();
        let original = c.refresh(SOURCE).await.unwrap();
        let snapshot = c.configuration().await.unwrap();
        let mut changed: serde_json::Value = serde_json::from_slice(&config(false)).unwrap();
        changed["feeds"]["subscriptions"][0]["url"] = json!("https://example.net/new.xml");
        let changed = serde_json::to_vec(&changed).unwrap();
        c.call(|w, _| {
            w.faults.before_lifecycle_commit = true;
            Ok(())
        })
        .await
        .unwrap();
        assert!(
            c.replace_configuration(&snapshot.configuration_token, changed.clone())
                .await
                .is_err()
        );
        assert_eq!(
            c.offline(SOURCE).await.unwrap().raw_sha256,
            original.raw_sha256
        );
        c.call(|w, _| {
            w.faults.before_lifecycle_commit = false;
            Ok(())
        })
        .await
        .unwrap();
        c.replace_configuration(&snapshot.configuration_token, changed)
            .await
            .unwrap();
        assert!(c.offline(SOURCE).await.is_err());
        c.activate(SOURCE).await.unwrap();
        c.refresh(SOURCE).await.unwrap();
        assert!(
            state
                .requests
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .conditional
                .is_none()
        );
        struct Different(Arc<State>);
        #[async_trait::async_trait]
        impl FeedSourceHostV1 for Different {
            fn descriptor_json(&self) -> Result<String> {
                Ok(json!({"schema":"booknook.feed-source-provider.v1","version":1,"id":"different.provider","available":true,"protocols":["public-xml-bytes.v1"]}).to_string())
            }
            async fn fetch(
                &self,
                r: String,
                c: Arc<crate::FeedSourceCancellationV1>,
            ) -> Result<crate::FeedSourceReplyV1> {
                Host(self.0.clone()).fetch(r, c).await
            }
        }
        let snapshot = c.configuration().await.unwrap();
        c.call(|w, _| {
            w.faults.before_lifecycle_commit = true;
            Ok(())
        })
        .await
        .unwrap();
        assert!(
            c.replace_provider(&snapshot.provider_token, Box::new(Different(state.clone())))
                .await
                .is_err()
        );
        assert!(c.offline(SOURCE).await.is_ok());
        c.call(|w, _| {
            w.faults.before_lifecycle_commit = false;
            Ok(())
        })
        .await
        .unwrap();
        c.replace_provider(&snapshot.provider_token, Box::new(Different(state.clone())))
            .await
            .unwrap();
        assert!(c.offline(SOURCE).await.is_err());
        c.activate(SOURCE).await.unwrap();
        let result = c.refresh(SOURCE).await.unwrap();
        assert_eq!(result.provenance.provider_id, "different.provider");
        finish(&c, &p).await;
    });
}
