# Delivery work in DevGraph

Initial registration was independently read back on **2026-11-13** against Book Nook source `abd2cb1928518be66eda247bea473c98413bb1a2`; the November 14 continuation below records later source and qualification checkpoints. DevGraph is the work tracker; this document is a navigation snapshot, not a replacement tracker or acceptance result.

## Current app and resumed Mac checkpoint — 2026-11-14

Canonical read-back at 22:55 UTC verifies Project/booknook **v45**, Issue/booknook-hands-on-workflows **v3**, Issue/booknook-physical-keyboard-ime **v3**, Issue/booknook-current-packages **v12** and Issue/booknook-ci-evidence **v19**. Five description-only patches prepend concise, record-specific progress and retain every previous description verbatim. Titles, priorities, lifecycle, parentage, dependencies, all **24 Project children** and inherited Castalia membership are unchanged; the existing plan's **46 records and 39 dependency edges** were re-read. SDK-publication **v6**, security **v9** and Castalia-live **v5** remain unchanged. Five unsigned EventReceipts report `pending`: local commit is complete, outbox processing remains pending; external delivery and portable signed attestation are not claimed.

[App preview .5](https://github.com/bananawalnut/booknook/releases/tag/v0.2.0-app-preview.20261114.5) publishes **0.2.0 (2026111405)** from exact source `633f53dcc6d246ba6454b3403bbde10450102ef2`. All ten local Release/archive phases and [all 38 exact-source CI steps](https://github.com/bananawalnut/booknook/actions/runs/43647644605) pass. All 11 actual unauthenticated public downloads match the manifest, totaling 37,558,605 bytes. [Current app evidence](app-2026111405-public-preview-evidence.json) retains the source/archive/runtime distinctions. The six SDK archives and public receiving-consumer proof remain bound to the unchanged `a945df2` SDK release.

Bounded Mac QA on exact build **1404** passed ASCII typing plus native Unicode paste, observed Undo/Redo, EPUB reading, three character-card versions/context, pane swap/resize/scroll, text-whiteboard-text switching and durable normal quit/reopen. Build **1405** separately passed corrected app/ontology labels and one later normal reopen with the same data. Its first startup delayed in `chmod` before SQLite and resumed unaided; the cause remains unresolved. The later reopen is not a performance benchmark or a fix. Earlier input timeout/ownership nonpasses remain preserved. This extends the partial fourth UX round without completing all workflows, ink placement, physical keyboard/IME or accessibility acceptance.

The latest 22:22 UTC read-only inventory found no reachable paired iOS device. Current normal iPad development-signing/trust/install remains required. Physical input, macOS 14/iPadOS 17 runtime, live accounts/Castalia authority, remote social/device exchange, recovery/accessibility/endurance and the two maintenance follow-ups remain open. Notarization/App Store/TestFlight are optional distribution choices. The earlier checkpoints below retain their original dates, sources and availability observations; none closes the production MVP.

## Ownership

- **Arena/castalia → Initiative/booknook-delivery → Project/booknook**: native reading notebook, SDKs and original release qualification.
- **Arena/castalia → Initiative/castalia-storage-publishing-filesystem → Project/castalia-portable-feeds**: shared Castaway feed format, preservation and selected disclosure. Book Nook consumes that work.

The existing minimal Book Nook delivery container is reused. No new initiative was created: DevGraph Projects inherit Arena membership through an Initiative. The separate `Task/booknook-consulting-reading-list` is not duplicated or reparented.

The old October attempt created the container but its expired-authority membership request did not commit; neither the Project nor its 17 prepared issues had been created. November's exact, version-checked requests completed using the existing configured authority, without renewal, new identity or widened grants. Initial read-back verified **42 records** (2 Projects, 26 Issues, 14 Tasks), their parentage, both effective Castalia memberships and **34 dependency edges**. November 14 added the verified cold-build Issue and its prerequisite edge; the plan then contained **44 records and 36 dependency edges**, including the separate Web artifact-storage capacity Issue added after the cold comparison. Records were created in draft status; none is claimed accepted.

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

See [the planned feed contract and rollout](FEEDS.md). The SDK and native Publications source are being delivered in separate qualified increments. Production host exchange and cross-client acceptance remain gated.

| Owner | Issue ID | Scope |
|---|---|---|
| Shared Castaway | castaway-feed-contract | Versioned data-only format, independent codecs, encrypted persistence and cross-client preservation |
| Shared Castaway | castaway-feed-disclosure | Capability negotiation, exact-origin selected projection and inert host import |
| Book Nook | booknook-feed-provider | Portable domain/SDK and composed host source interface |
| Book Nook | booknook-feed-ingestion | Bounded RSS/Atom fetch, normalization, conditional cache and offline state |
| Book Nook | booknook-feed-import | Reviewed import/export, conflicts, atomic commit and separate activation |
| Book Nook | booknook-external-discovery | Chronological external publications with visible provenance and safe cards |
| Book Nook | booknook-feed-qualification | Hostile input, supported-client round trips and native acceptance |
| Book Nook | booknook-feed-string-enums | Reject non-string Serde tag aliases through every feed input SDK; preserve valid literal encodings |
| Shared Castaway, later | castalia-public-feed-publication | Explicit already-public review/list/selected-note output |
| Shared Castaway, later | castalia-feed-component-hints | Allowlisted data hints and trusted installed-renderer fallbacks |

Concrete Tasks are linked to their owning Issues:

1. `castaway-feed-version-decision` → `castaway-feed-codec-fixtures` → `castaway-feed-storage-consumers` → `castaway-feed-selective-projection` → `castaway-feed-host-import`.
2. Shared codecs → `booknook-feed-portable-sdk` → `booknook-feed-source-port` → `booknook-feed-safe-fetch` → `booknook-feed-cache`.
3. Portable SDK and selected disclosure → `booknook-feed-import-activate`; cache and import → `booknook-feed-discovery-ui`.
4. Discovery, accepting-host import and shared encrypted persistence → `booknook-feed-cross-client-acceptance`.
5. Later: `castalia-feed-public-output` and `castalia-feed-safe-hints`.

Arrows above mean “precedes”; actual DevGraph DEPENDS_ON edges point from dependent to prerequisite. Issue-level dependencies mirror these boundaries. Current package qualification depends on the added feed qualification; SDK publication depends on current packages and CI. Shared work has one owner rather than a second Book Nook copy.

## Historical execution checkpoints — 2026-11-14

These earlier checkpoints retain their original source and Work versions; the current delivery checkpoint follows below. At the cache-facade checkpoint, published Book Nook main `a9a672ccfd5ea21b454773aa876cc540b9e7622c` passes both Apple and portable jobs in [exact-revision CI43595156561](https://github.com/bananawalnut/booknook/actions/runs/43595156561), completed November14 at12:53:14UTC. This covers the Rust-native cache and generated native/Swift cache facade, source/acquisition SDK, macOS and both iPad compilation targets, reading/recovery/UI workflows, generated feed/source/XML/offlineHTTPS/cache Swift checks and actual package checks. [Hosted facade evidence](feed-cache-bindings-ci-evidence.json) excludes the later portable export, enum correction and app feed controls. Installed apps, physical devices and minimum-OS runtime acceptance remain separate.

At the earlier progress checkpoint, exact patches were committed and read back on November 14: Project/booknook v39, Project/castalia-portable-feeds v16, Task/castaway-feed-storage-consumers v12, Task/booknook-feed-source-port v7, Task/booknook-feed-safe-fetch v7, Task/castaway-feed-selective-projection v10, Task/castaway-feed-host-import v5, Issue/booknook-ci-evidence v14 and Issue/castaway-feed-web-cold-build v8. New Issue/castaway-feed-web-artifact-storage v3 is parented to the shared Project, inherits Castalia membership and is a prerequisite of Issue/castaway-feed-contract. The string-enum defect adds one independently verified Book Nook Issue and one acceptance prerequisite: the plan then had 45 records and 37 dependencies. Records retain draft status; descriptions distinguish source, synthetic tests, development artifacts and remaining runtime acceptance.

Shared source is merged in [Wallet PR23](https://github.com/ZenithResearch/castalia-wallet/pull/23) at e16dc940, [Transfer PR19](https://github.com/ZenithResearch/zenith-transfer/pull/19) at 5cb22d7 and [iOS PR17](https://github.com/ZenithResearch/zenith-ios/pull/17) at add44321. At that storage checkpoint the canonical checkouts were clean main and only their owned merged feature branches were removed locally/remotely. The subsequent pure projection primitive is merged in [Wallet PR24](https://github.com/ZenithResearch/castalia-wallet/pull/24) at ef11cfe1; its merged branch is removed, and its standalone bindings are merged in [Wallet PR25](https://github.com/ZenithResearch/castalia-wallet/pull/25) at ba55cbc8 after four exact-head checks passed. The owned merged binding branch is removed locally/remotely; the unregistered host controller is merged in [Wallet PR26](https://github.com/ZenithResearch/castalia-wallet/pull/26) at a9234ebe after all four exact-head checks passed. Its merged tree matches the reviewed 25710ba and its owned branch is removed locally/remotely. A separate default-disabled Chrome adapter was active work at that checkpoint. Unrelated worktrees and branches remain untouched. No runtime deployment or app installation is inferred.

Wallet extension qualification passes 450 tests with 4 opt-in skips and 12 actual disposable Chrome storage checks. Final custody-epoch/cancellation fences cover invalidation after the last awaited storage read. Native consumers pin d688a385; shared source digests and generated-library hashes are verified. Four actual encrypted native tests cover restart, legacy/empty preservation, identity edits, stale CAS and SQLite rollback; full Transfer SwiftPM passes 416 with 20 opt-in/live skips. These are source/domain checks, without iOS runtime, physical-device or installed-package qualification.

[Web PR68](https://github.com/ZenithResearch/castalia-web/pull/68), currently a1173c9, introduced guarded lossless encrypted storage and import review at b6ff626. The full Web suite passes 226 with 8 opt-in skips, and 16 actual Chrome/Worker/IndexedDB checks cover restart, cross-tab CAS, callback cancellation/expiry, owner refusal and rollback. The private build-only v3 override has a default negative control; production admission/export guards remain unchanged. Quota denial is injected at the actual put. Public source hashes and limitations are in Web docs/feed-storage-evidence.json.

Terser and Binaryen O1 preserve generated bindings, Deflate64 fallback and typography. The unchanged JavaScript budget passes at 152,106/153,600 gzip bytes. Earlier hosted cold-WASM failures remain explicit. A single [comparison run 43584517919](https://github.com/ZenithResearch/castalia-web/actions/runs/43584517919) passed the unchanged source-build and startup checks on both the original x64 job and a separate empty-target ARM job; successful enclosing prerequisite durations were not emitted. This establishes neither reproducibility nor an ARM advantage. The comparison job was removed, restoring the original workflow byte for byte at Web a1173c9. That run failed later on production-artifact upload because GitHub storage quota was exhausted, so dependent release jobs skipped. The next unchanged original x64 run at a1173c9 (workflow43585277590/job132418319008) again failed the cold prerequisite at73,569.0/60,000ms. Both cold-build repeatability and artifact capacity remain unresolved. PR68 remains open; a dedicated shared Issue tracks capacity. No artifacts, billing, thresholds, public exports, cryptography or production-v3 guards changed. See Web docs/feed-cold-build-trials.md for bounded evidence.

Book Nook’s published Rust/native/Swift/WASM/TypeScript review SDK agrees on all 102 raw vectors and lifecycle cases. The additive source-provider SDK passes 117 core tests, focused source/native cases, strict Clippy, actual Node/web-target WASM consumers and fresh generated Swift checks. Conditional 304 requires matching retained cache bytes; activation, generation, cancellation, supersession and guarded final CAS are tested. Independent review verified all 18 final source files without a concrete blocker. The unchanged extracted-package suite also passes. Source bytes remain untrusted. The concrete safe HTTP/XML increment was qualified separately below; durable storage and app feed interfaces were still under implementation at this earlier source-provider checkpoint. No app feed interface or production v3 writer was enabled by that checkpoint.

The bounded acquisition increment now passes 132 core tests, 27 XML cases through actual native/Node/web-target WASM/fresh Swift bindings, strict Clippy and a real production HTTPS → UniFFI → XML fetch (164,045 bytes, 10 inert entries). The complete SDK suite also passes, including original archive/social/storage/provider regressions. Actual extracted packages pass 254 tests with five declared skips across all four crates; installed TypeScript CJS/ESM/NodeNext consumers pass and all 72 files match the archive. All 404 frozen source inputs remained unchanged. [Public package evidence](feed-ingestion-package-evidence.json) records local development provenance. The exact committed source subsequently passes the complete hosted workflow linked above; this does not retroactively turn the earlier local archives into clean release artifacts.

D131 keeps the existing shared feed-section payload and Book Nook wrapper. The pure shared selection primitive passes 21 focused Rust tests and strict Clippy on pinned Rust 1.85.1, plus 450 npm tests with four existing skips and typecheck. All four PR24 checks passed before merge. Opaque exact-source/destination preparation remains separate from host custody, consent and transmission. Existing Book Nook WASM produces matching inner sections for all four common positive shared vectors, retains its six stricter-origin refusals and rejects the 36 negative vectors without traps. The three confirmation vectors explicitly preserve the difference between shared exact source bytes and Book Nook canonical configuration/revision. No shared host adapter is qualified by this compatibility check.

The Rust-native cache foundation passes its [initial focused qualification](feed-cache-evidence.json) and both exact hosted jobs above. The additive UniFFI/Swift facade separately passes [foreign-consumer qualification](feed-cache-bindings-evidence.json):20 cache unit cases, three durable integration scenarios plus one ordinary child helper, an exported facade test, strict all-target Clippy and actual fresh/preserved matching generated Swift consumers. Synchronous cancellation fences held/queued SQLite work and active source tokens, preserves accepted acknowledgements and closes canceled opening. The facade was covered by a9a672c hosted evidence; app acceptance remained pending at that checkpoint. Its recorded Work versions were Task/booknook-feed-cache v8, Task/booknook-feed-import-activate v8 and Issue/booknook-feed-ingestion v9.

The origin-free export SDK now passes its [focused parity and lifecycle checks](feed-export-evidence.json), with raw section reimport, unchanged origin rules and old TypeScript adapter compatibility. Native Publications source separately passes actual focused/full app workflows, a WKWebView-anchor check and a macOS Debug build. The matching development native library contains macOS 15.2 minimum objects; this does not qualify the declared macOS 14 floor or a release package. Actual hands-on UI review stopped before interaction because the Mac is locked. No physical device or personal library was touched.

[The literal-string correction](feed-string-enums-evidence.json) now passes 51 core and 17 native tests with one declared live-network skip, actual fresh/preserved Swift and Node/browser-WASM consumers, strict all-target Clippy and 46 resource mirrors. Valid saved configuration reopens; malformed historical aliases refuse without a reset or byte rewrite. Parent verified all 49 source/artifact hashes. Issue/booknook-feed-string-enums is v5. The [complete SDK and extracted packages](feed-export-enum-package-evidence.json) now pass: 290 Rust tests with five declared skips, plus installed TypeScript CJS/ESM/Node/browser-WASM/NodeNext consumers. All 409 frozen inputs stayed unchanged during those checks, and all 72 installed npm files match the archive. These are local working-tree development artifacts, with no registry publication or clean release attestation.

The unregistered shared controller passes 511 npm tests with four existing opt-in skips and typecheck, including actual synthetic encrypted/WASM/FIFO cases. This does not register or enable a page API. D137 keeps the next Chrome adapter behind a separate false-by-default capability and explicit destination support predicate; selection and consent are separate, with one 120-second lifecycle and 16 actual-work slots. Production v3 guards and Web PR68 remain unchanged.

## Published preview checkpoint — 2026-11-14

Canonical post-publication read-back records Project/booknook **v42**, Project/castalia-portable-feeds **v17**, Issue/booknook-negentropy-security **v9**, Issue/booknook-sdk-publication **v6**, Issue/booknook-current-packages **v9**, Issue/booknook-ci-evidence **v16** and Issue/booknook-ontology-publication **v4**. Feed progress remains Task/booknook-feed-import-activate **v9**, Task/booknook-feed-discovery-ui **v8** and Task/castaway-feed-selective-projection **v11**. The plan retains **46 records and 39 dependency edges**. Five description-only patches recorded the completed preview evidence; titles, priorities, lifecycle, parentage, dependencies, 24 Project children and inherited Castalia membership were preserved. No Work acceptance is inferred.

Both pre-patch hosted jobs passed at [732c6a4](publications-release-ci-evidence.json); earlier [995b36bd evidence](feed-export-enum-ci-evidence.json) retains its own scope. The [pre-patch native Release increment](native-release-20261114-evidence.json) passed all three builds, app workflows, archive inspection and an extracted Swift consumer, correcting the historical Debug library's above-floor objects. Neither that build 2026111401 nor its CI qualifies the security replacement. Minimum-OS runtime, current hands-on UI and accessibility remain open. Xcode reports three paired iOS devices, with none reachable; no profile renewal or device change was attempted.

The guarded Wallet adapter at PR27 head `49b2562773dd6e3b1c03adb53d4a6f454c75fa4d` passes 48 focused and 559 full tests with four existing skips, typecheck and actual production-package inspection. Eleven disposable Chrome groups include ten [independent Book Nook receiver cases](feed-browser-receiver-evidence.json), exercising actual packaged WASM and IndexedDB rather than a mocked SDK. Production capability remains disabled and destination support defaults to refusal. Existing v3 guards, runtime pins and Web PR68's cold-build/artifact-capacity prerequisites remain unchanged.

The selected independent ontology host is GitHub Pages in the existing public repository. [Staging evidence](ontology-pages-staging-evidence.json) qualifies the public-only 82-file stage and its negative tests. Pages deployment `43603595461` at `732c6a4` and [public response checks](ontology-pages-public-evidence.json) now pass: all 80 files match the source and all three Release offline bundles, with verified TLS, JSON/JSON-LD MIME, CORS and five nonstaged-path 404s. The Raw mirror remains a separate historical byte-download check. The first SDK/app preview was published and then withdrawn to draft; its 17 assets/tag are preserved as [historical publication evidence](public-preview-publication-evidence.json). The unchanged ontology remains qualified independently of that withdrawal.

The priority-1 `Issue/booknook-negentropy-security` remains under Book Nook with inherited Castalia membership, and SDK publication/current-package acceptance both depend on it. It now covers the broader official audit: workspace 11 vulnerability findings plus seven informational findings (three unsound), probe six plus five. The negentropy 0.5.3 regression and successful source `130d5dba` CI remain partial pre-broader-audit history, not an all-clean/publication result. Current fetch/send paths still do not demonstrate NIP-77 parser reachability.

Maintained Nostr SDK 0.45.6/lru 0.18.2 and the Rust transport/type migration target unified app/SDK **0.2.0**, build **2026111403**, tag `v0.2.0-preview.20261114`. [Exact-lock audits/registry checks](security-dependency-audit-evidence.json) pass with maintenance notices retained. The 291-test Rust migration, [17 Apple/21 portable a945 CI](security-020-ci-evidence.json) and [nine-phase native Release/Swift/SQLite qualification](native-020-release-evidence.json) now pass. Selected local Rust/npm archives also pass all 52 Rust test binaries (291 passed, five skips) and installed typed/WASM checks. The [replacement preview](https://github.com/bananawalnut/booknook/releases/tag/v0.2.0-preview.20261114) is published with all 17 server asset identities matching; all 17 unauthenticated downloads match. All six receiving install/smoke/typecheck commands now pass after the fresh 333-package lock (25 newer compatible versions) triggered a pre-compilation source-subset guard. Retain that nonpass, separate exact receiving-lock audit and unchanged-lock approved continuation. This is offline resolution; fresh online resolution is not qualified. Held 0.1.1 candidates were never published. Ontology/wire/saved-data formats are unchanged; Rust source compatibility changes.

The active plan is: reconcile the qualified archive checksums/notices → retain the published manifest, actual public receiving proof and canonical Work progress → continue external gates. Audit/CI/native proof is retained without repeating passed work absent changed inputs. Maintenance notices remain explicit; no documentation update accepts Work. Hardware, minimum-OS runtime, live accounts/authority, remote social and cross-device gates remain open. Public feed output/component hints remain later work; the active goal/objective is unchanged.


## Historical profile-import source follow-up — 2026-11-14

This source-only checkpoint preceded the fixed app-only release below; its pending package observations describe that earlier stage.

The subsequent four description-only updates were independently read back as Project/booknook **v43**, Issue/booknook-castalia-live **v4**, Issue/booknook-current-packages **v10** and Issue/booknook-ci-evidence **v17**. They record source `995bf0df36f3a638fd185f704c19f41af4269cd6`, the actual old FIFO hang and [fixed-reader evidence](profile-import-file-evidence.json): 14 local file cases, the full native workflow, both iPad typechecks and all 38 steps in [exact-source CI43624695716](https://github.com/bananawalnut/booknook/actions/runs/43624695716), including the hosted FIFO regression. Original descriptions remain verbatim below each checkpoint. Titles, priorities, lifecycle, relationships, 24 Project children and inherited Castalia membership are unchanged.

The published a945 preview still lacks this app-only fix; its notes disclose the limitation and all 17 assets/tag are preserved. SDK source and artifacts are unchanged. Fresh native Release app archives for build **2026111404** remain pending; assigning a build number does not qualify compilation, packaging or installation. Current input/accessibility, live account/authority, remote social/device exchange and minimum-OS runtime gates remain open. Four unsigned EventReceipts report `pending`: local mutation committed, outbox processing pending, without a claim of external delivery.

## Historical fixed app-only preview checkpoint — 2026-11-14

At this earlier checkpoint, canonical read-back recorded Project/booknook **v44**, Issue/booknook-castalia-live **v5**, Issue/booknook-current-packages **v11** and Issue/booknook-ci-evidence **v18**. Four description-only patches put the new delivery checkpoint first and retain each previous description verbatim below it. Titles, priorities, lifecycle, parentage, dependencies, all 24 Project children and inherited Castalia membership are unchanged. SDK-publication **v6** and security **v9** remain unchanged because this app-only batch does not replace SDK or dependency evidence. Four unsigned EventReceipts report `pending`: locally committed, with outbox processing pending; external delivery is not claimed.

Book Nook **0.2.0 (2026111404)** now includes the profile-file correction in three fresh arm64 Release app archives at exact source [7590973](https://github.com/bananawalnut/booknook/commit/759097365d933ddeb9fc4d37e09481380ea64eba). All ten local build/archive verification phases pass, including full archive extraction/tree and platform/deployment metadata checks using unchanged qualified Release Rust libraries. [Exact-source CI43626272370](https://github.com/bananawalnut/booknook/actions/runs/43626272370) also passes all 17 Apple and 21 portable steps. Local Release artifacts and hosted source-build checks retain their separate scopes.

The distinct [app-only prerelease](https://github.com/bananawalnut/booknook/releases/tag/v0.2.0-app-preview.20261114.4) was published at 18:54:24 UTC, with its tag pointing directly to `7590973`. All 11 server assets matched their expected names, sizes and hashes: three build-qualified app archives and eight supporting files. The [app release manifest](https://github.com/bananawalnut/booknook/releases/download/v0.2.0-app-preview.20261114.4/RELEASE-MANIFEST.json), SHA-256 `139da3f971b6837857aead4a7f2a02e6bc9ddba44581b759980f7f67478794eb`, binds those exact sources and artifacts. All 11 actual public HTTPS downloads then matched at 18:54:47 UTC, totaling 37,549,753 bytes, with TLS verification and no ambient proxy, authentication, cookies or client certificate. No local app substitution was used.

The six SDK archives and their public receiving-consumer qualification remain attached to source `a945df2` and the [separate SDK preview](https://github.com/bananawalnut/booknook/releases/tag/v0.2.0-preview.20261114). They were not rebuilt or repackaged. Older app build 2026111403 remains historical and lacks the profile-reader correction. The new app download checks establish byte availability, not installation or execution. Device archives are unsigned, not installable signed IPAs; linker ad-hoc signatures do not establish production signing, notarization or store distribution.

The final 18:56:15 UTC availability check found the Mac locked and no reachable paired iOS device. Installed iOS Simulator runtimes are 18.6/26.4.1; the iOS 17 runtime is absent. These are Simulator versions, not paired-device OS observations. No orientation, UI input, launch, install, signing, credential or global-toolchain action occurred. Physical input/accessibility, current hands-on workflows, actual macOS 14/iPadOS 17 runtime, live accounts/Castalia authority, remote social exchange and cross-device synchronization remain open. The two dependency-maintenance notices remain actionable; this delivery does not accept the production MVP.

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
