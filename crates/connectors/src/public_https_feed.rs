// SPDX-License-Identifier: MIT OR Apache-2.0
//! Explicitly installed credential-free native public HTTPS feed acquisition.
//!
//! This adapter returns bounded UNTRUSTED XML bytes. It does not normalize XML,
//! activate a source, retain cookies/validators, or write a cache. Install it only
//! through the source session and perform separately qualified XML/cache admission.
use booknook_core::{
    Error, Result,
    feed_source::{
        FeedCancellation, FeedFuture, FeedSourceDescriptor, FeedSourceProtocol, FeedSourceProvider,
        FeedSourceReply, FeedSourceRequest, FeedSourceResponse, FeedValidators,
        MAX_FEED_BODY_BYTES, MAX_SOURCE_METADATA_BYTES,
    },
    feeds::validate_public_feed_endpoint,
};
use reqwest::{Client, Url, header};
use std::{
    collections::BTreeSet,
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::sync::Semaphore;

pub const PUBLIC_HTTPS_FEED_PROVIDER_ID: &str = "booknook.public-https.v1";
const USER_AGENT: &str = "BookNook-PublicFeed/1";
const MAX_DNS_ADDRESSES: usize = 32;
const MAX_REDIRECTS: usize = 3;
const TOTAL_TIMEOUT: Duration = Duration::from_secs(20);
const IO_TIMEOUT: Duration = Duration::from_secs(5);

fn invalid(message: &str) -> Error {
    Error::Invalid(message.into())
}
fn unavailable(message: &str) -> Error {
    Error::Unavailable(message.into())
}
fn request_slots() -> Arc<Semaphore> {
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SLOTS.get_or_init(|| Arc::new(Semaphore::new(4))).clone()
}
fn resolver_slots() -> Arc<Semaphore> {
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SLOTS.get_or_init(|| Arc::new(Semaphore::new(4))).clone()
}

/// Conservative IANA profile, reviewed 2026-11-14 against:
/// https://www.iana.org/assignments/iana-ipv4-special-registry/
/// https://www.iana.org/assignments/iana-ipv6-special-registry/
/// https://www.iana.org/assignments/ipv6-unicast-address-assignments/
/// All special-purpose entries are refused, including globally reachable anycast
/// exceptions. IPv6 additionally requires a currently allocated RIR prefix;
/// future allocations need review. Mapped/translation/transition forms are refused.
fn is_public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            let value = u32::from(address);
            ![
                (0x0000_0000, 8),
                (0x0a00_0000, 8),
                (0x6440_0000, 10),
                (0x7f00_0000, 8),
                (0xa9fe_0000, 16),
                (0xac10_0000, 12),
                (0xc000_0000, 24),
                (0xc000_0200, 24),
                (0xc01f_c400, 24),
                (0xc034_c100, 24),
                (0xc058_6300, 24),
                (0xc0a8_0000, 16),
                (0xc0af_3000, 24),
                (0xc612_0000, 15),
                (0xc633_6400, 24),
                (0xcb00_7100, 24),
                (0xe000_0000, 4),
                (0xf000_0000, 4),
            ]
            .iter()
            .any(|(prefix, bits)| value >> (32 - bits) == prefix >> (32 - bits))
        }
        IpAddr::V6(address) => {
            let value = u128::from(address);
            let in_prefix = |prefix: u32, bits: u32| {
                value >> (128 - bits) == (u128::from(prefix) << 96) >> (128 - bits)
            };
            // Documentation and special-purpose AS112 within RIR allocations.
            if in_prefix(0x2001_0db8, 32) || value >> 80 == 0x2620_004f_8000 {
                return false;
            }
            [
                (0x2001_0200, 23),
                (0x2001_0400, 22),
                (0x2001_0800, 21),
                (0x2001_1200, 23),
                (0x2001_1400, 22),
                (0x2001_1800, 21),
                (0x2001_2000, 19),
                (0x2001_4000, 21),
                (0x2001_4800, 22),
                (0x2001_4c00, 23),
                (0x2001_5000, 20),
                (0x2001_8000, 19),
                (0x2001_a000, 20),
                (0x2001_b000, 20),
                (0x2003_0000, 18),
                (0x2400_0000, 12),
                (0x2410_0000, 12),
                (0x2600_0000, 12),
                (0x2610_0000, 23),
                (0x2620_0000, 23),
                (0x2630_0000, 12),
                (0x2800_0000, 12),
                (0x2a00_0000, 12),
                (0x2a10_0000, 12),
                (0x2c00_0000, 12),
            ]
            .iter()
            .any(|(prefix, bits)| in_prefix(*prefix, *bits))
        }
    }
}

