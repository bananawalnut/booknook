# Native feed cache controller

`booknook_native::FeedCacheController` provides standalone host-local SQLite storage for public feed sources. The Rust SDK owns acquisition attempts, durable configuration and admission. It is separate from the notebook database, portable exports, credentials and social state. UniFFI/Swift integration and the app interface are subsequent delivery steps.

## Opening and ownership

Call synchronous `open(directory, expected_configuration, provider)` or `public_https(directory, expected_configuration)` off the UI thread. Construction performs local I/O and stays inactive. A new store needs explicitly supplied strict configuration; an existing store loads its authoritative bytes when `expected_configuration` is absent, or requires an exact byte match. Constructor arguments never replace stored settings. Unknown schema or authoritative corruption fails without resetting the store.

Share clones of one controller across windows. A dedicated bounded storage thread owns SQLite and the nonblocking filesystem owner lock. A second independent live writer is refused. An unpredictable durable owner incarnation and configuration/provider/activation/request fences protect against stale work and restart reuse. Reopening rotates ownership and clears activation. Dropping the final controller does not join a storage thread on the caller; the worker drains or discards work and retains its lock until it exits. Explicit async `close()` acknowledges resource release.

The directory must be host-controlled; it receives mode 0700 and regular cache/lock files receive mode 0600. The cache contains public feed bytes and local subscription choices. It is local SQLite storage, not an encrypted synchronization format or a hostile-directory security boundary.

## Settings, activation and refresh

`configuration()` returns exact authoritative bytes and opaque configuration/provider tokens. `replace_configuration(expected_token, bytes)` and `replace_provider(expected_token, provider)` use atomic compare-and-swap and revoke activation. Portable `enabled` is a preference; only explicit `activate(source_id)` permits acquisition. `disable(source_id)` pauses acquisition while retaining valid cached text. Import, navigation, offline reads and restart never activate or fetch.

Reviewed configuration changes atomically evict rows and receipts only for removed sources or a changed URL/expected format under the same ID. Cosmetic or enabled-preference changes retain compatible rows. A different provider ID evicts incompatible rows. Failed transactions retain the old settings and cache. This deliberate invalidation allows a newly activated edited source to refresh unconditionally; corrupted rows do not trigger an automatic network fallback. An additive empty import is still a no-op at the review layer, not a removal instruction.

`refresh(source_id)` privately registers its request before dispatch. Newer requests supersede only the same source. Network I/O runs outside storage work; normalization, SQLite and cancellation checks use the bounded dedicated worker. Four stalled network blocking workers cannot strand offline storage. Queued capacity remains charged until the actual work is dequeued, even if its caller disappears.

## Durable data and offline reads

One current row per source retains exact original XML, SHA-256, the **full** normalization context, normalizer profile, validators and an accepted-request receipt. Full context includes source/provider, requested and final URLs and expected format: identical XML can resolve relative links differently. `offline(source_id)` verifies stored bindings and hashes and reruns the pure normalizer. It never trusts deserialized produced-only normalized JSON and never fetches as a fallback.

A conditional 304 reads and hashes the actual retained raw bytes inside the final transaction. Missing, evicted, changed or mismatched context/body/profile/revision refuses admission. The opaque core guard encloses the synchronous transaction commit and eviction; no foreign callback, network request or await occurs there. A staged snapshot alone is insufficient.

The logical quota is 64 MiB of retained raw plus conservatively counted row metadata, with at most 128 current rows. Atomic eviction uses least-recently-admitted/refreshed order, excluding the new candidate. Offline reads do not require a recency write, so they remain useful on a full disk. This is **not** a physical SQLite/WAL file-size cap; checkpointing and page allocation may use additional disk space. An actual SQLite-full failure preserves the previous committed row.

## Cancellation, recovery and display data

Dropping a Rust operation marks its queued storage work canceled and cancels its active core acquisition. Final guards recheck cancellation and durable fences. Once guarded commit begins, cancellation cannot promise rollback. A failed acknowledgement is reconciled against the exact committed request receipt and complete row in one fresh transaction; it is not blindly replayed. Unreconcilable outcomes close the controller and report uncertainty. Lifecycle commit errors also fail closed.

Local observation times stay separate from publisher dates. Local clock rollback refuses a new admission while retaining the old cache; it does not clamp or invent a timestamp. Missing, invalid, pre-epoch and leap-second publisher dates remain recoverable with raw text and signed seconds/nanoseconds. Variant keys use source plus normalized content/profile, excluding source order, duplicate count and conflict markers; conflicting native IDs remain separate entries.

Display records preserve provenance, raw/context/profile hashes and configuration/cache tokens so hosts can reject stale completions. Render inert text literally. Neither source metadata nor a transport key establishes Castalia authorship. `read_status()` describes local activation, retained data and failure; it does not assert publisher freshness.

[Focused qualification](feed-cache-evidence.json) passes 17 native unit cases and three integration scenarios plus an ordinary child-helper entry, strict all-target Clippy and formatting. It uses actual SQLite/OS process primitives and explicitly labels injected acknowledgements and DNS-equivalent executor saturation. The local clock rollback branch is source-reviewed policy, not an injected-clock test. This API does not establish an installed app, foreign-future cancellation, physical-device acceptance or cross-client synchronization.
