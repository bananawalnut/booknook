# Book Nook JavaScript SDK

The package includes a typed facade, ESM/CommonJS JavaScript, and separate browser/Node WebAssembly runtimes. It requires no Book Nook server. Install a locally generated `booknook-sdk-0.1.0.tgz`; no npm registry publication is assumed.

```js
import {BookNook} from '@booknook/sdk';
import * as core from '@booknook/sdk/wasm'; // Node initializes its packaged WASM itself.
const desk = await BookNook.open(core, yourAtomicStorage);
await desk.execute({op:'add_book', title:'A book', author:'A writer', at:1});
```

Browser hosts initialize the included web runtime before opening a desk:

```js
import init, * as core from '@booknook/sdk/wasm/web';
await init(); // Resolves the adjacent packaged .wasm URL; serve application/wasm.
```

`NotebookStorage.load()` returns a copied State or null. `compareAndSave(expectedRevision, state)` must atomically compare the expected revision, including null for an empty store. A failed comparison requires reload/review, never an unconditional retry that could overwrite another edit. Hosts own transport and custody; private notebook contents do not enter recommendation processing.

For replaceable data sources, construct `BookNookServices.open(core, storage, providers)`. Each `BookNookProvider` supplies immutable `descriptorJSON` and `execute(requestJSON, signal?)`. The original registry/response JSON passes through Rust validation before decoding. Use `importCatalog` for normalized or legacy Kindle/Goodreads reconciliation, `acquireContent` for hash/source/media verification, `sourcePositions` and `attachPosition` for explicitly reviewed opaque locations, and `recommendations` for the minimal public input. The host must parse and store actual EPUB/PDF assets; a validated content envelope alone is not readable content. `fetch` also supports directory identity and social transport, but returned transport events still need signature, binding, permission, membership and replay verification before application.

Register executable providers explicitly; a downloaded descriptor cannot install one. Descriptor capabilities, availability, namespace and optional shared connection are checked independently. Source changes invalidate pending reads. Cancellation before commit leaves state unchanged; a successful CAS already in progress is reported as committed even if cancellation arrives late. Registration replacement is refused while a commit is in progress. Each provider receives only the selected operation, never complete private state. The default request ID uses `crypto.randomUUID`; supply `requestID` in service options on hosts without it.

`validateCastaliaRequest` and `verifyCastaliaReceipt` accept original request/presentation JSON, the host's exact expected canonical request, expected disclosure and independent `CastaliaTrustPolicy`. They verify generic Castalia v2 contracts for another application without imposing a Book Nook account. The receiver must still validate its operation and atomically consume the verification result's replay key. Native custody signs after explicit consent; this WASM API exposes verification and no signing secret.

The facade exports `validateNativeAuthorityProfile(core, rawJSON)` and `validateProviderConnections(core, rawJSON)` with shared Castalia types. Supply the original JSON to preserve duplicate/unknown-field rejection. These validate public configuration and nonsecret references; they do not establish issuer trust, connect a wallet or grant permissions. Book Nook needs no separate membership scope. Source-bound import and position records can preserve optional `provider_connection_id` values in portable archives.

`recommendations(core, input)` uses catalog metadata, selected interests and independently verified shared ratings; private writing is excluded. `socialRecommendations(core, state, candidates, excluded, policy, now)` verifies the persisted social history and its projections using current independent issuer pins before using friends' public ratings. It rejects forged or revoked evidence. The policy actor is the viewer; `now` is a `bigint`. Use local recommendations without shared ratings when no Castalia identity is configured.

`prepareSyncBatch(core, actor, revision, recipients, plaintext)` prepares an exact byte digest. After explicit host consent, `sealSyncBatch(core, batch, plaintext, pairingKey, requestJSON, presentationJSON, policy, now)` verifies the Castalia approval and returns `{packet, replay_key}` in the native `booknook.sync-packet.v1` format. `openSyncBatch(core, packet, pairingKey, recipient, policy, now)` verifies historical approval under current pins before returning `{plaintext: Uint8Array, packet_digest, replay_key}`. Plaintext and the 32-byte symmetric pairing key are `Uint8Array`; the key is not serialized. No signing key or signing operation is exposed. The maximum plaintext size is 64 MiB.

Hosts own the durable lifecycle: atomically consume the approval replay key with storing the outgoing packet, then transmit that exact saved packet. Receiving hosts separately verify their paired source, payload schema/source/revision and atomically apply content with batch ID/digest bookkeeping. Repeat IDs with equal digests are idempotent; changed content under an existing ID must fail. Packet decoding itself is pure and does not consume a replay or write notebook state. Native transfer plaintext requires `booknook.sync-content.v1`, `source_device`, complete `state`, and matching `assets`; opaque bytes alone are not a native desk export. Current issuer revocation is checked even for old packets. Keep original raw JSON when using the corresponding WASM exports to preserve duplicate-field rejection.

