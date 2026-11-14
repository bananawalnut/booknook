# Delivery work in DevGraph

Reconciled and independently read back on **2026-11-13** against Book Nook source `abd2cb1928518be66eda247bea473c98413bb1a2`. DevGraph is the work tracker; this document is a navigation snapshot, not a replacement tracker or acceptance result.

## Ownership

- **Arena/castalia → Initiative/booknook-delivery → Project/booknook**: native reading notebook, SDKs and original release qualification.
- **Arena/castalia → Initiative/castalia-storage-publishing-filesystem → Project/castalia-portable-feeds**: shared Castaway feed format, preservation and selected disclosure. Book Nook consumes that work.

The existing minimal Book Nook delivery container is reused. No new initiative was created: DevGraph Projects inherit Arena membership through an Initiative. The separate `Task/booknook-consulting-reading-list` is not duplicated or reparented.

The old October attempt created the container but its expired-authority membership request did not commit; neither the Project nor its 17 prepared issues had been created. November's exact, version-checked requests completed using the existing configured authority, without renewal, new identity or widened grants. Read-back verifies **42 records** (2 Projects, 26 Issues, 14 Tasks), their parentage, both effective Castalia memberships and **34 dependency edges**. Records were created in draft status; none is claimed accepted.

Operational EventReceipts and exact request/idempotency files remain outside this public repository. Canonical read-back establishes local commits. Pending outbox processing is not a completed external delivery or a portable cryptographic receipt.

## Original Book Nook backlog

All 17 previously prepared issues are now children of `Project/booknook`. Their descriptions retain acceptance criteria, source evidence and the distinction between synthetic, automated, physical and live qualification.

| Issue ID | Remaining work |
|---|---|
| booknook-kindle-live | Real library/account coverage, repeat imports, unavailable content and expiry |
| booknook-audible-live | Real login/MFA/registration, secure storage, renewal and listening positions |
| booknook-castalia-live | Existing identity, issuer trust, physical custody and exact operational consent |
| booknook-physical-ink | Working Pencil and finger input, palm rejection, anchoring and recovery |
| booknook-physical-keyboard-ime | Hardware/software input, first key, IME and exact undo/redo; preserve the unexplained intermittent report |
| booknook-accessibility-endurance | VoiceOver, keyboard navigation, text size and prolonged long-document use |
| booknook-hands-on-workflows | Remaining current-build reader, character, sharing and recovery paths |
| booknook-nostr-remote | Real bound identities, remote relays, private groups, removal, outage and retries |
| booknook-device-sync-live | Approved encrypted transfer, conflicts and recovery across actual devices |
| booknook-current-packages | Rebuild and verify current native/SDK archives, provenance and runtime behavior at the declared macOS 14/iPadOS 17 minimums |
| booknook-ontology-publication | Independent public ontology discovery and offline/public artifact agreement |
| booknook-sdk-publication | Public package destinations, installable consumers and compatible versions |
| booknook-ci-evidence | Current exact-revision CI and reconciliation of historical delivery claims |
| booknook-local-install-renewal | Inspect and renew the historical local iPad development install if needed |
| booknook-optional-distribution | Optional TestFlight/App Store/notarization; separate from the user's chosen local install |
| booknook-post-mvp-roadmap | Later browser, automatic narration following, live notebooks and direct peer connections |
| booknook-large-library-scaling | Profile whole-state storage/history and design indexed or chunked delivery |

The October 16 iPad install and profile-expiry evidence are historical. The profile's recorded October 23 Vancouver expiry has passed; this does not establish the current state of the device. No physical device was changed during this reconciliation.

## Feed increment

See [the planned feed contract and rollout](FEEDS.md). These are new capabilities to implement, not features of the current app.

| Owner | Issue ID | Scope |
|---|---|---|
| Shared Castaway | castaway-feed-contract | Versioned data-only format, independent codecs, encrypted persistence and cross-client preservation |
| Shared Castaway | castaway-feed-disclosure | Capability negotiation, exact-origin selected projection and inert host import |
| Book Nook | booknook-feed-provider | Portable domain/SDK and composed host source interface |
| Book Nook | booknook-feed-ingestion | Bounded RSS/Atom fetch, normalization, conditional cache and offline state |
| Book Nook | booknook-feed-import | Reviewed import/export, conflicts, atomic commit and separate activation |
| Book Nook | booknook-external-discovery | Chronological external publications with visible provenance and safe cards |
| Book Nook | booknook-feed-qualification | Hostile input, supported-client round trips and native acceptance |
| Shared Castaway, later | castalia-public-feed-publication | Explicit already-public review/list/selected-note output |
| Shared Castaway, later | castalia-feed-component-hints | Allowlisted data hints and trusted installed-renderer fallbacks |

