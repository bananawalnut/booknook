# Goodreads ISBN history

Book Nook tracks `isbn` and `isbn13` separately for each Goodreads catalog edition. A correction or clear can then survive an exchange with an older copy of the library. Other identifiers and attached reading editions keep their existing behavior.

## How an import changes an ISBN

A supplied Goodreads column records its normalized value. A supplied blank records an empty value set. An omitted column leaves that namespace alone. Reimporting the same source row through the same connection adds no ISBN observation, even if the import time changes.

A **head** is a recorded value or decision with no successor in this field’s history. Several heads can agree on the same value set. When they agree, a new import can advance all of them. When they disagree, an import advances only matching import heads for its Goodreads source ID and connection. Independent source changes, reader decisions and competing legacy values stay available for review. Import times and file arrival order do not settle a dispute.

Suppose two devices start with the same ISBN. One clears it and the other corrects it. After exchange, the reader can choose either the empty set or the correction. Until then, `provisional_values` gives the deterministic value shown in the edition. Receiving the shared starting observation again preserves that history. An explicit historical reassertion can restore the starting ISBN as a new decision.

## Reading and recording a decision

| SDK | Pending decisions | History for one edition and namespace |
| --- | --- | --- |
| Rust | `isbn_history::pending_reviews(&state)` | `isbn_history::history(&state, edition_id, namespace)` |
| Swift | `BookNookNotebook.isbnReviewsJSON()` | `BookNookNotebook.isbnHistoryJSON(editionID:namespace:)` |
| TypeScript | `isbnReviews(core, state)` | `isbnHistory(core, state, editionId, namespace)` |

Each pending decision supplies `edition_id`, `namespace`, `heads`, distinct `choices`, and `provisional_values`. Read the portable decisions after opening, restoring or receiving a desk, even when the ordinary sync conflict count is zero. The native Sync & Recovery view presents the same choices and labels an empty set as a clear.

- To choose a current alternative, send `review_isbn` with the returned `edition_id`, `namespace`, the complete `heads` array as `expected_heads`, and the selected `values` array.
- To restore an earlier value, read that field’s history and send `reassert_isbn` with its `edition_id`, `namespace`, complete `expected_heads` and the superseded node ID as `historical_node`. This also works after a dispute was settled. A legacy value anchor can be the only surviving record of an earlier catalog value.
- An accepted decision returns `node_id` and `replayed`. An exact retry acknowledges the existing decision and preserves the revision. If another change makes a new decision’s heads stale, reload the field and let the reader choose against the new heads. Independently made decisions can themselves require review after exchange.

A legacy catalog can carry one or more earlier ISBNs. Book Nook retains that observed set when it first records history. If an older catalog is empty and its source rows cannot establish an explicit clear, `legacy_absence_unknown` records the uncertainty. This diagnostic appears separately from the selectable value sets.

## Archives, privacy and authority

Managed history lives in `State.isbn_history` under the edition ID and namespace. Recording the first managed history promotes the state to `booknook.archive.v5` with ontology `0.5.0`. Reading and round-tripping v1–v4 documents preserves their version without adding ISBN history. A frozen v4-only client rejects a v5 state as unsupported. A receiving client needs v5 support to preserve these decisions across devices. [Ontology manifest and conformance cases](../ontology/v0.5.0/manifest.json)

Rust hosts constructing `State` with a struct literal must supply `isbn_history: Default::default()` or retain `..State::default()`. The SDK 0.4.0 version marks this source-contract addition.

ISBN nodes use content hashes and parent references. They are unsigned local catalog history. Local imports, autosave and ISBN decisions use local storage without a signing prompt. Castalia authorization applies separately when a reader approves an encrypted transfer to a paired device or a social publication.

A private portable export retains the full ISBN history, source rows and earlier choices alongside the current reading desk. Exporting the file leaves publication and delivery under the reader’s control. The enclosing format remains `booknook.portable-export.v1`. [Portable protocol manifest](../ontology/portable/v4/manifest.json)