fn checked_addresses(addresses: Vec<SocketAddr>) -> Result<Vec<SocketAddr>> {
    if addresses.is_empty()
        || addresses.len() > MAX_DNS_ADDRESSES
        || addresses
            .iter()
            .any(|address| address.port() != 443 || !is_public_address(address.ip()))
    {
        return Err(invalid(
            "Feed DNS must return 1..32 exclusively public native IP addresses on 443",
        ));
    }
    // Validate the entire answer set BEFORE deduplicating; never discard unsafe answers.
    Ok(addresses
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect())
}

async fn blocking_resolution<F>(slots: Arc<Semaphore>, resolve: F) -> Result<Vec<SocketAddr>>
where
    F: FnOnce() -> Result<Vec<SocketAddr>> + Send + 'static,
{
    let permit = slots
        .acquire_owned()
        .await
        .map_err(|_| unavailable("Feed DNS resolver is closed"))?;
    tokio::task::spawn_blocking(move || {
        // Cancellation/timeout drops the JoinHandle, not getaddrinfo. The actual
        // blocking job retains this permit until completion, bounding orphan jobs.
        let _permit = permit;
        resolve()
    })
    .await
    .map_err(|_| unavailable("Feed DNS resolver failed"))?
}

async fn resolve_public(host: String) -> Result<Vec<SocketAddr>> {
    let answers = tokio::time::timeout(
        IO_TIMEOUT,
        blocking_resolution(resolver_slots(), move || {
            let addresses = (host.as_str(), 443)
                .to_socket_addrs()
                .map_err(|_| unavailable("Feed DNS lookup failed"))?
                .take(MAX_DNS_ADDRESSES + 1)
                .collect();
            Ok(addresses)
        }),
    )
    .await
    .map_err(|_| unavailable("Feed DNS deadline exceeded"))??;
    checked_addresses(answers)
}

fn redirect_endpoint(base: &Url, location: &str) -> Result<Url> {
    if location.is_empty()
        || location.len() > 2048
        || location
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
        || location.contains(['\\', '?', '#'])
    {
        return Err(invalid("Feed redirect has forbidden raw URL syntax"));
    }
    // Validate absolute/network-path authority BEFORE URL parsing can erase :443,
    // uppercase DNS, userinfo or absolute dot segments. Relative path dot segments
    // are resolved according to URL rules, then the complete endpoint is checked.
    let target = if location.starts_with("//") {
        let absolute = format!("https:{location}");
        validate_public_feed_endpoint(&absolute)?;
        Url::parse(&absolute).map_err(|_| invalid("Invalid feed redirect"))?
    } else if Url::parse(location).is_ok() {
        validate_public_feed_endpoint(location)?;
        Url::parse(location).map_err(|_| invalid("Invalid feed redirect"))?
    } else {
        base.join(location)
            .map_err(|_| invalid("Invalid relative feed redirect"))?
    };
    validate_public_feed_endpoint(target.as_str())?;
    Ok(target)
}

fn one_header(
    headers: &header::HeaderMap,
    name: header::HeaderName,
    maximum: usize,
) -> Result<Option<String>> {
    let mut values = headers.get_all(name).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() || value.as_bytes().len() > maximum {
        return Err(invalid("Duplicate or oversized feed response header"));
    }
    let value = value
        .to_str()
        .map_err(|_| invalid("Feed response header is not ASCII"))?;
    if value.chars().any(char::is_control) {
        return Err(invalid("Control character in feed response header"));
    }
    Ok(Some(value.into()))
}

