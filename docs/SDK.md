# Using the open SDKs

The portable document and social contracts live in Rust. The reference apps use the same command execution and validation that SDK hosts use. A host can provide storage or source adapters without operating a Book Nook server. Account ownership, catalog metadata, accessible content, source positions, directory attribution and authority are separate concerns.

## Rust notebook storage

The native SQLite adapter and a custom host adapter share `NotebookStorage`:

```rust,no_run
use booknook_core::{Command, providers::Notebook};
use booknook_native::SqliteNotebookStorage;

let storage = SqliteNotebookStorage::open(std::path::Path::new("./my-desk"))?;
let notebook = Notebook::open(storage)?;
let response = notebook.execute(Command::AddBook {
    title: "A book from my library".into(),
    author: "An author".into(),
    at: 1_700_000_000,
})?;
let book_id = response.result["book_id"].as_str().unwrap();
assert!(response.state.books.contains_key(book_id));
# Ok::<(), Box<dyn std::error::Error>>(())
```

To replace SQLite, implement `Provider` and `NotebookStorage`, initialize a valid `State` in your store, then pass that implementation to `Notebook::open`. `load` returns the complete current state. `compare_and_save(expected_revision, state)` must commit atomically only when the stored revision matches; it must return an error without changing storage on conflict or failure. The core validates inputs and outputs and returns command success only after that commit. It does not silently retry competing edits.

`crates/core/tests/providers.rs` demonstrates an independent host store, including a competing writer and failed save. `crates/native/tests/provider_storage.rs` proves that the public SQLite adapter and the actual native `Desk` observe the same data and reject stale writes. The native reference composition additionally provides a durable pending-edit journal, asset storage, custody, social state and encrypted transfer operations. Replacing that whole composition requires implementing those operations; supplying a document store alone does not imply those capabilities.

## Swift and native operations

UniFFI generates `apps/apple/Generated/booknook_native.swift` and its C header/module map. `scripts/build-apple.sh` regenerates these from the actual Rust library. `Desk` is the native reference composition; it accepts a local directory and exposes commands, snapshots, imports, history, custody, social and synchronization operations. Invoke disk, cryptography and network work outside the main actor, as the reference app's `DeskRepository` does.

For an independent app, use `scripts/package-swift.sh` to produce the standalone `booknook-swift-sdk-0.1.0.tar.gz`, then add its extracted directory as a local Swift Package. `BookNookNative` exposes the Rust bridge, `BookNookServices` provides asynchronous provider composition, and `CastaliaNative` supplies reusable custody, secure-storage and consent coordination. It includes arm64 macOS, iOS device and iOS Simulator slices, generated Swift and headers. Its consumer check extracts the actual archive before linking; the package does not depend on the reference app's source tree.

Local command results contain `state` and `result`. Use IDs returned by `result` rather than searching by display title. Keep an editor draft until a throwing save acknowledges persistence. Custody unlock does not authorize a publication: use the prepared disclosure and explicit consent lifecycle. Do not infer operational permission from a directory identity or a Nostr signature.

## WASM and TypeScript

Build the browser-targeted core with `bash scripts/build-wasm.sh web`; the SDK checks use the Node target. `BookNook.open(core, storage)` accepts an asynchronous host store. `compareAndSave` must be atomic; `null` means create only if the record does not already exist. Social storage has the same rule and must commit approval, replay consumption and outbox updates together before transport.

`scripts/package-typescript.sh` creates an installable local npm archive containing ESM/CommonJS JavaScript, declarations and separate browser/Node WASM runtimes. Import the facade from `@booknook/sdk`; Node imports the initialized runtime from `@booknook/sdk/wasm`, while browser hosts initialize `@booknook/sdk/wasm/web`. See the [JavaScript package guide](../sdk/typescript/README.md). `scripts/package-rust.sh` similarly builds and tests actual extracted `.crate` archives with local patches for unpublished Book Nook siblings. No registry publication is assumed.

The facade validates JavaScript's exact integer boundary. Raw JSON preserves Rust integer values, and WASM time arguments use unsigned 64-bit `bigint` values. Private document state and social state have separate schemas. Audio matching APIs return explainable edition candidates; confirmed association does not convert narration time into an ebook location.

### Recommendations and approved encrypted transfer

