// SPDX-License-Identifier: MIT OR Apache-2.0
use booknook_core::{
    Error, Result,
    feed_source::*,
    feeds::{FeedConfiguration, FeedSection},
};
use serde_json::{Value, json};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
};
fn config() -> FeedConfiguration {
    FeedConfiguration::new(
        0,
        FeedSection::parse(include_bytes!("fixtures/feeds-v1/section.json")).unwrap(),
    )
    .unwrap()
}
fn descriptor(id: &str) -> FeedSourceDescriptor {
    FeedSourceDescriptor {
        schema: "booknook.feed-source-provider.v1".into(),
        version: 1,
        id: id.into(),
        available: true,
        protocols: vec![FeedSourceProtocol::PublicXmlBytesV1],
    }
}
fn mk_session() -> FeedSourceSession {
    FeedSourceSession::new(config(), vec![descriptor("reader.public")]).unwrap()
}
fn source() -> String {
    config().feeds.subscriptions[0].id.clone()
}
fn mk_attempt(session: &FeedSourceSession) -> FeedSourceAttempt {
    session.activate(&source(), "reader.public").unwrap();
    session.begin(&source()).unwrap()
}
fn response(request: &FeedSourceRequest, body: &[u8]) -> Vec<u8> {
    serde_json::to_vec(
        &FeedSourceResponse::for_request(request, &request.url, "application/rss+xml", body)
            .unwrap(),
    )
    .unwrap()
}
fn poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}
struct Source {
    calls: AtomicUsize,
    pending: Mutex<bool>,
    descriptor: FeedSourceDescriptor,
}
impl Source {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            pending: Mutex::new(true),
            descriptor: descriptor("reader.public"),
        }
    }
}
impl FeedSourceProvider for Source {
    fn descriptor(&self) -> Result<FeedSourceDescriptor> {
        Ok(self.descriptor.clone())
    }
    fn fetch<'a>(
        &'a self,
        request: FeedSourceRequest,
        cancellation: FeedCancellation,
    ) -> FeedFuture<'a, FeedSourceReply> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(std::future::poll_fn(move |_| {
            if *self.pending.lock().unwrap() {
                return Poll::Pending;
            }
            if cancellation.is_cancelled() {
                return Poll::Ready(Err(Error::Conflict("cancelled".into())));
            }
            let body = b"<rss/>".to_vec();
            Poll::Ready(Ok(FeedSourceReply {
                response_bytes: response(&request, &body),
                body,
            }))
        }))
    }
}
#[test]
fn imported_enabled_and_advertisement_cannot_fetch_without_local_activation() {
    let session = mk_session();
    let source = Source::new();
    assert!(config().feeds.subscriptions[0].enabled);
    assert!(session.begin(&self::source()).is_err());
    assert!(session.activate(&self::source(), "not-installed").is_err());
    assert_eq!(source.calls.load(Ordering::SeqCst), 0);
    let attempt = mk_attempt(&session);
    let request = attempt.dispatch().unwrap();
    let value = serde_json::to_value(request).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 12);
    for private in [
        "title",
        "collections",
        "collectionIds",
        "disclosure",
        "identity",
        "configurationDigest",
    ] {
        assert!(value.get(private).is_none());
    }
    assert!(attempt.dispatch().is_err());
}
#[test]
fn actual_async_provider_execution_rechecks_after_await_and_binds_provider() {
    let session = mk_session();
    let provider = Source::new();
    let attempt = mk_attempt(&session);
    let mut read = Box::pin(attempt.execute(&provider));
    assert!(poll(read.as_mut()).is_pending());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    *provider.pending.lock().unwrap() = false;
    let Poll::Ready(Ok(read)) = poll(read.as_mut()) else {
        panic!("provider did not complete")
    };
    assert_eq!(read.body(), b"<rss/>");
    assert!(read.cache_condition().is_ok());
    let wrong = Source {
        descriptor: descriptor("different"),
        ..Source::new()
    };
    let attempt = session.begin(&source()).unwrap();
    let mut future = Box::pin(attempt.execute(&wrong));
    assert!(matches!(poll(future.as_mut()), Poll::Ready(Err(_))));
    assert_eq!(wrong.calls.load(Ordering::SeqCst), 0);
}
#[test]
fn disabled_cancelled_and_configuration_aba_reads_never_produce_cache_admission() {
    for change in 0..4 {
        let session = mk_session();
        let provider = Source::new();
        let attempt = mk_attempt(&session);
        let token = attempt.cancellation();
        let mut future = Box::pin(attempt.execute(&provider));
        assert!(poll(future.as_mut()).is_pending());
        match change {
            0 => session.disable(&source()).unwrap(),
            1 => {
                attempt.cancel();
            }
            2 => {
                session.replace_configuration(config()).unwrap();
                session.activate(&source(), "reader.public").unwrap();
            }
            _ => {
                session
                    .replace_providers(vec![descriptor("reader.public")])
                    .unwrap();
                session.activate(&source(), "reader.public").unwrap();
            }
        }
        assert!(token.is_cancelled());
        *provider.pending.lock().unwrap() = false;
        assert!(matches!(poll(future.as_mut()), Poll::Ready(Err(_))));
    }
    let mut disabled = config();
    disabled.feeds.subscriptions[0].enabled = false;
    let session = FeedSourceSession::new(disabled, vec![descriptor("reader.public")]).unwrap();
    assert!(session.activate(&source(), "reader.public").is_err());
}
#[test]
fn response_correlation_raw_validation_and_endpoint_policy_fail_whole_read() {
    for field in [
        "runtime_id",
        "request_id",
        "source_id",
        "provider_id",
        "configuration_generation",
        "provider_generation",
        "activation_generation",
        "final_url",
        "body_sha256",
    ] {
        let session = mk_session();
        let attempt = mk_attempt(&session);
        let request = attempt.dispatch().unwrap();
        let body = b"<rss/>";
        let mut value: Value = serde_json::from_slice(&response(&request, body)).unwrap();
        value[field] = if field.ends_with("generation") {
            json!(999)
        } else if field == "final_url" {
            json!("https://127.0.0.1/")
        } else {
            json!("wrong")
        };
        assert!(
            attempt
                .complete(&serde_json::to_vec(&value).unwrap(), body.to_vec())
                .is_err(),
            "{field}"
        );
        assert!(
            attempt
                .complete(&response(&request, body), body.to_vec())
                .is_err()
        );
    }
    for endpoint in [
        "https://reader.example.org/feed?token=a",
        "https://reader.example.org:443/feed",
        "https://reader.example.org/feed#x",
        "https://reader.local/feed",
        "http://reader.example.org/feed",
    ] {
        let session = mk_session();
        let attempt = mk_attempt(&session);
        let request = attempt.dispatch().unwrap();
        assert!(
            FeedSourceResponse::for_request(&request, endpoint, "application/xml", b"x").is_err()
        );
    }
    let session = mk_session();
    let attempt = mk_attempt(&session);
    let request = attempt.dispatch().unwrap();
    let raw = response(&request, b"x");
    let text = String::from_utf8(raw).unwrap();
    let duplicate = text.replacen("{", "{\"schema\":\"booknook.feed-source-response.v1\",", 1);
    assert!(
        attempt
            .complete(duplicate.as_bytes(), b"x".to_vec())
            .is_err()
    );
    let session = mk_session();
    let attempt = mk_attempt(&session);
    let request = attempt.dispatch().unwrap();
    let body = vec![b'x'; MAX_FEED_BODY_BYTES + 1];
    assert!(
        FeedSourceResponse::for_request(&request, &request.url, "application/xml", &body).is_err()
    );
    assert!(attempt.complete(&response(&request, b"x"), body).is_err());
}
#[test]
fn completed_candidate_remains_opaque_current_and_cancelable_before_cache_commit() {
    let session = mk_session();
    let attempt = mk_attempt(&session);
    let request = attempt.dispatch().unwrap();
    let read = attempt
        .complete(&response(&request, b"<rss/>"), b"<rss/>".to_vec())
        .unwrap();
    let condition = read.cache_condition().unwrap();
    assert_eq!(condition.request_id, request.request_id);
    assert_eq!(condition.body_sha256, booknook_core::digest(b"<rss/>"));
    session.disable(&source()).unwrap();
    assert!(read.cache_condition().is_err());
    session.activate(&source(), "reader.public").unwrap();
    assert!(read.cache_condition().is_err());
}
struct Cache {
    staged: AtomicUsize,
    committed: AtomicUsize,
    pending: Mutex<bool>,
    session: FeedSourceSession,
}
impl FeedCacheStore for Cache {
    fn compare_and_store<'a>(&'a self, candidate: &'a FeedSourceRead) -> FeedFuture<'a, ()> {
        self.staged.fetch_add(1, Ordering::SeqCst);
        Box::pin(std::future::poll_fn(move |_| {
            if *self.pending.lock().unwrap() {
                return Poll::Pending;
            }
            // Test adapter's final synchronous CAS is guarded by the live runtime;
            // a production durable adapter must also compare its own persisted key.
            Poll::Ready(candidate.with_current_cache_commit(None, |_| {
                self.committed.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }))
        }))
    }
}
#[test]
fn async_cache_adapter_rechecks_after_storage_await_and_atomically_guards_commit() {
    for change in [false, true] {
        let session = mk_session();
        let attempt = mk_attempt(&session);
        let request = attempt.dispatch().unwrap();
        let read = attempt
            .complete(&response(&request, b"<rss/>"), b"<rss/>".to_vec())
            .unwrap();
        let cache = Cache {
            staged: AtomicUsize::new(0),
            committed: AtomicUsize::new(0),
            pending: Mutex::new(true),
            session: session.clone(),
        };
        let mut future = Box::pin(read.store(&cache));
        assert!(poll(future.as_mut()).is_pending());
        if change {
            cache.session.replace_configuration(config()).unwrap();
        }
        *cache.pending.lock().unwrap() = false;
        let result = poll(future.as_mut());
        if change {
            assert!(matches!(result, Poll::Ready(Err(_))));
        } else {
            assert!(matches!(result, Poll::Ready(Ok(()))));
        }
        assert_eq!(cache.committed.load(Ordering::SeqCst), usize::from(!change));
    }
}
#[test]
fn independent_descriptor_protocol_is_strict_and_does_not_extend_provider_v1() {
    let raw = serde_json::to_vec(&descriptor("reader.public")).unwrap();
    assert!(FeedSourceDescriptor::parse(&raw).is_ok());
    for token in ["1.0", "1e0", "2"] {
        let text = String::from_utf8(raw.clone())
            .unwrap()
            .replace("\"version\":1", &format!("\"version\":{token}"));
        assert!(FeedSourceDescriptor::parse(text.as_bytes()).is_err());
    }
    let duplicate = String::from_utf8(raw)
        .unwrap()
        .replacen("{", "{\"version\":1,", 1);
    assert!(FeedSourceDescriptor::parse(duplicate.as_bytes()).is_err());
    let mut unavailable = descriptor("reader.public");
    unavailable.available = false;
    let session = FeedSourceSession::new(config(), vec![unavailable]).unwrap();
    assert!(session.activate(&source(), "reader.public").is_err());
}

