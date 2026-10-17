# Composing Book Nook with other data sources

The portable `provider_runtime` contracts and the native `ProviderHost` callback let an embedding application replace catalog, content, position, directory, recommendation and social transport sources. Providers return data; Rust validates and reconciles it. A downloaded descriptor is public metadata and does not install executable code, change trust settings or grant authority. No Book Nook server is required.

Existing `Desk.new`, import commands, stable Kindle/Goodreads IDs, archive versions and signing contracts remain supported. The new composition is additive. Storage substitution remains the separate `NotebookStorage`/compare-and-save contract; the native `Desk` continues to supply SQLite, local assets, recovery and social persistence.

## Registration and selection

The host supplies immutable original JSON through `descriptors_json`:

```json
{
  "schema": "booknook.provider-registry.v1",
  "providers": [{
    "id": "example.library",
    "source_namespace": "example.library",
    "capabilities": ["catalog", "content", "positions", "identity"],
    "available": true,
    "reason": null,
    "connection": null
  }]
}
```

A registry is limited to 1 MiB and 64 distinct adapter IDs. Native composition reserves two entries for `booknook.local-recommendations` and `booknook.nostr`; custom native hosts can therefore register at most 62 adapters. They cannot override those IDs. Select a custom provider explicitly to use it. A provider with an unavailable descriptor or connection cannot execute an operation.

`connection` is either `null` or the exact shared `castalia.provider-connection.v1` descriptor. The field is required. A null credential reference within a connection says that no portable credential reference is exposed; it does not assert that authentication is unnecessary. The connection's advertised source and capability must agree with the runtime operation. For example, a Kindle catalog cannot be attributed to an Audible account. Ephemeral sources may carry a connection without changing the saved companion registry.

`source_namespace` identifies source data independently of the runtime adapter ID. The host keeps local paths, file access grants, HTTP credentials and temporary WK sessions behind opaque `source_id` values. Rust does not interpret a provider's source identifier as a local file path.

## Read, review and commit

Create `Desk.with_providers(directory, host, generation)` with a positive JavaScript-safe generation. `replace_provider_host` validates the new registry and requires a strictly larger generation. The Swift `BookNookServices` facade supplies the corresponding asynchronous provider protocol and checked persistence calls. WASM/TypeScript uses the same core contracts with an injected host and atomic document storage.

`provider_fetch(provider_id, operation_json)` constructs a request containing an opaque request ID, generation, selected adapter, canonical descriptor digest, captured nullable connection ID and typed operation. The callback receives this original request JSON and returns `ProviderReply { response_json, content_bytes }`. Its response must echo request identity exactly. Duplicate keys, unknown fields, missing required nullable fields, mismatched capabilities, stale generations and changed descriptors are rejected before the result is exposed.

A successful native fetch returns a `booknook.provider-execution.v1` record in `ProviderReply.response_json`, with complete `request` and `response` objects. The [published conformance fixture](../ontology/providers/v1/conformance.json) supplies complete synthetic examples, including original raw responses with duplicate-key and missing-field failures. Fetching does not modify notebook state. Retain this exact execution JSON and its separate bytes through any human review, then call the appropriate synchronous commit. Native retains up to 128 recent reads, binding each to exact execution bytes and the saved connection settings. A changed, expired or fabricated read cannot commit. The final SQLite transaction checks both current settings and notebook revision, including changes made by another `Desk` handle.

Swift cancellation does not automatically propagate through UniFFI into a foreign asynchronous callback. The public facade checks cancellation immediately before synchronous commit. A cancelled read may finish fetching but cannot claim a saved import. Once a synchronous commit succeeds, its durable state must be accepted even if a UI task is cancelled afterward. Hosts must replace their registered adapters when source configuration changes; they must not mutate captured credentials or source selection behind a descriptor during a read.

For a custom storage host, `apply_catalog` and `apply_source_position` validate and stage changes in Rust, incrementing revision once. Reload the current state after the asynchronous read and use atomic `compareAndSave(expectedRevision, nextState)`. A CAS failure is a conflict, not a successful import or permission to overwrite the competing state.

## Source operations

