// SPDX-License-Identifier: MIT OR Apache-2.0
//! Inert, bounded RSS 2.0 / Atom normalization, independent of transport/cache.
//! The nonrecursive pull-reader preflight MUST precede roxmltree's recursive DOM.
//! Output strings are text only, never HTML or renderer instructions. Escaped HTML
//! is an explicitly tagged literal fallback, not a claimed HTML-to-text parser.
pub use crate::feeds::FeedFormat;
use crate::{Error, Result, digest, feeds, strict_parse};
use chrono::DateTime;
use quick_xml::{Reader, events::Event};
use roxmltree::Node;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const NORMALIZER_PROFILE: &str = "booknook.rss2-atom.inert.v1";
pub const MAX_XML_BYTES: usize = crate::feed_source::MAX_FEED_BODY_BYTES;
pub const MAX_XML_DEPTH: usize = 64;
pub const MAX_XML_EVENTS: usize = 50_000;
pub const MAX_XML_ATTRIBUTES: usize = 64;
pub const MAX_FEED_ENTRIES: usize = 500;
pub const MAX_TEXT_BYTES: usize = 65_536;
pub const MAX_NORMALIZED_BYTES: usize = 4 * 1024 * 1024;
const ATOM: &str = "http://www.w3.org/2005/Atom";
const XHTML: &str = "http://www.w3.org/1999/xhtml";
const XML: &str = "http://www.w3.org/XML/1998/namespace";
const CONTENT: &str = "http://purl.org/rss/1.0/modules/content/";
const DC: &str = "http://purl.org/dc/elements/1.1/";
fn invalid(message: &str) -> Error {
    Error::Invalid(message.into())
}
fn bound(value: &str, limit: usize) -> Result<()> {
    if value.len() > limit {
        Err(invalid("Feed string exceeds its byte limit"))
    } else {
        Ok(())
    }
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizationContext {
    pub schema: String,
    pub source_id: String,
    pub provider_id: String,
    pub requested_url: String,
    pub final_url: String,
    pub expected_format: FeedFormat,
}
impl NormalizationContext {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        if raw.len() > 16_384 {
            return Err(invalid("Feed normalization context exceeds 16 KiB"));
        }
        let context: Self = strict_parse(raw)?;
        if context.schema != "booknook.feed-normalization-context.v1"
            || !crate::feed_source::safe_id(&context.provider_id)
        {
            return Err(invalid("Invalid normalization context"));
        }
        feeds::uuid(&context.source_id)?;
        feeds::endpoint(&context.requested_url)?;
        feeds::endpoint(&context.final_url)?;
        Ok(context)
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct FeedProvenance {
    pub source_id: String,
    pub provider_id: String,
    pub requested_url: String,
    pub final_url: String,
    pub raw_sha256: String,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum EntryIdentity {
    Native { value: String },
    DerivedSha256 { value: String },
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TextRepresentation {
    Text,
    LiteralMarkup,
    XhtmlText,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct InertText {
    pub representation: TextRepresentation,
    pub text: String,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DateStatus {
    Missing,
    Invalid,
    Parsed,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FeedDate {
    pub status: DateStatus,
    pub raw: Option<String>,
    /// Non-leap seconds since Unix epoch, including negative pre-epoch values.
    pub unix_seconds: Option<i64>,
    /// Chrono representation: may be 1_000_000_000..<2_000_000_000 for a leap
    /// second, with unix_seconds naming the preceding whole non-leap second.
    /// The original raw date remains separately recoverable.
    pub nanoseconds: Option<u32>,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FeedLink {
    pub url: String,
    pub rel: String,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NormalizedEntry {
    pub identity: EntryIdentity,
    pub title: Option<InertText>,
    pub authors: Vec<String>,
    pub summary: Option<InertText>,
    pub content: Option<InertText>,
    pub omitted_content: bool,
    pub links: Vec<FeedLink>,
    pub omitted_links: u32,
    pub published: FeedDate,
    pub updated: FeedDate,
    /// Zero-based ordinal of the first identical variant in the original source.
    pub first_source_ordinal: u32,
    pub duplicate_count: u32,
    pub conflicting_identity: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct NormalizedFeed {
    pub schema: String,
    pub normalizer_profile: String,
    pub format: FeedFormat,
    pub provenance: FeedProvenance,
    pub title: Option<InertText>,
    pub entries: Vec<NormalizedEntry>,
}

/// Pure normalization of original XML bytes. No network, renderer, clock, cache,
/// identity attribution or operational permission is invoked by this function.
pub fn normalize(context_bytes: &[u8], raw: &[u8]) -> Result<NormalizedFeed> {
    let context = NormalizationContext::parse(context_bytes)?;
    preflight(raw)?;
    let text = std::str::from_utf8(raw).map_err(|_| invalid("Feed XML must be UTF-8"))?;
    let document = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: MAX_XML_EVENTS as u32,
        },
    )
    .map_err(|_| invalid("Malformed or unsupported feed XML"))?;
    let root = document.root_element();
    let (format, container, entry_name, ns) = if named(root, None, "rss")
        && root.attribute("version") == Some("2.0")
    {
        (
            FeedFormat::Rss,
            singleton(root, None, "channel")?.ok_or_else(|| invalid("RSS requires one channel"))?,
            "item",
            None,
        )
    } else if named(root, Some(ATOM), "feed") {
        (FeedFormat::Atom, root, "entry", Some(ATOM))
    } else {
        return Err(invalid("Expected RSS 2.0 or namespaced Atom feed"));
    };
    if context.expected_format != FeedFormat::Auto && context.expected_format != format {
        return Err(invalid("Feed format does not match requested format"));
    }
    let title = singleton(container, ns, "title")?
        .map(|node| construct(node, format))
        .transpose()?
        .flatten();
    let mut entries = Vec::new();
    for node in container.children().filter(|n| named(*n, ns, entry_name)) {
        if entries.len() == MAX_FEED_ENTRIES {
            return Err(invalid("Feed exceeds 500 entries"));
        }
        let mut item = entry(node, format, &context.final_url)?;
        item.first_source_ordinal = entries.len() as u32;
        entries.push(item);
    }
    let mut variants: BTreeMap<(String, String), NormalizedEntry> = BTreeMap::new();
    for item in entries {
        let identity = serde_json::to_string(&item.identity)?;
        let mut comparable = item.clone();
        comparable.first_source_ordinal = 0;
        let variant = serde_json::to_string(&comparable)?;
        match variants.get_mut(&(identity.clone(), variant.clone())) {
            Some(previous) => previous.duplicate_count += 1,
            None => {
                variants.insert((identity, variant), item);
            }
        }
    }
    let mut counts = BTreeMap::<String, usize>::new();
    for (identity, _) in variants.keys() {
        *counts.entry(identity.clone()).or_default() += 1;
    }
    let entries = variants
        .into_iter()
        .map(|((id, _), mut item)| {
            item.conflicting_identity = counts[&id] > 1;
            item
        })
        .collect();
    let result = NormalizedFeed {
        schema: "booknook.normalized-feed.v1".into(),
        normalizer_profile: NORMALIZER_PROFILE.into(),
        format,
        provenance: FeedProvenance {
            source_id: context.source_id,
            provider_id: context.provider_id,
            requested_url: context.requested_url,
            final_url: context.final_url,
            raw_sha256: digest(raw),
        },
        title,
        entries,
    };
    if serde_json::to_vec(&result)?.len() > MAX_NORMALIZED_BYTES {
        return Err(invalid("Normalized feed exceeds 4 MiB"));
    }
    Ok(result)
}
fn preflight(raw: &[u8]) -> Result<()> {
    if raw.is_empty() || raw.len() > MAX_XML_BYTES {
        return Err(invalid("Feed XML must be nonempty and at most 2 MiB"));
    }
    std::str::from_utf8(raw).map_err(|_| invalid("Feed XML must be UTF-8"))?;
    let mut reader = Reader::from_reader(raw);
    reader.config_mut().check_comments = true;
    let mut depth = 0;
    for count in 0..=MAX_XML_EVENTS {
        if count == MAX_XML_EVENTS {
            return Err(invalid("Feed XML event limit exceeded"));
        }
        let event = reader
            .read_event()
            .map_err(|_| invalid("Malformed feed XML preflight"))?;
        // Bound names/declarations/combined start tags too, before the DOM.
        if event.as_ref().len() > MAX_TEXT_BYTES {
            return Err(invalid("XML event exceeds string limit"));
        }
        match &event {
            Event::DocType(_) => return Err(invalid("DTD and entity declarations are forbidden")),
            Event::Start(start) | Event::Empty(start) => {
                if depth + 1 > MAX_XML_DEPTH {
                    return Err(invalid("Feed XML depth exceeds 64"));
                }
                for (index, attribute) in start.attributes().enumerate() {
                    let attribute =
                        attribute.map_err(|_| invalid("Malformed or duplicated XML attribute"))?;
                    if index >= MAX_XML_ATTRIBUTES {
                        return Err(invalid("XML attributes exceed 64"));
                    }
                    if attribute.value.len() > MAX_TEXT_BYTES {
                        return Err(invalid("XML attribute exceeds string limit"));
                    }
                }
                if matches!(event, Event::Start(_)) {
                    depth += 1;
                }
            }
            Event::End(_) => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| invalid("Unbalanced feed XML"))?;
            }
            Event::Decl(decl) => {
                if let Some(encoding) = decl.encoding() {
                    let encoding =
                        encoding.map_err(|_| invalid("Invalid XML encoding declaration"))?;
                    if !encoding.eq_ignore_ascii_case("utf-8") {
                        return Err(invalid("Only UTF-8 XML declarations are supported"));
                    }
                }
            }
            Event::Eof => {
                return if depth == 0 {
                    Ok(())
                } else {
                    Err(invalid("Unclosed feed XML"))
                };
            }
            _ => {
                if event.as_ref().len() > MAX_TEXT_BYTES {
                    return Err(invalid("XML event exceeds string limit"));
                }
            }
        }
    }
    unreachable!()
}
fn named(node: Node<'_, '_>, ns: Option<&str>, name: &str) -> bool {
    node.is_element() && node.tag_name().namespace() == ns && node.tag_name().name() == name
}
fn singleton<'a, 'input>(
    parent: Node<'a, 'input>,
    ns: Option<&str>,
    name: &str,
) -> Result<Option<Node<'a, 'input>>> {
    let mut nodes = parent.children().filter(|n| named(*n, ns, name));
    let first = nodes.next();
    if nodes.next().is_some() {
        return Err(invalid("Duplicate feed singleton field"));
    }
    Ok(first)
}
fn plain(node: Node<'_, '_>, limit: usize) -> Result<String> {
    if node.children().any(|n| n.is_element()) {
        return Err(invalid("Unexpected nested markup in scalar feed field"));
    }
    let value = node
        .children()
        .filter(|n| n.is_text())
        .filter_map(|n| n.text())
        .collect::<String>();
    bound(&value, limit)?;
    Ok(value)
}
fn scalar(
    parent: Node<'_, '_>,
    ns: Option<&str>,
    name: &str,
    limit: usize,
) -> Result<Option<String>> {
    singleton(parent, ns, name)?
        .map(|n| plain(n, limit))
        .transpose()
}
fn construct(node: Node<'_, '_>, format: FeedFormat) -> Result<Option<InertText>> {
    if node.attribute("src").is_some() {
        return Ok(None);
    }
    let kind = if format == FeedFormat::Atom {
        node.attribute("type").unwrap_or("text")
    } else if node.tag_name().name() == "description"
        || node.tag_name().namespace() == Some(CONTENT)
    {
        "html"
    } else {
        "text"
    };
    let (representation, text) = match kind {
        "text" | "text/plain" => (TextRepresentation::Text, plain(node, MAX_TEXT_BYTES)?),
        "html" | "text/html" => (
            TextRepresentation::LiteralMarkup,
            plain(node, MAX_TEXT_BYTES)?,
        ),
        "xhtml" => {
            let elements: Vec<_> = node.children().filter(|n| n.is_element()).collect();
            if elements.len() != 1 || !named(elements[0], Some(XHTML), "div") {
                return Err(invalid("Atom XHTML requires one namespaced div"));
            }
            let mut pieces = Vec::new();
            for text in elements[0].descendants().filter(|n| n.is_text()) {
                if text
                    .ancestors()
                    .take_while(|n| *n != node)
                    .filter(|n| n.is_element())
                    .any(|n| {
                        n.tag_name().namespace() != Some(XHTML)
                            || blocked_xhtml(n.tag_name().name())
                    })
                {
                    continue;
                }
                if let Some(value) = text.text() {
                    pieces.push(value);
                }
            }
            let text = pieces
                .join(" ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            bound(&text, MAX_TEXT_BYTES)?;
            (TextRepresentation::XhtmlText, text)
        }
        _ => return Ok(None),
    };
    Ok(Some(InertText {
        representation,
        text,
    }))
}
fn blocked_xhtml(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "script"
            | "style"
            | "iframe"
            | "object"
            | "embed"
            | "svg"
            | "math"
            | "template"
            | "noscript"
            | "link"
            | "meta"
            | "base"
            | "audio"
            | "video"
            | "source"
            | "img"
            | "input"
            | "button"
            | "form"
            | "canvas"
    )
}
fn date(raw: Option<String>, format: FeedFormat) -> FeedDate {
    let parsed = raw.as_ref().and_then(|s| {
        if format == FeedFormat::Atom {
            DateTime::parse_from_rfc3339(s.trim()).ok()
        } else {
            DateTime::parse_from_rfc2822(s.trim()).ok()
        }
    });
    FeedDate {
        status: if raw.is_none() {
            DateStatus::Missing
        } else if parsed.is_some() {
            DateStatus::Parsed
        } else {
            DateStatus::Invalid
        },
        raw,
        unix_seconds: parsed.map(|d| d.timestamp()),
        nanoseconds: parsed.map(|d| d.timestamp_subsec_nanos()),
    }
}
// Explicit-action article data has a different policy from acquisition URLs:
// HTTP(S), queries/fragments and ports are retained. No automatic request occurs.
// Reuse the shared conservative DNS-host syntax/suffix policy only, after
// removing article-specific components from a clone; never attach credentials.
fn safe_link_reference(raw: &str) -> bool {
    if raw.is_empty()
        || raw.len() > 2048
        || raw.chars().any(|c| c.is_control() || c.is_whitespace())
        || raw.contains('\\')
    {
        return false;
    }
    if raw.as_bytes().iter().enumerate().any(|(i, b)| {
        *b == b'%'
            && !raw
                .as_bytes()
                .get(i + 1..i + 3)
                .is_some_and(|v| v.iter().all(u8::is_ascii_hexdigit))
    }) {
        return false;
    }
    let authority = raw
        .strip_prefix("//")
        .or_else(|| raw.split_once("://").map(|(_, tail)| tail));
    !authority.is_some_and(|v| {
        v.split(['/', '?', '#'])
            .next()
            .is_some_and(|v| v.contains('@'))
    })
}
fn safe_article_url(url: &url::Url) -> bool {
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.as_str().len() > 2048
    {
        return false;
    }
    let mut host_probe = url.clone();
    if host_probe.set_scheme("https").is_err() || host_probe.set_port(None).is_err() {
        return false;
    }
    host_probe.set_query(None);
    host_probe.set_fragment(None);
    feeds::endpoint(host_probe.as_str()).is_ok()
}
fn link(node: Node<'_, '_>, value: &str, base: &str) -> Option<String> {
    if !safe_link_reference(value) {
        return None;
    }
    let mut url = url::Url::parse(base).ok()?;
    for ancestor in node
        .ancestors()
        .filter(|n| n.is_element())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        if let Some(base) = ancestor.attribute((XML, "base")) {
            if !safe_link_reference(base) {
                return None;
            }
            url = url.join(base).ok()?;
            if !safe_article_url(&url) {
                return None;
            }
        }
    }
    let resolved = url.join(value).ok()?;
    safe_article_url(&resolved).then(|| resolved.to_string())
}
fn entry(node: Node<'_, '_>, format: FeedFormat, base: &str) -> Result<NormalizedEntry> {
    let ns = if format == FeedFormat::Atom {
        Some(ATOM)
    } else {
        None
    };
    let native = scalar(
        node,
        ns,
        if format == FeedFormat::Atom {
            "id"
        } else {
            "guid"
        },
        8192,
    )?;
    let title = singleton(node, ns, "title")?
        .map(|n| construct(n, format))
        .transpose()?
        .flatten();
    let summary = singleton(
        node,
        ns,
        if format == FeedFormat::Atom {
            "summary"
        } else {
            "description"
        },
    )?
    .map(|n| construct(n, format))
    .transpose()?
    .flatten();
    let content_node = if format == FeedFormat::Atom {
        singleton(node, ns, "content")?
    } else {
        singleton(node, Some(CONTENT), "encoded")?
    };
    let content = content_node
        .map(|n| construct(n, format))
        .transpose()?
        .flatten();
    let omitted_content = content_node.is_some() && content.is_none();
    let mut authors = BTreeSet::new();
    for author in node.children().filter(|n| {
        named(*n, ns, "author") || (format == FeedFormat::Rss && named(*n, Some(DC), "creator"))
    }) {
        let value = if format == FeedFormat::Atom {
            scalar(author, ns, "name", 8192)?.unwrap_or_default()
        } else {
            plain(author, 8192)?
        };
        if !value.is_empty() {
            authors.insert(value);
        }
        if authors.len() > 64 {
            return Err(invalid("Feed authors exceed 64"));
        }
    }
    let mut links = BTreeMap::new();
    let mut omitted_links = 0;
    for candidate in node.children().filter(|n| named(*n, ns, "link")) {
        let rel = if format == FeedFormat::Atom {
            candidate.attribute("rel").unwrap_or("alternate")
        } else {
            "alternate"
        };
        if !matches!(rel, "alternate" | "related") {
            omitted_links += 1;
            continue;
        }
        let raw = if format == FeedFormat::Atom {
            candidate.attribute("href").unwrap_or("").to_owned()
        } else {
            plain(candidate, 2048)?
        };
        if let Some(url) = link(candidate, &raw, base) {
            links.insert(
                (url.clone(), rel.to_owned()),
                FeedLink {
                    url,
                    rel: rel.into(),
                },
            );
        } else {
            omitted_links += 1;
        }
        if links.len() > 64 {
            return Err(invalid("Feed links exceed 64"));
        }
    }
    let mut result = NormalizedEntry {
        identity: EntryIdentity::DerivedSha256 {
            value: String::new(),
        },
        title,
        authors: authors.into_iter().collect(),
        summary,
        content,
        omitted_content,
        links: links.into_values().collect(),
        omitted_links,
        published: date(
            scalar(
                node,
                ns,
                if format == FeedFormat::Atom {
                    "published"
                } else {
                    "pubDate"
                },
                512,
            )?,
            format,
        ),
        updated: date(
            if format == FeedFormat::Atom {
                scalar(node, ns, "updated", 512)?
            } else {
                None
            },
            format,
        ),
        first_source_ordinal: 0,
        duplicate_count: 1,
        conflicting_identity: false,
    };
    result.identity = match native.filter(|value| !value.is_empty()) {
        Some(value) => EntryIdentity::Native { value },
        None => EntryIdentity::DerivedSha256 {
            value: digest(&serde_json::to_vec(&result)?),
        },
    };
    Ok(result)
}
