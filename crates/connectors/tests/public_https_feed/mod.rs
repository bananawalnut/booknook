// SPDX-License-Identifier: MIT OR Apache-2.0
// Unit-module tests intentionally exercise private policy and compile-only fixtures.
use super::*;
use booknook_core::{
    feed_source::{FeedConditionalHint, FeedSourceAttempt, FeedSourceOutcome, FeedSourceSession},
    feeds::{FeedConfiguration, FeedSection},
};
use std::{
    collections::VecDeque,
    sync::{
        Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer},
};

const BODY: &[u8] = b"<?xml version='1.0' encoding='UTF-8'?><rss version='2.0'><channel><title>Public fixture</title></channel></rss>";

fn attempt(url: &str, conditional: bool) -> (FeedSourceSession, FeedSourceAttempt) {
    let section = FeedSection::parse(&serde_json::to_vec(&serde_json::json!({
        "schema":"castalia.feeds-section.v1","version":1,
        "subscriptions":[{"id":"b41ef5e8-0e06-43e0-aa47-19f382e00c08","url":url,"format":"auto","title":"","enabled":true,"collectionIds":[],"disclosure":"private"}],
        "collections":[],"preferences":{"order":"newest-first"}
    })).unwrap()).unwrap();
    let source = section.subscriptions[0].id.clone();
    let session = FeedSourceSession::new(
        FeedConfiguration::new(0, section).unwrap(),
        vec![PublicHttpsFeedProvider::new().descriptor().unwrap()],
    )
    .unwrap();
    session
        .activate(&source, PUBLIC_HTTPS_FEED_PROVIDER_ID)
        .unwrap();
    let hint = conditional.then(|| FeedConditionalHint {
        schema: "booknook.feed-conditional-hint.v1".into(),
        source_id: source.clone(),
        provider_id: PUBLIC_HTTPS_FEED_PROVIDER_ID.into(),
        url: url.into(),
        body_sha256: booknook_core::digest(BODY),
        validators: FeedValidators {
            etag: Some("\"known\"".into()),
            last_modified: Some("Wed, 21 Oct 2015 07:28:00 GMT".into()),
        },
    });
    let attempt = session.begin_with_conditional(&source, hint).unwrap();
    (session, attempt)
}

struct Reply {
    head: String,
    head_delay: Duration,
    chunks: Vec<(Duration, Vec<u8>)>,
}
impl Reply {
    fn new(status: u16, headers: &[(&str, &str)], body: &[u8]) -> Self {
        let mut head = format!("HTTP/1.1 {status} Fixture\r\nConnection: close\r\n");
        for (name, value) in headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        if !headers.iter().any(|(name, _)| {
            name.eq_ignore_ascii_case("content-length")
                || name.eq_ignore_ascii_case("transfer-encoding")
        }) {
            head.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        head.push_str("\r\n");
        Self {
            head,
            head_delay: Duration::ZERO,
            chunks: vec![(Duration::ZERO, body.to_vec())],
        }
    }
    fn xml() -> Self {
        Self::new(
            200,
            &[
                ("Content-Type", "application/rss+xml; charset=utf-8"),
                ("ETag", "\"fresh\""),
            ],
            BODY,
        )
    }
}
struct Server {
    address: SocketAddr,
    root: reqwest::Certificate,
    requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn start(replies: Vec<Reply>) -> Self {
        let key = rcgen::generate_simple_self_signed(vec![
            "feeds.example.org".into(),
            "other.example.org".into(),
        ])
        .unwrap();
        let root = reqwest::Certificate::from_der(key.cert.der()).unwrap();
        let tls = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![key.cert.der().clone()],
                PrivatePkcs8KeyDer::from(key.signing_key.serialize_der()).into(),
            )
            .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(tls));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let replies = Arc::new(Mutex::new(VecDeque::from(replies)));
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let acceptor = acceptor.clone();
                let requests = recorded.clone();
                let replies = replies.clone();
                tokio::spawn(async move {
                    let Ok(mut stream) = acceptor.accept(stream).await else {
                        return;
                    };
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") && request.len() <= 8192 {
                        let mut byte = [0u8; 1];
                        if stream.read_exact(&mut byte).await.is_err() {
                            return;
                        }
                        request.push(byte[0]);
                    }
                    requests
                        .lock()
                        .unwrap()
                        .push(String::from_utf8(request).unwrap());
                    let Some(reply) = replies.lock().unwrap().pop_front() else {
                        return;
                    };
                    tokio::time::sleep(reply.head_delay).await;
                    if stream.write_all(reply.head.as_bytes()).await.is_err() {
                        return;
                    }
                    for (delay, chunk) in reply.chunks {
                        tokio::time::sleep(delay).await;
                        if stream.write_all(&chunk).await.is_err() {
                            return;
                        }
                    }
                    let _ = stream.shutdown().await;
                });
            }
        });
        Self {
            address,
            root,
            requests,
            task,
        }
    }
    fn provider(&self) -> PublicHttpsFeedProvider {
        PublicHttpsFeedProvider {
            test_transport: Some(TestTransport {
                addresses: vec![self.address],
                root: Some(self.root.clone()),
                timeout: None,
                slots: Arc::new(Semaphore::new(4)),
            }),
        }
    }
    async fn wait_requests(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while self.requests.lock().unwrap().len() < count {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }
}