#[test]
fn guarded_commit_reentry_fails_without_deadlock_and_cannot_retract_started_commit() {
    let session = mk_session();
    let attempt = mk_attempt(&session);
    let request = attempt.dispatch().unwrap();
    let read = attempt
        .complete(&response(&request, b"x"), b"x".to_vec())
        .unwrap();
    read.with_current_cache_commit(None, |_| {
        assert!(session.replace_configuration(config()).is_err());
        assert!(session.disable(&source()).is_err());
        assert!(!read.cancellation().cancel());
        Ok(())
    })
    .unwrap();
    assert!(read.cache_condition().is_err());
    assert!(read.with_current_cache_commit(None, |_| Ok(())).is_err());
}
#[test]
fn one_source_disable_does_not_cancel_another_and_invalid_replacements_are_atomic() {
    let mut configuration = config();
    let mut second = configuration.feeds.subscriptions[0].clone();
    second.id = "b41ef5e8-0e06-43e0-aa47-000000000999".into();
    second.url = "https://other.example.org/feed".into();
    let second_id = second.id.clone();
    configuration.feeds.subscriptions.push(second);
    let session = FeedSourceSession::new(configuration, vec![descriptor("reader.public")]).unwrap();
    let first = mk_attempt(&session);
    let request = first.dispatch().unwrap();
    session.activate(&second_id, "reader.public").unwrap();
    session.disable(&second_id).unwrap();
    assert!(!first.cancellation().is_cancelled());
    let mut invalid = descriptor("reader.public");
    invalid.version = 2;
    assert!(session.replace_providers(vec![invalid]).is_err());
    assert!(
        first
            .complete(&response(&request, b"x"), b"x".to_vec())
            .unwrap()
            .cache_condition()
            .is_ok()
    );
}

