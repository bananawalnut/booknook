# booknook-native

Local SQLite and assets, durable edit recovery, credential custody, synchronization and a UniFFI bridge.

New SDK code is MIT OR Apache-2.0. Bundled dependency inventories retain upstream license boundaries. Synthetic test vectors contain no user credentials.

This crate is part of the Book Nook source workspace. Local sibling dependencies are versioned for future registry release; no registry publication is implied. The workspace packaging script creates and verifies extracted packages together without relying on unpublished registry versions.

The optional `booknook-issuer` binary uses an explicitly configured policy and operator-provided signing key. Its operator contract and setup are in [REFERENCE-ISSUER.md](REFERENCE-ISSUER.md). It neither provisions keys automatically nor grants permissions based on wallet possession.