#[test]
fn public_address_policy_is_conservative_across_both_families() {
    for raw in [
        "8.8.8.8",
        "1.1.1.1",
        "93.184.216.34",
        "2606:4700:4700::1111",
        "2001:4860:4860::8888",
    ] {
        assert!(is_public_address(raw.parse().unwrap()), "{raw}");
    }
    for raw in [
        "0.0.0.0",
        "0.1.2.3",
        "10.0.0.1",
        "100.64.0.1",
        "100.127.255.255",
        "127.0.0.1",
        "169.254.169.254",
        "172.16.0.1",
        "172.31.255.255",
        "192.0.0.9",
        "192.0.2.1",
        "192.88.99.1",
        "192.168.1.1",
        "198.18.0.1",
        "198.19.255.255",
        "198.51.100.1",
        "203.0.113.1",
        "224.0.0.1",
        "240.0.0.1",
        "255.255.255.255",
        "::",
        "::1",
        "::ffff:8.8.8.8",
        "::ffff:127.0.0.1",
        "64:ff9b::808:808",
        "64:ff9b::7f00:1",
        "64:ff9b:1::1",
        "100::1",
        "100:0:0:1::1",
        "2001::1",
        "2001:2::1",
        "2001:db8::1",
        "2002:7f00:1::",
        "3fff::1",
        "fc00::1",
        "fe80::1",
        "ff02::1",
    ] {
        assert!(!is_public_address(raw.parse().unwrap()), "{raw}");
    }
}

#[test]
fn redirects_support_relative_paths_without_normalizing_away_forbidden_authority() {
    let base = reqwest::Url::parse("https://feeds.example.org/a/feed.xml").unwrap();
    for (raw, expected) in [
        ("next.xml", "https://feeds.example.org/a/next.xml"),
        ("../next.xml", "https://feeds.example.org/next.xml"),
        ("/next.xml", "https://feeds.example.org/next.xml"),
        (
            "//other.example.org/next.xml",
            "https://other.example.org/next.xml",
        ),
        (
            "https://other.example.org/next.xml",
            "https://other.example.org/next.xml",
        ),
    ] {
        assert_eq!(redirect_endpoint(&base, raw).unwrap().as_str(), expected);
    }
    for raw in [
        "",
        "https://feeds.example.org:443/x",
        "//feeds.example.org:443/x",
        "https://FEEDS.example.org/x",
        "//FEEDS.example.org/x",
        "https://user@feeds.example.org/x",
        "//user@feeds.example.org/x",
        "https://feeds.example.org/x?",
        "/x?q=1",
        "#x",
        "/x#",
        "http://feeds.example.org/x",
        "https://127.0.0.1/x",
        "//127.0.0.1/x",
        "//feeds.local/x",
        "https://feeds.example.org/a/../x",
        "https://feeds.example.org/%",
        "/%zz",
        " /x",
        "/x\\y",
        "///feeds.example.org/x",
    ] {
        assert!(redirect_endpoint(&base, raw).is_err(), "{raw:?}");
    }
}