`recommendations(core, input)` runs the shared Rust recommendation provider with catalog candidates, chosen interests, exclusions and independently verified shared ratings. Its input has no notebook, annotation, character-card or handwriting fields. `socialRecommendations(core, socialState, candidates, excluded, policy, now)` additionally rebuilds signed history under the host's current issuer pins and rejects forged friendship/group projections, invalid receipts or revoked trust before scoring. The policy actor is the viewer; other authors are checked using the same trusted pins, audience/caller and `social.execute` policy. Private group ratings and blocked readers do not contribute. Use the local API with no shared ratings when no identity is configured; verification failures do not silently turn untrusted social data into evidence.

The portable encrypted-transfer API uses the existing native `booknook.sync-packet.v1` wire:

- `prepareSyncBatch(core, actor, revision, recipients, plaintext)` hashes the exact `Uint8Array` bytes; it neither signs nor sends. Revisions and times passed to WASM use unsigned 64-bit `bigint`; the typed facade rejects results that exceed JavaScript's exact integer range.
- After explicit review through the host's custody integration, `sealSyncBatch(core, batch, plaintext, pairingKey, requestJSON, presentationJSON, policy, now)` verifies the fresh Castalia receipt, actor, `sync.send` action, revision and byte digest before encryption. It returns `{packet, replay_key}`. The pairing key is exactly 32 bytes, distinct from a signing key, and is never included in the packet.
- `openSyncBatch(core, packet, pairingKey, recipient, policy, now)` verifies historical approval using current independent issuer pins before decrypting. It returns `{plaintext: Uint8Array, packet_digest, replay_key}`. An expired *new* approval is rejected; a previously approved packet remains receivable after that receipt's expiry unless trust was revoked. Future approval, changed recipients, ciphertext, request or presentation fail verification.

These functions are pure packet operations, not an automatic synchronization service. The host must atomically consume the approval `replay_key` and store the exact outgoing packet before transport. Retry the stored packet instead of re-encrypting it. On receive, independently check the paired source and authorized recipient, validate the plaintext's schema/source/revision and atomically apply it with batch ID/digest bookkeeping. Equal IDs/digests are idempotent; a reused ID with different contents is rejected. Decoding alone does not consume replay IDs or grant permission to overwrite a notebook. Native `Desk` retains these peer, SQLite transaction and conflict guards and uses the same shared packet verifier. The native plaintext is `booknook.sync-content.v1` with `source_device`, complete `state` and its `assets`; arbitrary bytes are useful to portable hosts but do not qualify as a native desk snapshot. The content bound remains 64 MiB.

Raw WASM counterparts are `recommendations`, `social_recommendations`, `prepare_sync_batch`, `seal_sync_batch` and `open_sync_batch`. Raw packet/request JSON is parsed strictly; keep original JSON when duplicate-field rejection matters. These additions do not change archive, ontology or native transfer formats, and do not expose native signing secrets. `tests/portable-services-parity.cjs`, the typed SDK checks, and native transfer tests exercise both directions of the shared packet contract and its rejection paths.

## Sources and capabilities

Providers advertise what they actually supply. `Catalog` never implies `Content` or `Positions`. The companion's capability response controls available native source actions. Missing adapters, unsupported renewal and unqualified access stay visible. A provider's self-reported qualification flag is descriptive, not independent release evidence or authority.

Rust hosts use `booknook_connectors::amazon::CompanionClient::audible_positions_in_marketplace(token, marketplace, asins, now)` for an explicit `us`, `uk`, `de`, `ca` or `au` account selection. The existing `audible_positions` method preserves its original request and legacy US default. `provider_connections(token, marketplace)` fetches strictly validated shared Castalia descriptors from companions advertising `provider_connections_v1`; an unsupported route remains an error. These descriptors carry nonsecret references, not account access or signing authority. Older external adapters retain their own account-selection limits. See the [connector crate guide](../crates/connectors/README.md) for compatibility and transport bounds.

`CatalogRecommendations` implements the public recommendation port used by the reference app. Its input cannot include private notebook content. A host must independently verify shared social evidence before placing it in recommendation inputs, as the native bridge does.

### Executable provider composition

The [provider composition guide](PROVIDER-COMPOSITION.md) includes registration examples and the independently versioned provider schemas and conformance fixtures. Existing domain ontology releases remain unchanged.

The reference Apple app constructs the public Swift `BookNookServices` actor and injects it into `DeskRepository`. A host supplies `BookNookProvider` implementations with immutable descriptor JSON and an asynchronous `execute(requestJSON:)` method. `Desk.withProviders` is the corresponding native constructor; `providerFetch` validates source results without changing the notebook, and separate synchronous commit methods revalidate the captured generation and current connection settings. SQLite checks settings and state revision inside the write transaction, including changes through another desk handle. Existing `Desk(directory:)` and direct methods remain compatible.

