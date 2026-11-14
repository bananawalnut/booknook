# booknook-native

Local SQLite and assets, durable edit recovery, credential custody, synchronization and a UniFFI bridge.

New SDK code is MIT OR Apache-2.0. Bundled dependency inventories retain upstream license boundaries. Synthetic test vectors contain no user credentials.

This crate is part of the Book Nook source workspace. Local sibling dependencies are versioned for future registry release; no registry publication is implied. The workspace packaging script creates and verifies extracted packages together without relying on unpublished registry versions.

The optional `booknook-issuer` binary uses an explicitly configured policy and operator-provided signing key. Its operator contract and setup are in [REFERENCE-ISSUER.md](REFERENCE-ISSUER.md). It neither provisions keys automatically nor grants permissions based on wallet possession.

The standalone Rust `FeedCacheController` owns a separate bounded SQLite cache for public feed sources. It opens inactive, preserves authoritative configuration through compare-and-swap and re-normalizes retained raw XML offline. Synchronous open belongs off the UI thread; other operations use an isolated bounded storage worker. The additive UniFFI facade and Swift `BookNookFeedCacheV1` provide async host operations and operation-scoped cancellation; focused native/generated-Swift checks pass. The native app interface remains subsequent work. See the source workspace documentation at `docs/FEED-CACHE.md`.