#[test]
fn public_endpoint_wrapper_preserves_existing_exact_policy() {
    assert!(validate_public_feed_endpoint("https://feeds.example.org/feed.xml").is_ok());
    for raw in [
        "https://feeds.example.org:443/feed.xml",
        "https://FEEDS.example.org/feed.xml",
        "https://feeds.example.org/feed.xml?q=1",
        "https://127.0.0.1/feed.xml",
    ] {
        assert!(validate_public_feed_endpoint(raw).is_err(), "{raw}");
    }
}

#[test]
fn complete_dns_set_is_checked_before_deduplication_and_peer_is_exact() {
    let public: SocketAddr = "8.8.8.8:443".parse().unwrap();
    let private: SocketAddr = "127.0.0.1:443".parse().unwrap();
    assert!(checked_addresses(vec![]).is_err());
    assert!(checked_addresses(vec![public, private]).is_err());
    assert!(checked_addresses(vec![public, "[::ffff:8.8.8.8]:443".parse().unwrap()]).is_err());
    assert!(checked_addresses(vec![public; 33]).is_err());
    assert_eq!(checked_addresses(vec![public; 32]).unwrap(), vec![public]);
    assert!(checked_addresses(vec!["8.8.8.8:444".parse().unwrap()]).is_err());
    assert!(peer_matches(Some(public), &[public]));
    assert!(!peer_matches(None, &[public]));
    assert!(!peer_matches(Some(private), &[public]));
    assert!(!peer_matches(
        Some("8.8.8.8:80".parse().unwrap()),
        &[public]
    ));
}

#[test]
fn no_fallback_resolver_cannot_resolve_an_unpinned_name() {
    use reqwest::dns::Resolve;
    let runtime = tokio::runtime::Runtime::new().unwrap();
    assert!(
        runtime
            .block_on(NoFallbackDns.resolve("unreviewed.example.org".parse().unwrap()))
            .is_err()
    );
}

#[test]
fn encoding_refusal_uses_actual_utf8_and_declared_encoding() {
    assert!(validate_utf8(BODY).is_ok());
    assert!(validate_utf8(b"\xef\xbb\xbf<?xml version='1.0' encoding='utf-8'?><rss/>").is_ok());
    for bytes in [
        b"\xff<rss/>".as_slice(),
        b"\xff\xfe<\0r\0s\0s\0/\0>\0",
        b"<?xml version='1.0' encoding = 'ISO-8859-1'?><rss/>",
        b"<?xml\nversion='1.0' encoding=\"UTF-16\"?><rss/>",
    ] {
        assert!(validate_utf8(bytes).is_err());
    }
}

#[tokio::test]
async fn real_tls_acquisition_returns_exact_raw_bytes_and_bound_metadata() {
    let server = Server::start(vec![Reply::xml()]).await;
    let provider = server.provider();
    let (_session, attempt) = attempt("https://feeds.example.org/feed.xml", false);
    let read = attempt.execute(&provider).await.unwrap();
    assert_eq!(read.body(), BODY);
    assert_eq!(read.response().body_sha256, booknook_core::digest(BODY));
    assert_eq!(
        read.response().validators.etag.as_deref(),
        Some("\"fresh\"")
    );
    assert_eq!(
        read.response().final_url,
        "https://feeds.example.org/feed.xml"
    );
    let request = server.requests.lock().unwrap()[0].to_ascii_lowercase();
    assert!(request.starts_with("get /feed.xml http/1.1\r\n"));
    assert!(request.contains("host: feeds.example.org\r\n"));
    assert!(request.contains("accept-encoding: identity\r\n"));
    assert!(request.contains("user-agent: booknook-publicfeed/1\r\n"));
    for header in [
        "authorization:",
        "proxy-authorization:",
        "cookie:",
        "referer:",
    ] {
        assert!(!request.contains(header));
    }
}