The portable `booknook.provider-registry.v1`, request, response and execution contracts live in `booknook_core::provider_runtime`. They distinguish catalog, content, positions, identity, recommendations and social transport. Original JSON reaches the strict shared parser before platform decoding; unknown/duplicate fields, mismatched capabilities, changed provider/connection identity and stale registrations fail. A descriptor registers no executable code and grants no permission. The host supplies the implementation explicitly.

Catalog operations accept normalized metadata or the existing Kindle/Goodreads formats. Reconciliation preserves legacy identifiers, source provenance and newer personal edits. Catalog access alone cannot claim content or source positions; legacy Kindle files containing progress require the explicit positions capability too. Content carries its source, media type, SHA-256 and separate bounded bytes. Native commits parse the actual EPUB/PDF and ingest assets. WASM validates the envelope and hash only; browser hosts must validate and persist assets themselves before claiming that content is readable.

`source_positions` preserves opaque source IDs, location strings and observation times. Its namespace comes from the registered provider. `attachPosition` records the reader-selected pass with unmatched alignment; it never converts audio percentages into ebook progress. The existing Audible `positions` operation retains edition candidate review and explicit confirmation. Identity resolution returns directory attribution, including `Actor` normalization to `Entity`; it cannot authorize an action.

The TypeScript `BookNookServices.open(core, storage, providers)` facade uses these same validators through WASM and the existing atomic `NotebookStorage`. `importCatalog` loads fresh state after fetching, rechecks cancellation/registration, then commits with CAS. `sourcePositions` and `attachPosition` follow the same rules. Registration replacement invalidates pending reads and is refused during a local commit. Once a commit begins and succeeds, late cancellation does not misreport it as unsaved. A provider-owned byte buffer is copied before hashing. Supply a unique `requestID` factory if the host lacks `crypto.randomUUID`.

Native `providerSocialExchange` consumes the registered social transport while retaining Nostr signature, Castalia binding, membership, replay, encryption and outbox checks. The transport receives already signed packets, never signing secrets, and only acknowledged events advance delivery. The native recommendation path verifies shared social history before passing its minimal projection to the selected recommender. Neither path passes private notebook contents to a provider. The built-in IDs are `booknook.nostr` and `booknook.local-recommendations`.

Swift cancellation does not automatically cancel UniFFI foreign work. The facade therefore separates asynchronous reads from synchronous commits and checks cancellation immediately before mutation. A cancelled network operation may finish in the background, but cannot commit its catalog/content/position result through the cancelled caller. Social delivery is different: a packet already sent cannot be recalled. No SQLite lock spans provider or consent awaits.

### Reusable Castalia custody

`CastaliaNative` is independent of `Desk`, SQLite and a Book Nook account. It accepts host-owned secure storage, current trust policy, issuer request source and consent presentation. The reference app consumes the same source implementation. Read the [Castalia component guide](../sdk/swift/CASTALIA.md) for construction and migration. Existing recovery envelopes, Keychain service/account names and Castalia v2 signing transcripts remain unchanged.

Rust `validate_castalia_request` / `verify_castalia_receipt` and TypeScript `validateCastaliaRequest` / `verifyCastaliaReceipt` bind the issuer response to the host's exact canonical request and disclosure under independent holder/caller/audience/policy/pins. These generic functions do not grant application permissions or consume replay keys. Receivers must validate their own operation and atomically consume the returned replay key. Existing Book Nook authorization wrappers still require its action, actor, payload and version. WASM exposes verification only; native custody retains the private key and signs only a prepared, explicitly approved presentation.

## Standalone native and Swift notebook storage

`booknook_native::NotebookEngine` exposes the same core `providers::snapshot` and `providers::execute` engine to UniFFI hosts. `NotebookEngine::with_storage(host)` accepts `NotebookStorageHost`; `NotebookEngine::local(directory)` supplies the reference SQLite adapter. The public Swift `BookNookNotebook(storage:)` / `BookNookNotebook(localDirectory:)` facade and `BookNookNotebookStorage` protocol live in `BookNookServices`. See the [Swift package guide](../sdk/swift/README.md) for construction.

The host supplies `descriptor_json`, `load_json` and atomic `compare_and_save(expected_revision, state_json)`. The descriptor keeps the existing core `ProviderDescriptor` JSON shape: nonempty ID (at most 256 UTF-8 bytes, no controls), unique capabilities including `notebook_storage`, availability, and optional reason (at most 4,096 bytes). Descriptor JSON is limited to 16 KiB and rejects unknown and duplicate fields. State/command JSON uses the existing strict 128 MiB parser and semantic validation. The host initializes valid state before opening; this API does not silently create or overwrite an existing external record.