fn response_validators(headers: &header::HeaderMap) -> Result<FeedValidators> {
    let validators = FeedValidators {
        etag: one_header(headers, header::ETAG, 512)?,
        last_modified: one_header(headers, header::LAST_MODIFIED, 29)?,
    };
    validators.validate()?;
    Ok(validators)
}

fn content_type(headers: &header::HeaderMap) -> Result<String> {
    let value = one_header(headers, header::CONTENT_TYPE, 128)?
        .ok_or_else(|| invalid("Feed response requires an XML Content-Type"))?;
    let mut parts = value.split(';');
    let mime = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
    if ![
        "application/rss+xml",
        "application/atom+xml",
        "application/xml",
        "text/xml",
    ]
    .contains(&mime.as_str())
    {
        return Err(invalid("Feed response is not an admitted XML media type"));
    }
    let mut charset = false;
    for parameter in parts {
        let (name, encoding) = parameter
            .split_once('=')
            .ok_or_else(|| invalid("Malformed XML media type parameter"))?;
        if name.trim().eq_ignore_ascii_case("charset") {
            let encoding = encoding.trim();
            let encoding = if let Some(quoted) = encoding.strip_prefix('"') {
                quoted
                    .strip_suffix('"')
                    .ok_or_else(|| invalid("Unterminated XML charset quote"))?
            } else {
                encoding
            };
            if charset || !encoding.eq_ignore_ascii_case("utf-8") {
                return Err(invalid("Feed transport supports UTF-8 XML only"));
            }
            charset = true;
        }
    }
    Ok(value)
}

fn validate_utf8(body: &[u8]) -> Result<()> {
    let text =
        std::str::from_utf8(body).map_err(|_| invalid("Feed transport supports UTF-8 XML only"))?;
    if text.contains('\0') {
        return Err(invalid("NUL is not admissible XML text"));
    }
    let text = text.trim_start_matches('\u{feff}');
    if let Some(declaration) = text
        .strip_prefix("<?xml ")
        .or_else(|| text.strip_prefix("<?xml\t"))
        .or_else(|| text.strip_prefix("<?xml\n"))
        .or_else(|| text.strip_prefix("<?xml\r"))
    {
        let end = declaration
            .find("?>")
            .filter(|end| *end <= 1024)
            .ok_or_else(|| invalid("Unterminated or oversized XML declaration"))?;
        let mut declaration = &declaration[..end];
        while !declaration.trim().is_empty() {
            declaration = declaration.trim_start();
            let (name, rest) = declaration
                .split_once('=')
                .ok_or_else(|| invalid("Malformed XML declaration"))?;
            let rest = rest.trim_start();
            let quote = rest
                .chars()
                .next()
                .filter(|c| *c == '\'' || *c == '"')
                .ok_or_else(|| invalid("Malformed XML declaration value"))?;
            let (value, rest) = rest[1..]
                .split_once(quote)
                .ok_or_else(|| invalid("Malformed XML declaration value"))?;
            if name.trim() == "encoding" && !value.eq_ignore_ascii_case("utf-8") {
                return Err(invalid("Feed transport refuses non-UTF-8 XML declarations"));
            }
            declaration = rest;
        }
    }
    Ok(())
}

/// Internally constructed isolated transport. No application client, credentials,
/// custom roots, proxy, arbitrary headers or DNS bypass can be supplied publicly.
#[derive(Default)]
pub struct PublicHttpsFeedProvider {
    #[cfg(test)]
    test_transport: Option<TestTransport>,
}

#[cfg(test)]
#[derive(Clone)]
struct TestTransport {
    addresses: Vec<SocketAddr>,
    root: Option<reqwest::Certificate>,
    timeout: Option<Duration>,
    slots: Arc<Semaphore>,
}

impl PublicHttpsFeedProvider {
    pub fn new() -> Self {
        Self::default()
    }

