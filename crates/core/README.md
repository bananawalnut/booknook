# booknook-core

Portable documents, ontology validation, reading sessions, literary structures, rich notebook history and replaceable providers.

New SDK code is MIT OR Apache-2.0. Bundled dependency inventories retain upstream license boundaries. Synthetic test vectors contain no user credentials.

This crate is part of the Book Nook source workspace. Local sibling dependencies are versioned for future registry release; no registry publication is implied. The workspace packaging script creates and verifies extracted packages together without relying on unpublished registry versions.

Notebook handwriting uses `Command::SetNotebookInk` and `Command::SaveNotebookTranscription`. Each stable block may contain an independent fixed logical canvas, immutable full drawing snapshots and separate source-linked transcription records. Typed text remains a Yrs document; neither transcription nor erasure replaces the original drawing. `NotebookInk::heads()` exposes every concurrent branch. Reconciliation unions immutable records and rejects reused IDs with different content.

Legacy typed archives remain `booknook.archive.v1` with ontology `0.1.0`. Saving ink promotes the state to `booknook.archive.v2` and ontology `0.2.0`; new readers support both, old readers must reject v2. Private encrypted synchronization and portable exports carry that full state. There is no lossy downgrade. Native drawing bytes are optional canonical base64 alongside portable strokes; semantic validation checks bounds, references and parent graphs beyond JSON Schema. The 128 MiB state limit remains in effect.

## Notes and whiteboards

Optional note origin/face and absolute whiteboard canvas metadata promote only opt-in records to archive.v3 / ontology0.3.0. Legacy v1/v2 commands and documents retain their behavior. See `docs/NOTE-CONTRACT.md` in the source workspace for capture timing, recovery, conflict and coordinate rules. Published package fixture `tests/fixtures/note-whiteboard.json` exercises the shared strict contract.