Each operation checks a fresh descriptor and state, and saving rechecks descriptor identity and capability. A callback returning `false` becomes a conflict; an exception becomes a storage error. Neither returns the candidate state as successful, retries the command, nor overwrites an external writer. A host returning `true` must have durably committed the entire state atomically with every writer. These synchronous callbacks have no Rust mutex held across them; the host must provide its own safe I/O and locking. The Swift facade preserves declared errors through UniFFI. Run storage work off the UI actor.

This is an additive standalone state port, not replacement of the composed reference `Desk`. `Desk` continues consuming the same core storage interface through its local SQLite adapter, retaining its transactional settings/social/assets and write-ahead edit journal. The Apple app is not routed around that journal. Standalone hosts manage assets, publication and any text-edit recovery separately; they must not share the standalone SQLite directory with an active `Desk`. Native alternate-file-host tests and the installed Swift consumer exercise persistence, availability changes, failed saves and an actual competing writer without accounts or network access.

## Offline conformance and distribution

`ontology/v0.1.0/conformance.json` is a published, checksummed bundle of valid and invalid synthetic vectors. Its raw JSON case tests duplicate-key rejection; do not parse and reserialize that input before validation. State validity includes semantic references and CRDT projections beyond JSON Schema shape. `Actor` is accepted only as the Rolodex compatibility alias for `Entity`; a literary `Character` is not a directory `Person`.

Run `scripts/check.sh` and `scripts/test-sdk.sh` to exercise the contracts. Native framework/workflow checks are separate. See [deployment](DEPLOYMENT.md), [licensing](LICENSING.md), and the [validation record](VALIDATION.md) for build packaging and the boundary between tested behavior and external release qualification.

### Notebook handwriting contracts (ontology 0.2.0)

`Block.ink` is optional for existing typed documents. It contains `schema: "booknook.notebook-ink.v1"`, immutable `versions`, and independent `transcriptions`. A version stores its ID and parent IDs, reader/time/reading-pass context and source position observations, logical canvas dimensions, portable strokes with `#rrggbb` colors, and optional opaque canonical-base64 native drawing. Coordinates remain in the fixed logical page across rotation and reflow; these are notebook canvases, not edition-specific reader anchors. A transcription references one exact version and records its own ID, reader, time, text and `manual` or `on_device` method. No transcription command changes ink, prose, publication or authorization. Portable strokes retain centerlines, pressure, timing, color and width; native brush texture and opacity remain in the original native bytes and may be approximated outside PencilKit.

`set_notebook_ink` accepts notebook/block IDs, optional `version_id`, `parents`, canvas dimensions, strokes, optional native data and `at`; it returns `{ink_version_id, heads, conflict, replayed}`. `save_notebook_transcription` accepts notebook/block IDs, optional `transcription_id`, `ink_version_id`, text, method and `at`, returning `{transcription_id, replayed}`. Callers retain the original command and stable ID for retries; exact replay does not increment revision or recapture position observations. Reusing an ID for changed content fails atomically. Sequential pending drawings reference the preceding pending ID. Reconciliation unions history; multiple heads remain visible and never collapse by timestamp. Erasure is a new empty snapshot.

First ink use promotes the state from archive v1 / ontology 0.1.0 to archive v2 / ontology 0.2.0. Typed-only archives retain their original version. Current Rust/native/WASM readers support both, including mixed-version private reconciliation; old decoders reject v2 or its unknown ink field. There is no downgrade that omits drawings. The encrypted transport envelope is unchanged because its strict embedded State carries compatibility, and the signed plaintext digest covers that complete state. `ontology/v0.1.0` remains immutable; discovery now also publishes `ontology/v0.2.0` with its own schemas, JSON-LD, manifest and `notebook-ink.json` semantic conformance fixtures. JSON Schema alone cannot enforce graph consistency or native-drawing interpretation.

### Ordered notes and whiteboards (ontology 0.3.0)

The optional `Block.note` holds `schema: "booknook.note.v1"`, a `text` or `whiteboard` face and immutable creation context. A note retains its stable block ID, prose, ink, attached character cards and notebook ordering when its face changes through `set_note_face`. Supply `capture_context` when input begins, and `note_creation: {at, capture_context}` when a deferred save needs to preserve that first input's original time and locations. Explicit `add_block` accepts `face` and `capture_context`. A blank automatic page has `pending` origin until meaningful input; legacy records use labeled `history_fallback` or `unknown` evidence instead of inventing a creation time. Ebook and audio position observations retain separate source locations and metadata. Reconciliation rejects incompatible recorded origins for review.

