# Portable feeds

**Status: implemented development SDK and native app, 2026-11-14.** Current source a945df2 includes strict raw settings review, public-URL addition/import/export in Connections → Publications, explicit local Enable & refresh/Pause/Remove, native durable offline cache and external cards in Discover. [Exact-source CI](security-020-ci-evidence.json) and [0.2.0/build 2026111403 native Release qualification](native-020-release-evidence.json) pass. Opening/importing/navigation/network regain never auto-fetches. No production Castaway writer or live cross-client handoff is enabled; hands-on/device/accessibility and production cross-client acceptance remain open. These independent SDK contracts do not issue a new Castalia ontology. [DevGraph](DEVGRAPH.md) records remaining rollout work.

## Where it fits

Castaway owns portable non-secret subscriptions and preferences. Each accepting site owns its local review, activation, credentials, transport and cache. Book Nook's portable SDK owns deterministic configuration validation, reconciliation and normalized external-publication records; host adapters perform asynchronous acquisition and durable storage.

Native reading and writing remain calm, side by side and independently resizable. Feed management belongs in discovery/connections, not in the notebook margin or editor. External publications remain distinguishable from verified Castalia/Nostr friends and groups. A feed author label is source attribution, not a verified Rolodex identity or operational grant. Private notebook content is excluded from feed processing and recommendations.

## Compatibility decision

Inspected clients already implement strict Castaway v1 and **v2 for wallet identities**. Adding a section to either issued shape would cause rejection; v2 is not available for a different purpose.

The selected target is matching `castalia.castaway.v3` and `castalia.castaway-contents.v3`, integer version 3, with the existing encryption/KDF and authenticated owner-binding profile. V3 retains identity and known legacy sections and requires a `feeds` section, including when empty. Existing optional wallet identities must survive every import, edit and export. Account helpers must not force a downgrade to v2.

| Stored sections | Intended export | Import behavior |
|---|---|---|
| Legacy only | Existing v1 | Preserve already committed newer local sections |
| Wallet identities, no feeds | Existing v2 | Preserve local feeds; retain existing v2 meanings |
| Feeds present, even empty | Proposed v3 | Preserve every recognized section; no downgrade |
| Unknown version or section | Refuse entire import | Explain incompatibility; never strip data |

Readers, encrypted persistence, previews and independent fixtures ship before v3 writers are enabled. Old readers should reject v3 visibly. No generic opaque-extension container or CWS-001 draft implementation is introduced.

The Book Nook provider v1 and issued ontology releases remain byte-identical. Feed providers need an independent versioned contract or a later explicit provider release, rather than extending strict v1 enums in place. Reading backups remain separate from subscription configuration.

## Portable section

The intended contents section is `sections.feeds`:

```json
{
  "schema": "castalia.feeds-section.v1",
  "version": 1,
  "subscriptions": [{
    "id": "b41ef5e8-0e06-43e0-aa47-19f382e00c08",
    "url": "https://example.org/feed.xml",
    "format": "auto",
    "title": "Reading essays",
    "enabled": true,
    "collectionIds": [],
    "disclosure": "private"
  }],
  "collections": [],
  "preferences": {"order": "newest-first"}
}
```

This independent data contract is accepted by the native raw settings-file import flow. It grants neither disclosure nor fetch permission. Importing a complete encrypted Castaway vault or an origin-bound envelope is not that file flow. Downloading encrypted data and reviewing a selected projection remain separate actions.

Implemented limits and semantics, pinned by shared fixtures:

- Exact keys and versions; reject duplicate JSON properties, unknown fields, malformed UTF-8, lone surrogates and invalid control characters. Raw version tokens are JSON integers: decimal/exponent aliases such as 1.0 or 1e0 must fail before JavaScript number conversion.
- Canonical lowercase UUIDs with version 1–8 and RFC variant 8/9/a/b; nil/max values are rejected. IDs are unique separately within subscription and collection namespaces. Collection references are unique within a subscription and must resolve. Each collection is exactly an id/title record; subscription collectionIds carry membership.
- At most 256 subscriptions, 64 collections and 16 collection references per subscription. Limits fail the entire candidate without truncation.
- At most 256 KiB compact serialized UTF-8 feed-section JSON, with the same byte gate on standalone raw input; 2,048 UTF-8 bytes per URL and 512 UTF-8 bytes per label. Empty/whitespace labels mean no custom label and remain unchanged. Reject Unicode Cc controls and lone surrogates while preserving accepted text, including combining forms and ZWJ; no silent Unicode normalization. The existing whole encrypted Castaway artifact limit remains 1 MiB. Before committing wallet configuration, serialize/encrypt the complete candidate including preserved sections and base64 overhead and enforce that whole-file gate; reject an unexportable candidate without truncation or partial writes. A small feed section alone does not establish room in the vault.
- `format` is `auto`, `rss` or `atom`; `disclosure` is `private` or `selected`; `enabled` is a preference. All new-host sources start locally inactive regardless of these values.
- First version accepts query-free public HTTPS URLs on the default port, with DNS hostnames and no userinfo or fragments. Query-based feeds remain explicitly unsupported until a reviewed non-secret query policy exists. Require byte-identical URL serialization, ASCII DNS labels, implicit port, valid percent escapes and no parser-rewritten host/path. Reject IP literals, ambiguous host encodings, trailing host dots and local/unqualified hostnames. The initial suffix denylist is localhost/local/internal/home/lan/corp/test/invalid/example/onion/arpa, including subdomains; example.org/net/com are admissible syntax. Public-address checks still occur at connection time and on redirects.
- Do not carry cookies, keys, credentials, grants, tokenized URLs, executable renderer references, item caches or polling cursors. URL syntax cannot prove a path is non-secret: import/export review must identify the endpoint as public rather than claiming heuristic token detection is sufficient.
- Preserve array order in exports; display entries newest-first using source dates, stable tie-breakers and an explicit undated group. Do not invent publication times from fetch time or convert audiobook/ebook percentages.

## Import and disclosure

An absent feed section means **no incoming feed change**. An empty imported collection does not delete local subscriptions. Identical stable IDs and equal values are idempotent; differing values require explicit conflict review before any write. A canceled, stale or conflicting candidate commits nothing. Do not use timestamp-wins reconciliation.

Shared wallet import is additive and preserves unrelated sections. Replacement and deletion are separate reviewed host edits with explicit target scope. Duplicate URLs with different IDs are shown for review; title similarity never merges subscriptions automatically.

An accepting site advertises supported section versions and capabilities. The wallet reviews the exact destination origin and selected settings. The host receives only that projection, never a decrypted vault, identity claims or Member Key by default. Projection includes only selected subscriptions and their explicitly reviewed collection labels/memberships; unselected collection references are removed from the projection, and unreferenced collections are excluded. This transformation does not edit the saved source. Stored disclosure preferences do not transmit anything automatically.

Hosts bind review to exact source bytes/digest and local configuration revision, stage a candidate, then atomically commit. Import performs no fetch or renderer installation. Activation is a separate local choice with visible refresh, disable, last-success and failure state. Portable export/download remains available. Authenticated sources, if later supported, use non-secret descriptors resolved by secure local adapters.

## Source and cache boundary

The host supplies an asynchronous FeedSourceProvider through actual Rust/native/Swift/WASM/typed runtime paths. Provider capability advertisements do not install a provider or grant network authority. Every request and cache commit correlates source, provider, configuration generation and cancellation state.