#[test]
fn newer_request_supersedes_pending_and_completed_uncommitted_reads_for_only_its_source() {
    let session = mk_session();
    let first = mk_attempt(&session);
    let request = first.dispatch().unwrap();
    let second = session.begin(&source()).unwrap();
    assert!(first.cancellation().is_cancelled());
    assert!(
        first
            .complete(&response(&request, b"x"), b"x".to_vec())
            .is_err()
    );
    let request = second.dispatch().unwrap();
    let read = second
        .complete(&response(&request, b"x"), b"x".to_vec())
        .unwrap();
    let _third = session.begin(&source()).unwrap();
    assert!(read.cache_condition().is_err());
}
#[test]
fn cancelled_and_committed_handles_do_not_exhaust_request_capacity_when_retained() {
    let mut configuration = config();
    let base = configuration.feeds.subscriptions[0].clone();
    configuration.feeds.subscriptions.clear();
    for n in 0..=MAX_PENDING_FEED_READS {
        let mut source = base.clone();
        source.id = format!("b41ef5e8-0e06-43e0-aa47-{n:012x}");
        configuration.feeds.subscriptions.push(source);
    }
    let session =
        FeedSourceSession::new(configuration.clone(), vec![descriptor("reader.public")]).unwrap();
    let mut retained = vec![];
    for (index, source) in configuration.feeds.subscriptions.iter().enumerate() {
        session.activate(&source.id, "reader.public").unwrap();
        let attempt = session.begin(&source.id).unwrap();
        let request = attempt.dispatch().unwrap();
        let read = attempt
            .complete(&response(&request, b"x"), b"x".to_vec())
            .unwrap();
        if index % 2 == 0 {
            assert!(read.cancellation().cancel());
        } else {
            read.with_current_cache_commit(None, |_| Ok(())).unwrap();
        }
        retained.push(read);
    }
    assert_eq!(retained.len(), MAX_PENDING_FEED_READS + 1);
}