From the source workspace run `bash scripts/package-typescript.sh`. It builds both WASM targets without overwriting either, compiles both JavaScript module formats, packs locally, and verifies the actual installed archive. New SDK code is MIT OR Apache-2.0; dependency licenses remain distinct.

Notebook ink is additive to each stable `Block`; typed `text`, Yrs `update`, styles and character-card attachments remain independent. Save a full logical-canvas snapshot with `set_notebook_ink`, including `parents` from the version the editor actually displayed and an optional stable `version_id` for retry. Erasing saves an empty new snapshot. Save optional text using `save_notebook_transcription`, with an exact `ink_version_id`, `method: "manual" | "on_device"` and an optional stable `transcription_id`. Transcription never changes ink or prose. Reusing an ID with the identical payload and reader is a no-op (`replayed: true`); a different payload fails. Keep the original timestamp, parent IDs and payload across retries. Queue sequential local saves with the previous pending version ID as parent, and retain unsaved data after a storage error.

`set_notebook_ink` returns `{ink_version_id, heads, conflict, replayed}`. After reconciliation, derive heads from `Block.ink.versions` as versions not referenced by another version's `parents`; expose every head and require an explicit choice before continuing a branch. Do not select the newest timestamp or replace an entire ink history to resolve a conflict. Multiple transcripts for one drawing are independently retained.

Legacy typed states remain archive v1 / ontology 0.1.0, and fixed-page ink promotes them to archive v2 / ontology 0.2.0. The optional `Block.note` metadata and whiteboard `canvas_space` promote state to archive v3 / ontology 0.3.0. Current native and WASM readers accept all three; earlier strict readers reject new data they cannot preserve. Hosts must retain both optional fields and keep notebook block order during persistence, synchronization and export. Earlier ontology releases remain immutable.

`Block.note` records a `text` or `whiteboard` face and immutable creation context, while prose and ink remain independently retained on the same stable block. `set_note_face` changes presentation without moving the note. Supply `capture_context` at input time, and `note_creation: {at, capture_context}` when a delayed save must retain the first input's original context. Explicit `add_block` can take `face` and `capture_context`. Empty automatic pages remain `pending`; legacy history can supply a labeled `history_fallback`, and unavailable dates stay unknown. Ebook and audio observations retain their distinct locations and source metadata. Conflicting recorded origins require review.

Fixed-page canvas dimensions remain 1–16,384 logical units. Whiteboards supply `canvas_space: {kind: "whiteboard", origin_x, origin_y}` with absolute logical coordinates. Each dimension can grow to 2,000,000 units within world edges −1,000,000…1,000,000. Pan and zoom change the viewport without transforming stored strokes. Rotation/reflow changes presentation scale; portable colors use `#rrggbb` (six hex digits). Native drawing data is canonical base64, at most 8 MiB decoded per snapshot; portable strokes allow off-canvas points for clipped edge strokes, at most 10,000 strokes / 1,000,000 points per snapshot. Portable strokes retain centerlines, pressure, time, color and width; PencilKit brush texture and opacity remain in the original native drawing and may be approximated by other renderers. Transcription text is bounded to 1 MiB; the overall 128 MiB archive limit still applies.

For a **complete** native export, use `validatePortableArchive(core, originalJSON)` to obtain
its typed `PortableArchive`, or `roundtripPortableArchive(core, originalJSON)` for a raw
lossless roundtrip that also preserves u64 values outside JavaScript's exact numeric range.
`encodePortableArchive(core, archive)` validates a typed host-created envelope. These
preserve assets, ink/history, social state, binding proofs and pending relay events; do not
project only `.state` when storing an export. The existing `validate_archive` WASM API still
validates a bare State. Complete-envelope validation checks checksums and pending Nostr
signatures, but does not install authority, authorize publication, parse EPUB/PDF internals,
or atomically restore host stores. Native Desk restore performs its additional content and
transaction checks. JSON formatting can normalize; embedded signed strings and base64
assets remain unchanged. Settings, private keys and provider tokens are not exported.

Building the source WASM runtime requires a C compiler with a WebAssembly target for
`secp256k1-sys`. LLVM clang/llvm-ar can be selected with `CC_wasm32_unknown_unknown` and
`AR_wasm32_unknown_unknown`. On macOS, Apple clang lacks this target; alternatively set
`BOOKNOOK_ZIG=/absolute/path/to/zig` before the workspace build/package scripts. The build
script uses this explicit compiler only for WASM and keeps Zig caches under `target/`.
Prebuilt npm consumers do not need a C compiler. Zig 0.14.1 is a verified option; obtain
compiler archives and checksums from the [official Zig download index](https://ziglang.org/download/index.json).