| Operation | Result and commit |
| --- | --- |
| `catalog` with an opaque `source_id` | `catalog` result with `format: "normalized"`, `"kindle"` or `"goodreads"`; `provider_commit_catalog` returns `{state,result}`. |
| `content` with source ID and explicit EPUB/PDF media type | Matching source/media/SHA-256 plus separate bytes; `provider_commit_content` attaches an independently identified edition to a selected book and returns state. |
| `positions` with requested source IDs | Existing Audible `AudioObservation` values; `provider_attach_audio` retains the explicit reading-pass and optional edition review. |
| `source_positions` with requested source IDs | `{source_id,value,observed_at}` values; `provider_attach_position` stores the exact opaque location with unmatched alignment and an explicit reading pass. |
| `identity` with an absolute entity IRI | The matching descriptive `DirectoryReference`; neither directory kind nor `Actor` alias grants permission. |
| `recommendations` with `RecommendationInput` | Unique nonexcluded input candidates with bounded explanations; no notebook content is present in the input contract. |
| `social_publish` / `social_fetch` | Signed Nostr events and exact-event acknowledgments; transport validation is separate from application authorization. |

Normalized catalog entries contain `source_id`, `title`, `authors`, `subjects`, `edition_identifiers` and required nullable `language`. They supply metadata only: updates preserve personal rating, review, shelf, edited fields, writing, history and attached content editions. Stable identity uses the captured source namespace and source ID. Reimporting identical metadata does not duplicate books or provenance. Different sources do not silently collapse into one book.

The Kindle and Goodreads variants carry the original catalog JSON or CSV into the existing importers. They require the `kindle` and `goodreads` namespaces; normalized data cannot use those reserved namespaces. Legacy IDs and personal-edit protection remain intact. Kindle catalog entries containing progress require a separate runtime `positions` capability, and a bound connection must advertise `kindle.positions`. Catalog capability alone never implies content or progress access.

Content bytes are limited to 128 MiB and travel separately from JSON. The portable validator checks selected identity, allowed media type, size and independently computed SHA-256. Native persistence then applies its existing EPUB path/archive validation or PDF header validation and stores local assets. Portable hosts must perform their own safe content-format/asset persistence; a valid provider hash is not a complete EPUB/PDF safety or accessibility qualification. Native attachment retains source and connection provenance without inferring edition equivalence.

Audio and generic observations preserve source time and source identifiers. Generic locations are opaque and remain unmatched. No operation equates audiobook percentage with ebook percentage, and fetched observations do not automatically select a reading pass or confirm an ebook edition.

## Recommendations, transport and authority

`provider_social_recommendations` constructs the minimal recommendation input from catalog candidates, chosen interests and independently audited public social evidence. A failed social audit excludes that evidence. Private notebooks, annotations, character histories and private-group posts are not passed to a recommendation provider.

`provider_social_exchange` runs the existing native approval, binding, encryption, recipient, membership and replay checks around the selected transport. It receives the existing transient Nostr key bytes in native code; the callback sees only signed public events or encrypted private events. An outage keeps pending deliveries. Only actual exact-event relay acknowledgments advance them. Incoming data still passes Nostr signature, Castalia identity binding, publication approval and group-history checks. A valid Nostr signature alone does not authorize altered application content.

Authority is deliberately separate: the reusable `CastaliaCustody` object and `CastaliaNative` facade operate from independently selected trust policy, exact expected request/disclosure and explicit consent. Provider descriptors and directory results cannot supply trusted issuer pins, choose the holder or approve themselves.

## Contracts and verification

Schemas are published in `ontology/providers/v1/provider-{registry,operation,request,response,execution}.schema.json`. The independent provider-contract manifest and conformance fixture live beside them and are linked by discovery; frozen domain ontology 0.1.0/0.2.0 files remain unchanged. Generated TypeScript types are in `sdk/typescript/provider-types.ts`. Runtime validators additionally enforce UTF-8 bounds, unique IDs, capabilities, current source correlation, content hashes and domain rules that JSON Schema alone cannot express.

Focused coverage lives in `crates/core/tests/provider_runtime.rs`, `crates/native/tests/provider_composition.rs` and the injected-transport case in `crates/native/src/social_relay_acceptance.rs`. These exercise alternate metadata and opaque-position sources, existing import identity, private-edit preservation, SQLite reopen, cancellation-safe split calls, late provider replacement, cross-window settings conflicts, bounded content validation, minimized recommendation inputs, outage retry, duplicate reception and both transport/application forgery rejection. Rust/native/WASM and installed consumer checks cover the public facades separately. Synthetic acceptance does not qualify live provider accounts, production issuer provisioning, hardware input or distribution signing.
