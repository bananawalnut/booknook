# Delivery work in DevGraph

Reconciled and independently read back on **2026-11-13** against Book Nook source `abd2cb1928518be66eda247bea473c98413bb1a2`. DevGraph is the work tracker; this document is a navigation snapshot, not a replacement tracker or acceptance result.

## Ownership

- **Arena/castalia → Initiative/booknook-delivery → Project/booknook**: native reading notebook, SDKs and original release qualification.
- **Arena/castalia → Initiative/castalia-storage-publishing-filesystem → Project/castalia-portable-feeds**: shared Castaway feed format, preservation and selected disclosure. Book Nook consumes that work.

The existing minimal Book Nook delivery container is reused. No new initiative was created: DevGraph Projects inherit Arena membership through an Initiative. The separate `Task/booknook-consulting-reading-list` is not duplicated or reparented.

The old October attempt created the container but its expired-authority membership request did not commit; neither the Project nor its 17 prepared issues had been created. November's exact, version-checked requests completed using the existing configured authority, without renewal, new identity or widened grants. Initial read-back verified **42 records** (2 Projects, 26 Issues, 14 Tasks), their parentage, both effective Castalia memberships and **34 dependency edges**. November 14 added the verified cold-build Issue and its prerequisite edge; the plan now contains **43 records and 35 dependency edges**. Records were created in draft status; none is claimed accepted.

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

The complete goal remains active. Public [CI at a9d0fd9](https://github.com/bananawalnut/booknook/actions/runs/43574890030) passed both Apple and portable jobs. The subsequent c92c3e9 binding increment exposed an issuer process-test race, repaired at 3f480a6. [That hosted run](https://github.com/bananawalnut/booknook/actions/runs/43579104645) passes Apple and reaches a separate extracted-crate fixture-path failure in portable packaging. The narrow fixture-path repair now passes the unchanged local script against all four actual extracted crates; exact-commit hosted verification is still pending. No newer full hosted pass is claimed.

The latest exact progress patches were committed and read back on November 14: Project/booknook v29, Project/castalia-portable-feeds v10, Task/castaway-feed-codec-fixtures v9, Task/castaway-feed-storage-consumers v10, Task/booknook-feed-portable-sdk v7 and Issue/booknook-ci-evidence v8. They retain draft status. Current canonical descriptions distinguish source, synthetic tests, development artifacts and remaining runtime acceptance.

Shared source is merged in [Wallet PR23](https://github.com/ZenithResearch/castalia-wallet/pull/23) at e16dc940, [Transfer PR19](https://github.com/ZenithResearch/zenith-transfer/pull/19) at 5cb22d7 and [iOS PR17](https://github.com/ZenithResearch/zenith-ios/pull/17) at add44321. Those canonical checkouts are clean main; only their owned merged feature branches were removed locally/remotely. Unrelated worktrees and branches remain untouched. No runtime deployment or app installation is inferred.

Wallet extension qualification passes 450 tests with 4 opt-in skips and 12 actual disposable Chrome storage checks. Final custody-epoch/cancellation fences cover invalidation after the last awaited storage read. Native consumers pin d688a385; shared source digests and generated-library hashes are verified. Four actual encrypted native tests cover restart, legacy/empty preservation, identity edits, stale CAS and SQLite rollback; full Transfer SwiftPM passes 416 with 20 opt-in/live skips. These are source/domain checks, without iOS runtime, physical-device or installed-package qualification.

[Web PR68](https://github.com/ZenithResearch/castalia-web/pull/68) at b6ff626 adds guarded lossless encrypted storage and import review. The full Web suite passes 226 with 8 opt-in skips, and 16 actual Chrome/Worker/IndexedDB checks cover restart, cross-tab CAS, callback cancellation/expiry, owner refusal and rollback. The private build-only v3 override has a default negative control; production admission/export guards remain unchanged. Quota denial is injected at the actual put. Public source hashes and limitations are in Web docs/feed-storage-evidence.json.

Terser and Binaryen O1 preserve generated bindings, Deflate64 fallback and typography. The unchanged JavaScript budget now passes at 152,106/153,600 gzip bytes. The separate hosted cold-WASM prerequisite still fails at 74,704.6/60,000 ms. **Issue/castaway-feed-web-cold-build** is parented to the shared Project, verified under Castalia and required by Issue/castaway-feed-contract. Private compiler comparisons did not establish a cold pass; no budget, crypto or public-export waiver was used.

Book Nook’s published Rust/native/Swift/WASM/TypeScript review SDK agrees on all 102 raw vectors and lifecycle cases. The additive source-provider SDK passes 117 core tests, focused source/native cases, strict Clippy, actual Node/web-target WASM consumers and fresh generated Swift checks. Conditional 304 requires matching retained cache bytes; activation, generation, cancellation, supersession and guarded final CAS are tested. Independent review verified all 18 final source files without a concrete blocker. The unchanged extracted-package suite also passes. Source bytes remain untrusted; concrete safe HTTP/XML acquisition and durable cache are separate work. No app feed interface or production v3 writer is enabled.

Next: exact-commit hosted SDK/package checks; exact-origin shared projection and inert accepting-host import; safe acquisition/cache and native discovery; then cross-client acceptance and current packages. Public output and renderer hints remain later. All 17 original live/hardware/release issues stay open. No task is accepted merely because a branch exists or a targeted test passes.

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