#[tokio::test]
async fn real_tls_certificate_and_hostname_verification_remain_required() {
    let server = Server::start(vec![Reply::xml()]).await;
    let mut provider = server.provider();
    provider.test_transport.as_mut().unwrap().root = None;
    let (_session, first) = attempt("https://feeds.example.org/feed.xml", false);
    assert!(first.execute(&provider).await.is_err());
    let provider = server.provider();
    let (_session, second) = attempt("https://wrong.example.org/feed.xml", false);
    assert!(second.execute(&provider).await.is_err());
    assert!(server.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn real_redirect_drops_conditionals_and_cookies_and_keeps_metadata_bound() {
    let server = Server::start(vec![
        Reply::new(
            302,
            &[
                ("Location", "//other.example.org/next.xml"),
                ("Set-Cookie", "session=never-send"),
                ("ETag", "\"wrong-hop\""),
            ],
            &[],
        ),
        Reply::xml(),
    ])
    .await;
    let (_session, attempt) = attempt("https://feeds.example.org/feed.xml", true);
    let read = attempt.execute(&server.provider()).await.unwrap();
    assert_eq!(
        read.response().final_url,
        "https://other.example.org/next.xml"
    );
    let requests = server.requests.lock().unwrap();
    assert!(
        requests[0]
            .to_ascii_lowercase()
            .contains("if-none-match: \"known\"")
    );
    assert!(
        requests[0]
            .to_ascii_lowercase()
            .contains("if-modified-since:")
    );
    let second = requests[1].to_ascii_lowercase();
    for header in [
        "if-none-match:",
        "if-modified-since:",
        "cookie:",
        "authorization:",
        "proxy-authorization:",
        "referer:",
    ] {
        assert!(!second.contains(header), "{header}");
    }
}

#[tokio::test]
async fn real_304_requires_original_conditional_and_no_redirect() {
    let server = Server::start(vec![Reply::new(304, &[("ETag", "\"known\"")], &[])]).await;
    let (_session, conditional) = attempt("https://feeds.example.org/feed.xml", true);
    let read = conditional.execute(&server.provider()).await.unwrap();
    assert_eq!(read.response().outcome, FeedSourceOutcome::NotModified);
    assert!(read.body().is_empty());
    assert_eq!(read.response().body_sha256, booknook_core::digest(BODY));
    let server = Server::start(vec![Reply::new(304, &[], &[])]).await;
    let (_session, unsolicited) = attempt("https://feeds.example.org/feed.xml", false);
    assert!(unsolicited.execute(&server.provider()).await.is_err());
    let server = Server::start(vec![
        Reply::new(302, &[("Location", "/next.xml")], &[]),
        Reply::new(304, &[], &[]),
    ])
    .await;
    let (_session, redirected) = attempt("https://feeds.example.org/feed.xml", true);
    assert!(redirected.execute(&server.provider()).await.is_err());
}

#[tokio::test]
async fn real_redirects_refuse_unsafe_raw_targets_duplicate_location_and_loops() {
    for locations in [
        vec!["https://feeds.example.org:443/x"],
        vec!["//feeds.example.org:443/x"],
        vec!["http://feeds.example.org/x"],
        vec!["https://127.0.0.1/x"],
        vec!["//feeds.local/x"],
        vec!["/x?q=1"],
        vec!["/x", "/y"],
        vec!["/feed.xml"],
    ] {
        let headers: Vec<_> = locations.iter().map(|value| ("Location", *value)).collect();
        let server = Server::start(vec![Reply::new(302, &headers, &[]), Reply::xml()]).await;
        let (_session, attempt) = attempt("https://feeds.example.org/feed.xml", false);
        assert!(
            attempt.execute(&server.provider()).await.is_err(),
            "{locations:?}"
        );
        assert_eq!(server.requests.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn real_redirect_limit_allows_three_and_refuses_four() {
    for redirects in [3, 4] {
        let mut replies: Vec<_> = (0..redirects)
            .map(|i| Reply::new(302, &[("Location", &format!("/hop{i}.xml"))], &[]))
            .collect();
        replies.push(Reply::xml());
        let server = Server::start(replies).await;
        let (_session, attempt) = attempt("https://feeds.example.org/feed.xml", false);
        assert_eq!(
            attempt.execute(&server.provider()).await.is_ok(),
            redirects == 3
        );
        assert_eq!(server.requests.lock().unwrap().len(), 4);
    }
}

#[tokio::test]
async fn real_body_limits_cover_chunked_and_content_length_without_lossy_decode() {
    let good = vec![b'x'; MAX_FEED_BODY_BYTES];
    let mut chunked = format!("{:x}\r\n", good.len()).into_bytes();
    chunked.extend(&good);
    chunked.extend(b"\r\n0\r\n\r\n");
    let mut oversized = format!("{:x}\r\n", good.len() + 1).into_bytes();
    oversized.extend(&good);
    oversized.push(b'x');
    oversized.extend(b"\r\n0\r\n\r\n");
    for (reply, allowed) in [
        (
            Reply::new(200, &[("Content-Type", "application/xml")], &good),
            true,
        ),
        (
            Reply::new(
                200,
                &[
                    ("Content-Type", "application/xml"),
                    ("Transfer-Encoding", "chunked"),
                ],
                &chunked,
            ),
            true,
        ),
        (
            Reply::new(
                200,
                &[
                    ("Content-Type", "application/xml"),
                    ("Transfer-Encoding", "chunked"),
                ],
                &oversized,
            ),
            false,
        ),
        (
            Reply::new(
                200,
                &[
                    ("Content-Type", "application/xml"),
                    ("Content-Length", "2097153"),
                ],
                &[],
            ),
            false,
        ),
        (
            Reply::new(200, &[("Content-Type", "application/xml")], &[]),
            false,
        ),
        (
            Reply::new(200, &[("Content-Type", "application/xml")], b"\xff"),
            false,
        ),
    ] {
        let server = Server::start(vec![reply]).await;
        let (_session, attempt) = attempt("https://feeds.example.org/feed.xml", false);
        assert_eq!(attempt.execute(&server.provider()).await.is_ok(), allowed);
    }
}

#[tokio::test]
async fn real_encoding_media_type_validator_and_status_failures_do_not_yield_body() {
    for reply in [
        Reply::new(
            200,
            &[
                ("Content-Type", "application/xml"),
                ("Content-Encoding", "gzip"),
            ],
            BODY,
        ),
        Reply::new(
            200,
            &[("Content-Type", "application/xml;charset=ISO-8859-1")],
            BODY,
        ),
        Reply::new(200, &[("Content-Type", "text/html")], BODY),
        Reply::new(200, &[], BODY),
        Reply::new(
            200,
            &[("Content-Type", "application/xml"), ("ETag", "not-quoted")],
            BODY,
        ),
        Reply::new(
            200,
            &[
                ("Content-Type", "application/xml"),
                ("ETag", "\"one\""),
                ("ETag", "\"two\""),
            ],
            BODY,
        ),
        Reply::new(
            200,
            &[
                ("Content-Type", "application/xml"),
                ("Last-Modified", "yesterday"),
            ],
            BODY,
        ),
        Reply::new(
            200,
            &[("Content-Type", "application/xml")],
            b"<?xml version='1.0' encoding='UTF-16'?><rss/>",
        ),
        Reply::new(401, &[("WWW-Authenticate", "Basic realm=fixture")], BODY),
        Reply::new(407, &[("Proxy-Authenticate", "Basic realm=fixture")], BODY),
        Reply::new(206, &[("Content-Type", "application/xml")], BODY),
    ] {
        let server = Server::start(vec![reply]).await;
        let (_session, attempt) = attempt("https://feeds.example.org/feed.xml", false);
        assert!(attempt.execute(&server.provider()).await.is_err());
    }
}

#[tokio::test]
async fn malformed_charset_quotes_are_refused_before_body_admission() {
    for value in [
        "application/xml; charset=\"utf-8",
        "application/xml; charset=\"\"utf-8\"\"",
    ] {
        let server = Server::start(vec![Reply::new(200, &[("Content-Type", value)], BODY)]).await;
        let (_session, attempt) = attempt("https://feeds.example.org/feed.xml", false);
        assert!(
            attempt.execute(&server.provider()).await.is_err(),
            "{value}"
        );
    }
}

#[tokio::test]
async fn real_body_cancellation_and_total_deadline_refuse_late_bytes() {
    let mut reply = Reply::xml();
    reply.chunks[0].0 = Duration::from_millis(700);
    let server = Server::start(vec![reply]).await;
    let provider = server.provider();
    let (_session, attempt) = attempt("https://feeds.example.org/feed.xml", false);
    let execution = attempt.execute(&provider);
    tokio::pin!(execution);
    tokio::select! { result = &mut execution => panic!("completed before cancellation: {}", result.is_ok()), _ = server.wait_requests(1) => {} }
    attempt.cancel();
    assert!(
        tokio::time::timeout(Duration::from_millis(200), execution)
            .await
            .unwrap()
            .is_err()
    );
    let mut reply = Reply::xml();
    reply.chunks[0].0 = Duration::from_millis(700);
    let server = Server::start(vec![reply]).await;
    let mut provider = server.provider();
    provider.test_transport.as_mut().unwrap().timeout = Some(Duration::from_millis(150));
    let (_session, attempt) = self::attempt("https://feeds.example.org/feed.xml", false);
    assert!(
        attempt
            .execute(&provider)
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("deadline")
    );
}

#[tokio::test]
async fn queued_acquisition_is_bounded_and_cancelled_without_a_connection() {
    let server = Server::start(vec![Reply::xml()]).await;
    let mut provider = server.provider();
    provider.test_transport.as_mut().unwrap().timeout = Some(Duration::from_millis(100));
    let permits = provider.slots().acquire_many_owned(4).await.unwrap();
    let (_session, first) = attempt("https://feeds.example.org/feed.xml", false);
    assert!(
        first
            .execute(&provider)
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("deadline")
    );
    let (_session, second) = attempt("https://feeds.example.org/feed.xml", false);
    let execution = second.execute(&provider);
    tokio::pin!(execution);
    tokio::select! { result = &mut execution => panic!("queue completed unexpectedly: {}", result.is_ok()), _ = tokio::time::sleep(Duration::from_millis(20)) => {} }
    second.cancel();
    assert!(
        tokio::time::timeout(Duration::from_millis(70), execution)
            .await
            .unwrap()
            .is_err()
    );
    assert!(server.requests.lock().unwrap().is_empty());
    drop(permits);
    assert_eq!(provider.slots().available_permits(), 4);
}

#[tokio::test]
async fn total_deadline_is_not_reset_by_successful_redirect_hops() {
    let mut first = Reply::new(302, &[("Location", "/one.xml")], &[]);
    first.head_delay = Duration::from_millis(100);
    let mut second = Reply::new(302, &[("Location", "/two.xml")], &[]);
    second.head_delay = Duration::from_millis(100);
    let mut third = Reply::xml();
    third.head_delay = Duration::from_millis(100);
    let server = Server::start(vec![first, second, third]).await;
    let mut provider = server.provider();
    provider.test_transport.as_mut().unwrap().timeout = Some(Duration::from_millis(250));
    let (_session, attempt) = attempt("https://feeds.example.org/feed.xml", false);
    assert!(
        attempt
            .execute(&provider)
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("deadline")
    );
    assert_eq!(server.requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn cancellation_during_real_tls_handshake_does_not_wait_for_connect_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut provider = PublicHttpsFeedProvider::new();
    provider.test_transport = Some(TestTransport {
        addresses: vec![address],
        root: None,
        timeout: None,
        slots: Arc::new(Semaphore::new(4)),
    });
    let (_session, attempt) = attempt("https://feeds.example.org/feed.xml", false);
    let execution = attempt.execute(&provider);
    tokio::pin!(execution);
    let stream = tokio::select! { result = &mut execution => panic!("handshake unexpectedly completed: {}", result.is_ok()), accepted = listener.accept() => accepted.unwrap().0 };
    attempt.cancel();
    assert!(
        tokio::time::timeout(Duration::from_millis(200), execution)
            .await
            .unwrap()
            .is_err()
    );
    drop(stream);
}

#[tokio::test]
async fn cancelled_blocking_resolvers_keep_their_permits_until_actual_completion() {
    let slots = Arc::new(Semaphore::new(2));
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    struct Release(Arc<(Mutex<bool>, Condvar)>);
    impl Drop for Release {
        fn drop(&mut self) {
            *self.0.0.lock().unwrap() = true;
            self.0.1.notify_all();
        }
    }
    let release = Release(gate.clone());
    let entered = Arc::new(AtomicUsize::new(0));
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let gate = gate.clone();
        let entered = entered.clone();
        let slots = slots.clone();
        tasks.push(tokio::spawn(blocking_resolution(slots, move || {
            entered.fetch_add(1, Ordering::SeqCst);
            let (lock, wake) = &*gate;
            let mut ready = lock.lock().unwrap();
            while !*ready {
                ready = wake.wait(ready).unwrap();
            }
            Ok(vec!["8.8.8.8:443".parse().unwrap()])
        })));
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        while entered.load(Ordering::SeqCst) != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    for task in tasks {
        task.abort();
        let _ = task.await;
    }
    assert_eq!(slots.available_permits(), 0);
    let calls = entered.clone();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(50),
            blocking_resolution(slots.clone(), move || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(vec![])
            })
        )
        .await
        .is_err()
    );
    assert_eq!(entered.load(Ordering::SeqCst), 2);
    drop(release);
    tokio::time::timeout(Duration::from_secs(2), async {
        while slots.available_permits() != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn proxy_environment_is_ignored_by_real_tls_requests() {
    if std::env::var_os("BOOKNOOK_PROXY_FIXTURE_CHILD").is_none() {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "public_https_feed::tests::proxy_environment_is_ignored_by_real_tls_requests",
            ])
            .env("BOOKNOOK_PROXY_FIXTURE_CHILD", "1");
        for name in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            command.env(name, "http://127.0.0.1:9");
        }
        command.env_remove("NO_PROXY").env_remove("no_proxy");
        let result = command.output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        assert!(String::from_utf8_lossy(&result.stdout).contains("1 passed"));
        return;
    }
    let server = Server::start(vec![Reply::xml()]).await;
    let (_session, attempt) = attempt("https://feeds.example.org/feed.xml", false);
    assert!(attempt.execute(&server.provider()).await.is_ok());
}

#[tokio::test]
#[ignore = "Opt-in live read-only public HTTPS qualification; set BOOKNOOK_FEED_ACCEPTANCE_URL to a controlled/public XML endpoint"]
async fn public_https_acceptance_uses_production_dns_tls_and_no_test_override() {
    let url =
        std::env::var("BOOKNOOK_FEED_ACCEPTANCE_URL").expect("explicit public endpoint required");
    let provider = PublicHttpsFeedProvider::new();
    assert!(provider.test_transport.is_none());
    let (_session, attempt) = attempt(&url, false);
    let read = attempt.execute(&provider).await.unwrap();
    assert!(!read.body().is_empty());
    assert!(read.body().len() <= MAX_FEED_BODY_BYTES);
    assert_eq!(
        read.response().body_sha256,
        booknook_core::digest(read.body())
    );
    println!(
        "production public HTTPS: {} bytes, sha256 {}, final {}",
        read.body().len(),
        read.response().body_sha256,
        read.response().final_url
    );
}