    fn client(&self, host: &str, addresses: &[SocketAddr]) -> Result<Client> {
        let builder = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .referer(false)
            .retry(reqwest::retry::never())
            .https_only(true)
            .use_rustls_tls()
            // Clear every built-in source (including feature-unified native roots),
            // then opt back into the public WebPKI root set only.
            .tls_built_in_root_certs(false)
            .tls_built_in_webpki_certs(true)
            .http1_only()
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd()
            .connect_timeout(IO_TIMEOUT)
            .read_timeout(IO_TIMEOUT)
            .timeout(TOTAL_TIMEOUT)
            .user_agent(USER_AGENT)
            .resolve_to_addrs(host, addresses)
            // A missing override must fail closed rather than resolve another name.
            .dns_resolver(Arc::new(NoFallbackDns));
        #[cfg(test)]
        let builder = match self
            .test_transport
            .as_ref()
            .and_then(|test| test.root.clone())
        {
            Some(root) => builder.add_root_certificate(root),
            None => builder,
        };
        builder
            .build()
            .map_err(|_| unavailable("Cannot construct isolated feed TLS client"))
    }

    async fn addresses(&self, host: &str) -> Result<Vec<SocketAddr>> {
        #[cfg(test)]
        if let Some(test) = &self.test_transport {
            // Compile-only fixture override: production constructors cannot reach it.
            return Ok(test.addresses.clone());
        }
        resolve_public(host.into()).await
    }

    fn slots(&self) -> Arc<Semaphore> {
        #[cfg(test)]
        if let Some(test) = &self.test_transport {
            return test.slots.clone();
        }
        request_slots()
    }

    async fn acquire(&self, request: FeedSourceRequest) -> Result<FeedSourceReply> {
        if request.schema != "booknook.feed-source-request.v1"
            || request.provider_id != PUBLIC_HTTPS_FEED_PROVIDER_ID
            || serde_json::to_vec(&request)?.len() > MAX_SOURCE_METADATA_BYTES
        {
            return Err(invalid("Wrong provider, schema or oversized feed request"));
        }
        validate_public_feed_endpoint(&request.url)?;
        if let Some(hint) = &request.conditional {
            hint.validate()?;
            if hint.source_id != request.source_id
                || hint.provider_id != request.provider_id
                || hint.url != request.url
            {
                return Err(invalid("Conditional feed request binding mismatch"));
            }
        }
        let _permit = self
            .slots()
            .acquire_owned()
            .await
            .map_err(|_| unavailable("Feed transport is closed"))?;
        let mut url = Url::parse(&request.url).map_err(|_| invalid("Invalid feed endpoint"))?;
        let mut seen = BTreeSet::from([url.to_string()]);
        for hop in 0..=MAX_REDIRECTS {
            validate_public_feed_endpoint(url.as_str())?;
            let host = url
                .host_str()
                .ok_or_else(|| invalid("Feed hostname is absent"))?;
            let addresses = self.addresses(host).await?;
            let client = self.client(host, &addresses)?;
            let mut outgoing = client
                .get(url.clone())
                .header(header::ACCEPT_ENCODING, "identity")
                .header(
                    header::ACCEPT,
                    "application/rss+xml, application/atom+xml, application/xml, text/xml",
                );
            if hop == 0
                && let Some(hint) = &request.conditional
            {
                if let Some(etag) = &hint.validators.etag {
                    outgoing = outgoing.header(header::IF_NONE_MATCH, etag);
                }
                if let Some(date) = &hint.validators.last_modified {
                    outgoing = outgoing.header(header::IF_MODIFIED_SINCE, date);
                }
            }
            let mut response = outgoing
                .send()
                .await
                .map_err(|_| unavailable("Feed HTTPS request failed"))?;
            if response.url() != &url || !peer_matches(response.remote_addr(), &addresses) {
                return Err(invalid(
                    "Feed connection peer does not match the pinned DNS set",
                ));
            }
            let status = response.status();
            if matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308) {
                if hop == MAX_REDIRECTS {
                    return Err(invalid("Feed redirect limit exceeded"));
                }
                let location = one_header(response.headers(), header::LOCATION, 2048)?
                    .ok_or_else(|| invalid("Feed redirect is missing Location"))?;
                url = redirect_endpoint(&url, &location)?;
                if !seen.insert(url.to_string()) {
                    return Err(invalid("Feed redirect loop"));
                }
                continue;
            }
            let validators = response_validators(response.headers())?;
            if status == reqwest::StatusCode::NOT_MODIFIED {
                if hop != 0 {
                    return Err(invalid(
                        "Redirected feed 304 cannot satisfy the original cache binding",
                    ));
                }
                let metadata =
                    FeedSourceResponse::not_modified(&request, url.as_str(), validators)?;
                return Ok(FeedSourceReply {
                    response_bytes: serde_json::to_vec(&metadata)?,
                    body: vec![],
                });
            }
            if status != reqwest::StatusCode::OK {
                return Err(unavailable("Feed endpoint did not return 200 or bound 304"));
            }
            if one_header(response.headers(), header::CONTENT_ENCODING, 32)?
                .is_some_and(|v| !v.eq_ignore_ascii_case("identity"))
            {
                return Err(invalid(
                    "Compressed feed responses are outside the identity-encoding profile",
                ));
            }
            let content_type = content_type(response.headers())?;
            if response
                .content_length()
                .is_some_and(|length| length == 0 || length > MAX_FEED_BODY_BYTES as u64)
            {
                return Err(invalid("Feed body exceeds 2 MiB or is empty"));
            }
            let mut body = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| unavailable("Feed body read failed"))?
            {
                if chunk.len() > MAX_FEED_BODY_BYTES - body.len() {
                    return Err(invalid("Feed body exceeds 2 MiB"));
                }
                body.extend_from_slice(&chunk);
            }
            validate_utf8(&body)?;
            let mut metadata =
                FeedSourceResponse::for_request(&request, url.as_str(), &content_type, &body)?;
            metadata.validators = validators;
            return Ok(FeedSourceReply {
                response_bytes: serde_json::to_vec(&metadata)?,
                body,
            });
        }
        Err(invalid("Feed redirect limit exceeded"))
    }
}

