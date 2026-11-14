# Portable feeds: implementation target

**Status: planned, 2026-11-13.** The current app does not import Castaway feed settings, fetch RSS/Atom or present external-publication cards. This document selects a narrow implementation target; its schema names are proposals, not an issued Castalia ontology release or a new writable Castaway format. [DevGraph ownership and dependencies](DEVGRAPH.md) precede implementation.

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

## Proposed section

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

This is a proposed data contract, not an executable example supported by today's clients. It grants neither disclosure nor fetch permission. The encrypted file can itself reveal reading interests; downloading it and projecting selected settings are separate actions.

Initial limits and semantics to encode in shared fixtures:

- Exact keys and versions; reject duplicate JSON properties, unknown fields, malformed UTF-8, lone surrogates and invalid control characters.
- Canonical lowercase UUIDs for subscription and collection IDs; no duplicate IDs or dangling collection references. Each collection is exactly an id/title record; subscription collectionIds carry membership.
- At most 256 subscriptions, 64 collections and 16 collection references per subscription. Limits fail the entire candidate without truncation.
- At most 256 KiB raw UTF-8 feed-section JSON, 2,048 UTF-8 bytes per URL and 512 UTF-8 bytes per label. Preserve accepted labels exactly; no silent Unicode normalization. The existing whole encrypted Castaway artifact limit remains 1 MiB. Before committing wallet configuration, serialize/encrypt the complete candidate including preserved sections and base64 overhead and enforce that whole-file gate; reject an unexportable candidate without truncation or partial writes. A small feed section alone does not establish room in the vault.
- `format` is `auto`, `rss` or `atom`; `disclosure` is `private` or `selected`; `enabled` is a preference. All new-host sources start locally inactive regardless of these values.
- First version accepts query-free public HTTPS URLs on the default port, with DNS hostnames and no userinfo or fragments. Query-based feeds remain explicitly unsupported until a reviewed non-secret query policy exists. Reject IP literals, ambiguous host encodings and local/unqualified hostnames. Public-address checks still occur at connection time and on redirects.
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

## Consumer rollout evidence required

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

## Later work

Public RSS/Atom output uses only explicitly reviewed already-public ShareDraft/review/list snapshots. It does not publish private notebook text, purchased book content, private groups, subscription lists or reading history by implication. Rich component hints remain small, allowlisted data interpreted by trusted installed renderers with readable fallbacks. Feed-supplied code and renderer URLs are excluded. Both extensions remain later DevGraph work.