Whiteboard ink adds `canvas_space: {kind: "whiteboard", origin_x, origin_y}`. Stroke coordinates are absolute logical world positions; pan, zoom and canvas growth never translate old versions. A whiteboard dimension can grow to 2,000,000 logical units while its origin and far edge each stay within −1,000,000…1,000,000. Existing fixed-page versions keep the 1–16,384 bound. Native drawing bytes, portable strokes, parent histories and exact retry semantics are preserved across both coordinate spaces.

Using note metadata or whiteboard coordinates promotes the complete state to `booknook.archive.v3` / ontology 0.3.0. Readers accept legacy v1/v2 states and new v3 states; strict earlier readers reject v3. New commands are opt-in so legacy callers retain their old archive shape. Previously issued ontology and portable schema artifacts remain unchanged. `ontology/v0.3.0/note-whiteboard.json` publishes 12 valid and invalid vectors shared by Rust, native and WASM tests, including downgrade rejection.

## Complete portable exports

The existing native export envelope is now the public Rust `booknook_core::portable_archive::PortableArchive` contract. Its wire version remains `booknook.portable-export.v1`: `state` holds a v1, v2 or v3 domain document, `assets` maps asset IDs to original base64 bytes, and optional `social` retains the social state, transport binding/history proofs and pending relay events. The independent [portable protocol manifest v2](../ontology/portable/v2/manifest.json) includes v3 note and whiteboard fields. The [earlier portable edition](../ontology/portable/v1/manifest.json) and issued ontology 0.1.0 and 0.2.0 artifacts remain unchanged.

Rust `portable_archive::parse` and `roundtrip`, native/Swift `validate_portable_archive` / `validatePortableArchive`, and WASM `validate_portable_archive` consume the **original JSON**. They reject duplicate and unknown fields, unsupported versions, missing/corrupt referenced assets, invalid social structures and forged pending relay signatures under the existing 128 MiB envelope bound. Original asset encodings and embedded signed request, presentation and event strings are retained. Outer JSON whitespace, property order, and ordinary domain defaults may normalize; never sign the normalized envelope as if it were the original bytes. Legacy omitted `social` and omitted social `binding_history` remain accepted. Unreferenced v1 asset entries remain retained without being interpreted, preserving the previous envelope contract.

TypeScript exports `validatePortableArchive(core, rawJson): PortableArchive`, `encodePortableArchive(core, archive): string`, and `roundtripPortableArchive(core, rawJson): string`. The typed API rejects integers outside JavaScript's exact range; use the raw roundtrip to preserve Rust u64 values without passing them through `JSON.parse`. These helpers preserve the complete envelope rather than projecting only `.state`.

Validation is a portable integrity check, not authorization or a storage transaction. It does not install issuer pins, provider credentials or settings, trust an imported binding, approve publication, or bypass later replay/membership checks. A browser host must atomically preserve the complete validated envelope or implement its own multi-store restore. Native `Desk.restore_archive` still requires an empty library/social store, validates EPUB/PDF contents and exact asset metadata, and commits library and social records together. Its local revision is reset as before. Neither the shared contract nor native exports include connection settings, custody secrets or provider tokens. Existing `validate_archive` continues to validate a bare domain State for compatibility.

The protocol-only Nostr dependency verifies Schnorr signatures locally; no relay networking is added to the core or WASM APIs. `tests/portable-archive-parity.cjs` exercises real native export → WASM validation/roundtrip → native restore/re-export, including EPUB/PDF originals, rich text/CRDT and character history, branched ink and an exact-source transcription, archived approvals, binding history and pending relay events. Negative fixtures verify rejection before native state changes.

The source WASM build now compiles the existing locked `secp256k1-sys` dependency, so it
needs a WebAssembly-capable C compiler and archiver. Linux LLVM clang/llvm-ar can be selected
with `CC_wasm32_unknown_unknown` / `AR_wasm32_unknown_unknown`; Apple clang lacks the target.
`BOOKNOOK_ZIG=/absolute/path/to/zig bash scripts/build-wasm.sh nodejs` is the verified macOS
alternative (Zig 0.14.1, checksum checked against the [official index](https://ziglang.org/download/index.json)).
The script changes no global configuration and keeps compiler caches in `target/`. Prebuilt
SDK consumers do not need this compiler. The compiler is a build tool, not bundled SDK code.