Normalize [RSS 2.0](https://www.rssboard.org/rss-specification) and [Atom](https://www.rfc-editor.org/rfc/rfc4287) into bounded inert article/review/list records with source-native IDs, links, dates and provenance. Fetch policy must cover time, redirects, encoded/decompressed bytes, XML depth, entry count and string lengths; reject DTD/external entities. Resolve and verify public addresses at DNS, connection and redirect boundaries, with rebinding protection. Do not execute scripts, arbitrary HTML, plugins or feed-chosen renderers; media and outside links require explicit actions.

Public feed requests use an isolated credential-free transport: omit cookies, authentication, client certificates and inherited application headers; do not attach existing browser/native sessions. Do not store response cookies. A host proxy must not forward its own inbound authorization/cookies or ambient credentials to the publisher. Reapply the complete endpoint policy, not just public-address checks, to every redirect target: HTTPS, default port, no query/userinfo/fragment and admissible hostname. Redirects cannot become an authenticated or less restrictive source.

Native hosts supply transport; a browser host may need a disclosed proxy for CORS. Cache entries, ETags, polling cursors and local read state stay host-local in the first version. Conditional requests, offline reopening, bounded eviction and one-source failure isolation need acceptance coverage. A source changed during an awaited request cannot commit stale results.

## Source-provider SDK v1

The independent `booknook_core::feed_source` port composes explicitly installed host code. A descriptor negotiates support; it never installs transport or grants network permission:

```json
{"schema":"booknook.feed-source-provider.v1","version":1,"id":"reader.public","available":true,"protocols":["public-xml-bytes.v1"]}
```

Every runtime starts inactive, including when portable settings say enabled or selected. Activate a source locally before requesting it. Configuration/provider replacement revokes all activations and pending reads. Starting a newer refresh supersedes only older attempts for that source. Runtime, request, source, provider and monotonic generations bind responses and cache admission; provider requests omit collection labels, disclosure settings, identity and private configuration fingerprints.

Rust uses `FeedSourceSession`, `FeedSourceProvider` and `FeedCacheStore`. Swift exposes `BookNookFeedSourcesV1`, `BookNookFeedSourceProviderV1` and `BookNookFeedCacheStoreV1`; TypeScript exposes corresponding typed executable-provider and cache-stage interfaces backed by real WASM objects. Cancellation and close invalidate pending work; serialized request/preview data cannot create a valid opaque read. Bytes remain untrusted and require separate safe acquisition, XML normalization and rendering stages.

The local conditional hint is distinct from portable configuration:

```json
{"schema":"booknook.feed-conditional-hint.v1","source_id":"b41ef5e8-0e06-43e0-aa47-19f382e00c08","provider_id":"reader.public","url":"https://example.org/feed.xml","body_sha256":"b8a3805e669decf7180d8c7f4af5d4706d615d0678c424bd1c6b853def3bf3d2","validators":{"etag":"\"v1\"","last_modified":null}}
```

A not-modified response must match the exact issued source/provider/endpoint and prior digest, contain no body and use null content type. Cache admission requires actual retained bytes with that digest. The host must atomically compare durable source/generation/body state in its final transaction. Missing validators on 304 retain issued values; validators from a redirected final endpoint are not reused for the original endpoint. No fetch, publication or freshness timestamp is invented.

Async staging may precede the final commit. The final durable CAS must finish synchronously under the live runtime guard and cannot reenter it. An asynchronous final store, including IndexedDB, needs a separately qualified persistent-generation transaction; pre/post-await checks alone are insufficient. Cancellation returns false once that final transaction has started; it cannot undo an accepted commit. A later acknowledgement error likewise does not imply rollback.

Limits are 16 KiB metadata, 2 MiB untrusted body, 128 active pending reads and 32 installed core providers. ETags are at most 512 ASCII bytes with entity-tag syntax; Last-Modified requires valid 29-byte IMF-fixdate. Native/Swift and TypeScript wrappers compose one installed provider per runtime. This source port itself supplies no HTTP/XML/durable backend; the concrete implementations below compose it. Issued provider-v1, main Command and ontology bytes stay unchanged.

The source SDK passes 117 core tests (16 focused source cases), three native source tests, strict Clippy, actual optimized Node and web-target WASM/typed consumers, and fresh generated Swift async consumers. Existing 102-vector review checks still pass against the new artifacts. Independent review verified all 18 final files without a concrete blocker. The unchanged Rust package script also passes against all four extracted versioned crates, with local canonical native fixture mirrors. See [source SDK evidence](feed-source-evidence.json) and [214-pass extracted-package evidence](feed-package-evidence.json); these checks do not qualify a concrete HTTP/XML/storage backend or an installed app.

## Bounded XML normalization and native HTTPS

The additive `booknook_core::feed_xml::normalize(context_bytes, xml_bytes)` API is network-free. Native `normalize_feed_xml_v1`, WASM `normalize_feed_xml_v1`, Swift `BookNookFeedXMLV1.normalize(context:xml:)` and typed TypeScript `normalizeFeedXmlV1` expose the same original-byte operation. Its independent context is:

```json
{"schema":"booknook.feed-normalization-context.v1","source_id":"b41ef5e8-0e06-43e0-aa47-19f382e00c08","provider_id":"booknook.public-https.v1","requested_url":"https://example.org/feed.xml","final_url":"https://example.org/feed.xml","expected_format":"auto"}
```

Produced-only `booknook.normalized-feed.v1` records include `normalizer_profile: "booknook.rss2-atom.inert.v1"`, source/provider/requested/final endpoint provenance and raw SHA-256. They preserve native IDs, original date text, missing/invalid date state and source ordinals. Exact duplicate variants count; conflicting variants sharing an ID remain independently visible. Chrono leap seconds retain the preceding non-leap Unix second and nanoseconds in [1,000,000,000, 2,000,000,000); these values must not be fed directly into an ordinary Date constructor. No fetch-time publication date is invented.

A nonrecursive quick-xml preflight precedes the bounded roxmltree DOM. Limits are 2 MiB raw XML, depth 64, 50,000 events/nodes, 64 attributes per element, 64 KiB per raw event/string, 500 source entries and 4 MiB normalized JSON. Exceeding a limit rejects the whole candidate. DTDs, entity declarations, malformed UTF-8 and unsupported namespace/format shapes fail. Text and XHTML yield inert text; escaped HTML deliberately remains a tagged `literal-markup` fallback. No HTML execution, remote media, renderer installation, identity verification or network access occurs. Article links are explicit-action HTTP(S) data and can carry query/fragment; that does not relax the acquisition endpoint policy.

The produced-only types do not validate arbitrary reopened normalized-cache JSON. The native controller below binds full normalization context, raw digest and profile, and re-normalizes retained original bytes. Raw digest/profile alone cannot identify output across endpoints because relative links depend on final URL and XML base.

`booknook_connectors::public_https_feed::PublicHttpsFeedProvider::new()` supplies the native transport through the same source-provider port. It uses an isolated Rustls/WebPKI client with no proxy, cookie jar, inherited app client/headers, authentication or client identity. Every DNS answer must be public; every connection uses pinned answers without fallback and verifies its actual peer. Each redirect rechecks the complete strict query-free HTTPS endpoint policy, with at most three hops. Conditional validators go only to the original endpoint. Only identity content encoding, accepted XML media types and exact UTF-8 bytes/declarations qualify; other encodings are refused rather than silently decoded or truncated.

The shared 20-second deadline includes queue/DNS/redirect/body work; connection/read stalls have a 5-second ceiling. Four requests and four actual blocking DNS jobs bound concurrent work; cancelled OS lookups keep their permits until completion. Responses remain at most 2 MiB. Conservative special-address and currently allocated IPv6-prefix rules can refuse unusual public endpoints; new allocations need review. Default tests use actual TLS loopback sockets with test-only address/root hooks. A separate opt-in test uses the unchanged production constructor against a real public endpoint; these are distinct evidence boundaries.

Native/Swift hosts can instead choose `FeedSourceSessionV1::public_https(configuration_bytes)` / `BookNookFeedSourcesV1.publicHTTPS(configuration:)`. Construction performs no fetch and leaves every source inactive. The built-in host owns a lazy shared Tokio executor, so a Swift async caller does not need a reactor or perform network I/O on the calling thread. Dropping unfinished foreign work cancels and aborts the owned task; normal completion preserves the opaque read. Existing callback-based factories and guarded host cache interfaces are unchanged.

[XML parity evidence](feed-xml-evidence.json) records 132 core tests, the 27-case corpus through actual native, Node/web-target WASM and fresh Swift bindings, strict Clippy and declaration checks. [Transport evidence](feed-https-evidence.json) separates real TLS failure tests from one production public DNS/TLS fetch. [Factory evidence](feed-native-factory-evidence.json) records offline default Swift checks and an explicit production HTTPS → UniFFI → XML run producing 10 inert entries from 164,045 exact bytes. Web-target WASM is instantiated in Node, not browser UI. The full SDK regression and [actual package checks](feed-ingestion-package-evidence.json) also pass: 254 extracted-Rust tests, five declared opt-in/helper skips, all four crates, and installed CJS/ESM/NodeNext consumers with all 72 payload files matching their archive. The 404 frozen code/lock/license/fixture inputs did not change during verification. These are local development artifacts, not registry releases; exact-revision CI remains separate.

The historical acquisition-only checkpoint above did not include durable cache, host import or app discovery. Those native layers are now implemented below; production Castaway-v3 activation and cross-client handoff remain disabled/gated. Original account, custody, physical-device and production-release qualification remain separate.

## Native Rust cache foundation

The standalone `booknook_native::FeedCacheController` now owns coordinated SQLite storage independently of the private Desk and portable configuration exports. [The cache contract](FEED-CACHE.md) covers exact authoritative configuration CAS, inactive restart, durable owner/provider/activation/request fences, controller-created attempts, transactional actual-raw 304 admission and exact receipt reconciliation. Offline reads re-normalize original bytes, recheck a final read snapshot and require no recency write. Explicit endpoint/format/provider changes invalidate only incompatible rows through the reviewed lifecycle transaction. Logical retention is 64 MiB/128 rows with admission/refresh-recency eviction; it is not a physical SQLite/WAL cap.

[Focused native evidence](feed-cache-evidence.json) records 17 unit cases and three integration scenarios plus an ordinary child-helper entry, strict all-target Clippy and formatting. Real checks include SQLite FULL, write-disabled offline display, busy-handler-confirmed canceled lifecycle transactions, callback reentry, exact304 corruption/context fences, held network blocking slots with independent storage progress, and child SIGKILL/reopen. Acknowledgement/unreadable faults and executor saturation are explicitly classified; no public-network or injected-clock qualification is inferred. This is the initial Rust-native development evidence. Its committed source4585376 subsequently passes [both hosted jobs](feed-cache-ci-evidence.json). The additive UniFFI/Swift facade separately passes [focused foreign-consumer qualification](feed-cache-bindings-evidence.json): 20 native cache cases, three durable integration scenarios plus one ordinary helper, an exported facade test and actual matching generated Swift consumers. Explicit operation cancellation fences queued/running storage and active source tokens, retains accepted acknowledgements and closes canceled construction. These later facade changes are outside the 4585376 hosted evidence. Current native workflow/Release qualification is recorded below; hands-on app acceptance remains open.

## Historical rollout audit and remaining acceptance

The commit/pin inventory below records the preimplementation rollout audit. It is not a current branch/status report. Later source, guarded receiving and native qualification appear below; production cross-client enabling remains a separate gate.

| Surface | Inspected baseline | Required change |
|---|---|---|
| Wallet Rust portable and independent web-compatible codec | Wallet `5b43dea1144f4828888c9cf09b56efea41397c82`; core `9def6b2f5a2b06ce607b44135073cdab8d8c09c5` | V3 read/validate/encode, owner/version binding and lossless legacy/new sections |
| Rust recovery session | Same reviewed Wallet/core line | Candidate merge, exact atomic host write/confirm and no loss from older profile snapshots |
| Web contract and encrypted custody | Web `a1ff37d18fb020f0933f95bc961226dd98f35cc8`, core `19ec02f`/0.4.1 | TS codec, Worker/protocol/storage, reconstruction, preview and export |
| Wallet extension | Reviewed Wallet main above | Preserve v3 through account edits and import/export helpers currently forcing v2 |
| Transfer and iOS admission | Transfer `7f8e4ca6b3881d9827295194b5d09bb2ddd9d03e`; iOS `7a113d7` | Native envelope admission, portable sections, catalog/restore preview and review |
| Book Nook | `abd2cb1928518be66eda247bea473c98413bb1a2` | Public feed SDK/provider, selected inert import, local activation and discovery |
| Anima | Downstream core consumer | Requalify runtime when pinned; no current Castaway feed UI is assumed |

Native Dependencies.json at those reviewed commits actually pins `72ed5cd`; the workspace registry also records `9def6b2`. Preserve this provenance discrepancy and resolve actual package/runtime pins before release. The inspected portable codec blobs are identical at those core revisions, so the strict v1/v2 finding is unchanged. Historical worktree heads are inspection evidence, not automatically suitable development baselines.

Qualification requires raw duplicate-key and hostile-input fixtures; wrong-owner/version/unknown-section refusal; legacy imports preserving feeds; empty/conflicting/canceled/stale imports; account edits preserving v3; encrypted cross-client round trips; restart/failed atomic writes; no-network import; selected-origin projection; SSRF/XXE/decompression/cache-generation tests, whole-vault size preflight, credential-free redirect/proxy behavior and reviewed collection-projection closure; installed SDK consumer parity; and current native discovery/accessibility use. These do not replace original source-account, custody, hardware, Nostr or device-sync gates.

## Historical development checkpoints — 2026-11-14

The new `booknook_core::feeds` domain validates original bounded bytes against all 102 canonical raw cases and preserves accepted values. Pure import reviews bind the exact source bytes and current configuration; resolving conflicts requires the original review and a complete explicit choice batch. Pure confirmation returns data for a host to persist atomically. It performs no activation, networking or custody operation.

Selected projection reviews bind the exact HTTPS destination and source configuration. Sources, collection labels and each membership are separate choices; only marked-selected sources qualify. Unreferenced collection labels are removed after membership selection, including when no source is chosen. The original configuration is unchanged and its private fingerprint is excluded from the projection.

Fourteen focused core tests, three native adapter tests and actual newly generated Swift, Node WASM and web-target WASM consumers pass the same 102 canonical vectors. The consumers also exercise exact-input staleness, conflict choices, closed reviews and origin/projection closure. Strict TypeScript and focused Clippy pass; independent binding review found no concrete blockers. [Binding evidence](feed-sdk-bindings-evidence.json) records this increment; the [earlier Rust-only evidence](feed-sdk-evidence.json) describes the a9d0fd9 baseline. Issued provider-v1 and ontology bytes are unchanged.

Review previews cannot be deserialized into usable handles. Confirmation returns data; hosts own consent, cancellation, atomic persistence and disclosure. Each generated interface/native payload pair needs matching provenance; the current 0.2.0 Swift package is qualified separately in [native Release evidence](native-020-release-evidence.json). Neither Node-instantiated web-target WASM nor a binding test establishes browser UI or current installed-app acceptance.

Shared preservation source is merged in [Wallet PR23](https://github.com/ZenithResearch/castalia-wallet/pull/23), [Transfer PR19](https://github.com/ZenithResearch/zenith-transfer/pull/19) and [iOS PR17](https://github.com/ZenithResearch/zenith-ios/pull/17), at e16dc940, 5cb22d7 and add44321 respectively. Wallet’s full suite passes 450 with 4 opt-in skips; 12 actual disposable Chrome storage checks include final-read custody invalidation. Native consumers retain exact d688a385 development artifacts; four actual encrypted tests and the full 416/20-skip Transfer suite qualify the macOS domain, without packaged/iOS runtime acceptance.

[Web PR68](https://github.com/ZenithResearch/castalia-web/pull/68) remains open at a1173c9; its storage increment was introduced at b6ff626. Guarded encrypted storage/review passes 226 tests with 8 skips and 16 actual disposable Chrome/Worker/IndexedDB checks, including restart, CAS, cancellation, failed-write rollback and owner refusal. Quota denial is injected. The private transformed build has a default negative control; production v3 admission and writers remain disabled. The JavaScript size budget passes. Earlier cold-WASM failures retain a dedicated Issue; a later bounded comparison passed the unchanged source-build gate once on both x64 and ARM, establishing neither repeatability nor an ARM advantage. Its original x64 job then failed GitHub artifact upload on storage quota and downstream release jobs skipped. The comparison job was removed without a runner migration at Web a1173c9, and a separate shared Issue tracks artifact capacity. The next unchanged x64 run failed the cold gate again at73,569.0/60,000ms, so both qualification boundaries remain open. A development fixture or private qualification build is not an enabled production capability.

## Later work

Public RSS/Atom output uses only explicitly reviewed already-public ShareDraft/review/list snapshots. It does not publish private notebook text, purchased book content, private groups, subscription lists or reading history by implication. Rich component hints remain small, allowlisted data interpreted by trusted installed renderers with readable fallbacks. Feed-supplied code and renderer URLs are excluded. Both extensions remain later DevGraph work.

## Origin-free settings export SDK

A native settings file has no website destination. The additive closed `booknook.feed-export-selection.v1` request contains `subscriptionIds`, `collectionIds` and explicit `memberships`, with no origin. Opaque core/native/Swift/WASM/TypeScript reviews return the existing raw `castalia.feeds-section.v1`, which the original strict import accepts unchanged. Choosing export never marks a private source selected; eligibility is a separate reviewed configuration edit. Only chosen memberships survive and orphan labels are omitted.

Confirmation preserves Book Nook’s canonical configuration/revision semantics: equivalent JSON whitespace/key order remains acceptable; changed revision/content refuses. The origin-bound projection and HTTPS policy are unchanged. [Focused export qualification](feed-export-evidence.json) retains its 17 core/4 native/102-vector historical scope across actual Swift/Node/web-WASM and typed wrappers. App file operations have separate evidence below. The subsequent [literal-string correction](feed-string-enums-evidence.json) rejects legacy enum-tag aliases across feed inputs without changing valid string forms or silently rewriting malformed historical data.

## Native Publications and guarded browser receiving — 2026-11-14

Connections → Publications provides reviewed manual public-URL addition and bounded original raw settings-file import, per-ID conflicts and atomic configuration CAS. Export uses separate eligibility and selected source/collection/membership reviews. Real configuration changes pause sources; canonical no-ops preserve activation. One Desk-owned repository coordinates windows in a separate feed store and refuses malformed existing data without resetting it. Opening, importing, restarting, navigating or network regain never activates or refreshes a source.

Discover keeps book recommendations first, then eight external cards with local Show more/Show fewer controls. Its index is bounded to120 rows, loads offline and opens one full article at a time. Source/date/variant provenance is retained; strings are literal and external links require explicit action. [Current 0.2.0 native Release evidence](native-020-release-evidence.json) covers app workflows, all 3 Release builds, extracted Swift, SQLite continuity and archive inspection. The extracted Swift consumer executes macOS arm64 only; device/Simulator SDK slices are built/packaged/metadata-inspected, not runtime-executed. Hands-on UI, VoiceOver, multiwindow interaction and physical-device qualification remain open.

The [browser receiver fixture](feed-browser-receiver-evidence.json) independently exercises actual packaged WASM/TypeScript, opaque original-byte review and atomic IndexedDB persistence. It passes ten receiving-host cases in the Wallet adapter's eleven-group disposable Chrome run. Import consent and source activation remain distinct. This fixture is test automation for a future browser host, not a released browser app. Wallet PR27 retains its disabled production capability and default-deny destination policy; native file import does not claim a live browser handoff.

Both hosted jobs pass at exact current source a945df2: [run 43616208883](security-020-ci-evidence.json),17 Apple/21 portable steps including the new audit gate, SDK consumers and Rust/npm packaging. [Current native Release/Swift archives](native-020-release-evidence.json) have separate compiled-input and byte provenance; selected local Rust/npm release archives now pass extracted/installed checks, while the replacement is published; actual public SDK receiving consumers now pass under the separately audited lock. These are offline/deterministic receiving checks, with macOS-only Swift execution. Earlier [995b36bd CI](feed-export-enum-ci-evidence.json) and [package evidence](feed-export-enum-package-evidence.json) retain only historical scope. These results enable neither production Castaway handoff nor a released browser app.
