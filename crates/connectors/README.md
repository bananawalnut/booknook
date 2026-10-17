# booknook-connectors

Optional library companion, listening-position and Nostr transports.

`amazon::CompanionClient` talks to the explicitly configured paired companion. For an account outside the US, call `audible_positions_in_marketplace(token, marketplace, asins, now)` with `us`, `uk`, `de`, `ca` or `au`. The existing `audible_positions(token, asins, now)` keeps its original request shape and the companion's legacy US default. Both preserve source observation time, freshness and unmatched ebook alignment; neither controls playback or qualifies a real account.

The built-in account-aware adapter uses the explicit marketplace to select its corresponding credential. Older external adapters without account-completion support retain their original account-selection contract; sending a marketplace cannot add support that their implementation does not provide.

`provider_connections(token, marketplace)` retrieves the paired device's shared Castalia descriptors from `/v1/provider-connections/{marketplace}` on companions advertising `provider_connections_v1`. It uses bearer authentication, bounds the response at 1 MiB, and validates closed schemas, unique IDs, capabilities, labels and nonsecret credential references with the shared core rules. The method itself performs the route request; hosts can inspect `capabilities()` first for compatibility with older companions. An unsupported or denied route returns an error rather than fabricating a connection. Listing metadata never registers an account, retrieves provider secrets, installs authority or establishes live qualification.

These additive methods use the same no-redirect transport and bounded errors as the existing client. `tests/companion_client.rs` exercises the public methods against real loopback HTTP, including each supported marketplace, the original US request, invalid input without network I/O, malformed/oversized registries, denied authentication and refused redirects. Its records are synthetic; Amazon and production identity providers are not contacted.

New SDK code is MIT OR Apache-2.0. Bundled dependency inventories retain upstream license boundaries. Synthetic test vectors contain no user credentials.

This crate is part of the Book Nook source workspace. Local sibling dependencies are versioned for future registry release; no registry publication is implied. The workspace packaging script creates and verifies extracted packages together without relying on unpublished registry versions.