Concrete Tasks are linked to their owning Issues:

1. `castaway-feed-version-decision` → `castaway-feed-codec-fixtures` → `castaway-feed-storage-consumers` → `castaway-feed-selective-projection` → `castaway-feed-host-import`.
2. Shared codecs → `booknook-feed-portable-sdk` → `booknook-feed-source-port` → `booknook-feed-safe-fetch` → `booknook-feed-cache`.
3. Portable SDK and selected disclosure → `booknook-feed-import-activate`; cache and import → `booknook-feed-discovery-ui`.
4. Discovery, accepting-host import and shared encrypted persistence → `booknook-feed-cross-client-acceptance`.
5. Later: `castalia-feed-public-output` and `castalia-feed-safe-hints`.

Arrows above mean “precedes”; actual DevGraph DEPENDS_ON edges point from dependent to prerequisite. Issue-level dependencies mirror these boundaries. Current package qualification depends on the added feed qualification; SDK publication depends on current packages and CI. Shared work has one owner rather than a second Book Nook copy.

## Execution continuation — 2026-11-14

The complete original goal remains active. Public [CI at a9d0fd9](https://github.com/bananawalnut/booknook/actions/runs/43574890030) passed both Apple and portable jobs, including native builds, SDK packaging and reader/recovery/UI workflow checks. This qualifies that source increment, without closing original live/device/release gates.

The latest narrow DevGraph progress patches were committed and independently read back on November 14: Project/booknook v27, Project/castalia-portable-feeds v7, Task/castaway-feed-codec-fixtures v9, Task/castaway-feed-storage-consumers v8 and Task/booknook-feed-portable-sdk v6. These retain draft status. Their descriptions distinguish published source, synthetic test evidence, development artifacts and remaining runtime acceptance.

Shared readers and raw/encrypted fixtures are published on Wallet PR23 at e84428c6, with all hosted checks green. Web PR68 includes guarded encrypted storage at4a312fd and a separately measured all-binding-preserving optimizer trial at2e93ef7. Security, static, unit/contracts, CodeQL and browser accessibility checks pass. The unchanged cold-WASM budget still fails at73.731 seconds against60; the unchanged initial-JS size gate also needs reduction. These are active engineering gates, not exceptions or release passes.

Native source is public in Transfer PR19 atddc016b and iOS PR17 atbc75094. The exact d688a385 native core pin, source digest and each generated library hash are verified. All12 changed shared source/test/fixture/script files match. Four real encrypted native integration tests plus nine review tests pass; the complete Transfer SwiftPM suite passes416 with20 opt-in/live skips. No app packaging, installation, physical device, personal custody or authority change occurred.

Book Nook's Rust/native/Swift/WASM/TypeScript feed SDK has strict original-byte parsing, opaque exact-input reviews and origin-bound projections. Real generated consumers agree on all102 raw vectors and lifecycle cases; independent review found no concrete blockers. This is data-only: host persistence, accepting-site review, source-provider acquisition, normalization/cache, activation and discovery remain separate work. Production v3 export is still refused.

Next dependencies remain: complete real extension/browser persistence and review, selected-origin shared projection and inert accepting-site import, portable Book Nook source provider, safe acquisition/cache and native discovery, then cross-client acceptance and current packages. Public output and renderer hints remain later. No task is accepted merely because its branch exists or a targeted test passes.

## Read current state

Use the configured DevGraph CLI; no write authority is needed for these reads:

```sh
devgraph query work Project booknook
devgraph query arena-of Project booknook
devgraph query children Project booknook --limit 100
devgraph query work Project castalia-portable-feeds
devgraph query children Project castalia-portable-feeds --limit 100
devgraph query dependencies Issue booknook-feed-qualification --limit 100
devgraph query dependencies Task booknook-feed-cross-client-acceptance --limit 100
```

A change in tracked status requires its own evidence. Registering this backlog does not qualify the production MVP.