fn peer_matches(peer: Option<SocketAddr>, pinned: &[SocketAddr]) -> bool {
    peer.is_some_and(|peer| pinned.contains(&peer))
}

struct NoFallbackDns;
impl reqwest::dns::Resolve for NoFallbackDns {
    fn resolve(&self, _name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async { Err(std::io::Error::other("Unpinned feed DNS lookup refused").into()) })
    }
}

impl FeedSourceProvider for PublicHttpsFeedProvider {
    fn descriptor(&self) -> Result<FeedSourceDescriptor> {
        Ok(FeedSourceDescriptor {
            schema: "booknook.feed-source-provider.v1".into(),
            version: 1,
            id: PUBLIC_HTTPS_FEED_PROVIDER_ID.into(),
            available: true,
            protocols: vec![FeedSourceProtocol::PublicXmlBytesV1],
        })
    }
    fn fetch<'a>(
        &'a self,
        request: FeedSourceRequest,
        cancellation: FeedCancellation,
    ) -> FeedFuture<'a, FeedSourceReply> {
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(Error::Conflict("Feed acquisition cancelled".into()));
            }
            let timeout = TOTAL_TIMEOUT;
            #[cfg(test)]
            let timeout = self
                .test_transport
                .as_ref()
                .and_then(|test| test.timeout)
                .unwrap_or(timeout);
            let result = tokio::select! {
                result = tokio::time::timeout(timeout, self.acquire(request)) =>
                    result.map_err(|_| unavailable("Feed total acquisition deadline exceeded"))?,
                _ = async {
                    loop {
                        if cancellation.is_cancelled() { break; }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                } => Err(Error::Conflict("Feed acquisition cancelled".into())),
            };
            if cancellation.is_cancelled() {
                return Err(Error::Conflict("Feed acquisition cancelled".into()));
            }
            result
        })
    }
}

#[cfg(test)]
#[path = "../tests/public_https_feed/mod.rs"]
mod tests;
