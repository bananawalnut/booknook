# booknook-core

Portable documents, ontology validation, reading sessions, literary structures, rich notebook history and replaceable providers.

New SDK code is MIT OR Apache-2.0. Bundled dependency inventories retain upstream license boundaries. Synthetic test vectors contain no user credentials.

This crate is part of the Book Nook source workspace. Local sibling dependencies are versioned for future registry release; no registry publication is implied. The workspace packaging script creates and verifies extracted packages together without relying on unpublished registry versions.

For the standalone Rust SDK, extract the four same-version Book Nook archives into one directory. Copy `booknook-core-0.4.0/sdk-workspace.toml` and `booknook-core-0.4.0/Cargo.lock` to that directory as `Cargo.toml` and `Cargo.lock`, then run `cargo test --locked --workspace --all-targets`. The workspace selects the bundled faster-hex source with its heapless dependency update. Cargo applies patches from the top-level consumer workspace. Applications with an existing workspace must copy the matching `[patch.crates-io]` entries into their own root manifest, adjusting paths to their extracted SDK.

Goodreads reimports match the source Book Id and update unedited titles and authors. Supplied ISBN/ISBN13 columns record their corresponding catalog identifier; an explicit blank records a clear and a missing column preserves it. Private edits, unrelated identifiers, language and attached content editions remain intact. An already recorded identical source row is unchanged, even when imported after a correction. Portable ISBN history in archive v5 preserves corrections and clears across stale replicas, exposes competing choices for review, and allows deliberate restoration of earlier values. Readers retain v1–v4 support; a peer receiving v5 must support it. See the [ISBN history guide](https://github.com/bananawalnut/booknook/blob/main/docs/ISBN-HISTORY.md).

Notebook handwriting uses `Command::SetNotebookInk` and `Command::SaveNotebookTranscription`. Each stable block may contain an independent fixed logical canvas, immutable full drawing snapshots and separate source-linked transcription records. Typed text remains a Yrs document; neither transcription nor erasure replaces the original drawing. `NotebookInk::heads()` exposes every concurrent branch. Reconciliation unions immutable records and rejects reused IDs with different content.

Legacy typed archives remain `booknook.archive.v1` with ontology `0.1.0`. Saving ink promotes the state to `booknook.archive.v2` and ontology `0.2.0`; new readers support both, old readers must reject v2. Private encrypted synchronization and portable exports carry that full state. There is no lossy downgrade. Native drawing bytes are optional canonical base64 alongside portable strokes; semantic validation checks bounds, references and parent graphs beyond JSON Schema. The 128 MiB state limit remains in effect.

## Notes and whiteboards

Optional note origin/face and absolute whiteboard canvas metadata promote only opt-in records to archive.v3 / ontology0.3.0. Legacy v1/v2 commands and documents retain their behavior. See `docs/NOTE-CONTRACT.md` in the source workspace for capture timing, recovery, conflict and coordinate rules. Published package fixture `tests/fixtures/note-whiteboard.json` exercises the shared strict contract.