struct UnguardedCache;
impl FeedCacheStore for UnguardedCache {
    fn compare_and_store<'a>(&'a self, _candidate: &'a FeedSourceRead) -> FeedFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}
#[test]
fn storage_callback_cannot_claim_success_without_guarded_final_commit() {
    let session = mk_session();
    let attempt = mk_attempt(&session);
    let request = attempt.dispatch().unwrap();
    let read = attempt
        .complete(&response(&request, b"x"), b"x".to_vec())
        .unwrap();
    let mut future = Box::pin(read.store(&UnguardedCache));
    assert!(matches!(poll(future.as_mut()), Poll::Ready(Err(_))));
    assert!(read.cache_condition().is_ok());
}

fn conditional_hint() -> FeedConditionalHint {
    FeedConditionalHint::parse(&serde_json::to_vec(&json!({"schema":"booknook.feed-conditional-hint.v1","source_id":source(),"provider_id":"reader.public","url":config().feeds.subscriptions[0].url,"body_sha256":booknook_core::digest(b"<rss/>"),"validators":{"etag":"\"v1\"","last_modified":null}})).unwrap()).unwrap()
}
#[test]
fn conditional_not_modified_requires_issued_exact_hint_and_matching_cached_bytes_at_commit() {
    let session = mk_session();
    session.activate(&source(), "reader.public").unwrap();
    let attempt = session
        .begin_with_conditional(&source(), Some(conditional_hint()))
        .unwrap();
    let request = attempt.dispatch().unwrap();
    let response = FeedSourceResponse::not_modified(
        &request,
        &request.url,
        FeedValidators {
            etag: Some("\"v2\"".into()),
            last_modified: None,
        },
    )
    .unwrap();
    let read = attempt
        .complete(&serde_json::to_vec(&response).unwrap(), vec![])
        .unwrap();
    assert!(read.body().is_empty());
    assert_eq!(
        read.cache_condition().unwrap().expected_body_sha256,
        Some(booknook_core::digest(b"<rss/>"))
    );
    let calls = AtomicUsize::new(0);
    assert!(
        read.with_current_cache_commit(None, |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .is_err()
    );
    assert!(
        read.with_current_cache_commit(Some(b"wrong"), |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    read.with_current_cache_commit(Some(b"<rss/>"), |condition| {
        assert_eq!(condition.outcome, FeedSourceOutcome::NotModified);
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let attempt = session.begin(&source()).unwrap();
    let request = attempt.dispatch().unwrap();
    assert!(
        FeedSourceResponse::not_modified(&request, &request.url, FeedValidators::empty()).is_err()
    );
}
#[test]
fn conditional_hints_header_injection_mismatched_binding_and_late_304s_fail_closed() {
    for field in ["source_id", "provider_id", "url", "body_sha256"] {
        let mut value = serde_json::to_value(conditional_hint()).unwrap();
        value[field] = json!("wrong");
        let result = FeedConditionalHint::parse(&serde_json::to_vec(&value).unwrap());
        let session = mk_session();
        session.activate(&source(), "reader.public").unwrap();
        assert!(
            result
                .and_then(|hint| session.begin_with_conditional(&source(), Some(hint)))
                .is_err()
        );
    }
    for header in [
        "\"okay\"\r\nAuthorization: x",
        "W/\"broken",
        "x",
        "\"a\"x",
        "\"é\"",
    ] {
        let mut hint = serde_json::to_value(conditional_hint()).unwrap();
        hint["validators"]["etag"] = json!(header);
        assert!(FeedConditionalHint::parse(&serde_json::to_vec(&hint).unwrap()).is_err());
    }
    for date in [
        "Wed, 21 Oct 2015 07:28:00 GMT\r\nX: y",
        "Wed, 31 Feb 2024 07:28:00 GMT",
        "Wed, 21 Oct 2015 27:28:00 GMT",
    ] {
        let mut hint = serde_json::to_value(conditional_hint()).unwrap();
        hint["validators"]["last_modified"] = json!(date);
        assert!(FeedConditionalHint::parse(&serde_json::to_vec(&hint).unwrap()).is_err());
    }
    for change in 0..4 {
        let session = mk_session();
        session.activate(&source(), "reader.public").unwrap();
        let attempt = session
            .begin_with_conditional(&source(), Some(conditional_hint()))
            .unwrap();
        let request = attempt.dispatch().unwrap();
        let response =
            FeedSourceResponse::not_modified(&request, &request.url, FeedValidators::empty())
                .unwrap();
        match change {
            0 => session.replace_configuration(config()).unwrap(),
            1 => session
                .replace_providers(vec![descriptor("reader.public")])
                .unwrap(),
            2 => {
                attempt.cancel();
            }
            _ => session.disable(&source()).unwrap(),
        }
        assert!(
            attempt
                .complete(&serde_json::to_vec(&response).unwrap(), vec![])
                .is_err()
        );
    }
    let session = mk_session();
    session.activate(&source(), "reader.public").unwrap();
    let attempt = session
        .begin_with_conditional(&source(), Some(conditional_hint()))
        .unwrap();
    let request = attempt.dispatch().unwrap();
    assert!(
        FeedSourceResponse::not_modified(
            &request,
            "https://other.example.org/feed",
            FeedValidators::empty()
        )
        .is_err()
    );
    let mut response =
        FeedSourceResponse::not_modified(&request, &request.url, FeedValidators::empty()).unwrap();
    response.body_sha256 = booknook_core::digest(b"wrong");
    assert!(
        attempt
            .complete(&serde_json::to_vec(&response).unwrap(), vec![])
            .is_err()
    );
}

#[test]
fn rejected_conditional_request_cannot_cancel_a_valid_existing_candidate() {
    let session = mk_session();
    let attempt = mk_attempt(&session);
    let request = attempt.dispatch().unwrap();
    let read = attempt
        .complete(&response(&request, b"x"), b"x".to_vec())
        .unwrap();
    let mut hint = conditional_hint();
    hint.provider_id = "wrong-provider".into();
    assert!(
        session
            .begin_with_conditional(&source(), Some(hint))
            .is_err()
    );
    assert!(read.cache_condition().is_ok());
}

#[test]
fn cache_validators_preserve_omitted_304_values_and_do_not_cross_redirect_endpoints() {
    let session = mk_session();
    session.activate(&source(), "reader.public").unwrap();
    let attempt = session
        .begin_with_conditional(&source(), Some(conditional_hint()))
        .unwrap();
    let request = attempt.dispatch().unwrap();
    let response =
        FeedSourceResponse::not_modified(&request, &request.url, FeedValidators::empty()).unwrap();
    let read = attempt
        .complete(&serde_json::to_vec(&response).unwrap(), vec![])
        .unwrap();
    assert_eq!(
        read.cache_condition().unwrap().validators.etag,
        Some("\"v1\"".into())
    );
    let attempt = session.begin(&source()).unwrap();
    let request = attempt.dispatch().unwrap();
    let mut response = FeedSourceResponse::for_request(
        &request,
        "https://other.example.org/feed",
        "application/xml",
        b"x",
    )
    .unwrap();
    response.validators.etag = Some("\"other-endpoint\"".into());
    let read = attempt
        .complete(&serde_json::to_vec(&response).unwrap(), b"x".to_vec())
        .unwrap();
    assert_eq!(
        read.cache_condition().unwrap().validators,
        FeedValidators::empty()
    );
    assert!(read.response().validators.etag.is_some());
}
