# Castalia identity and provider connections

Book Nook is a Castalia application. It does not require a separate Book Nook account or membership credential. A reader can keep using the offline desk before connecting an identity. Connecting an identity does not authorize publication, synchronization, a Nostr binding, or a group operation: those continue to require explicit approval of the exact operation through the existing Castalia v2 contracts.

## Ownership and portability

Castalia owns the reusable public contracts `castalia.native-authority-profile.v1` and `castalia.provider-connection.v1`. Their TypeScript definitions, parsers and fixtures are developed in `castalia-wallet-auth`; Book Nook's independently implemented Rust consumer checks the same fixtures. The existing browser `castalia.provider-profile.v1`, browser-origin connection consent, membership credentials and signing domains remain unchanged.

Book Nook owns books, editions, notebooks, reading history and references to source connections. A provider descriptor contains a stable opaque connection ID, provider ID, label, observed capabilities/status and an optional **nonsecret credential reference**. The reference identifies material managed by the selected adapter; possession of it gives no access. No access token, recovery material, account password, transport key or permission grant appears in this contract.

The first implementation uses secure local custody and provider adapters. The connection contract also describes wallet-managed storage so a future adapter can use the same boundary; that value does not mean a wallet transport is installed. Provider credential portability is explicit: device-bound, requiring reauthorization, or governed by the provider. Copying a descriptor never promises that its credential resolves on a second device.

## Native authority setup

In Connections, import a public authority profile, inspect its issuer key, audience, caller, policy digest and exact endpoint, and explicitly accept it with the selected Castalia holder key. A profile deliberately contains **no holder key**, membership claim or secret. Parsing establishes only that its shape is supported. The user must obtain and check it through their trusted issuer's distribution channel.

Preview is read-only. Installation compares both the reviewed profile digest and the current settings digest in a SQLite transaction. A second window or process changing settings invalidates the review. Successful installation replaces only Castalia configuration and the profile label, preserving relay and provider settings. Existing manual configuration remains available. The native form also uses a conditional save so a stale second window cannot undo a newer profile installation. Recovery import, issuer access tokens, secure-store replacement and per-operation consent remain separate flows.

Profiles can be exported from the current public configuration. Private reading backups still exclude connection settings and Keychain material; they preserve source references in imported-book provenance and listening observations. An exported profile must be reviewed on the receiving device; it is not an automatic trust migration.

The reusable Castalia interface does not imply that the deployed Zenith or Castalia Web issuer currently accepts Book Nook's operation requests. Their membership issuance is distinct from generic v2 action authority. Reuse an existing issuer when it supports that contract and its operator explicitly permits the caller/actions. The optional reference issuer supplies a concrete compatible local implementation without inventing a membership requirement.

## Provider adapter behavior

The paired library companion advertises `provider_connections_v1`. Authenticated `GET /v1/provider-connections/{marketplace}` returns a JSON array of shared descriptors; the supported marketplace identifiers are `us`, `uk`, `de`, `ca` and `au`. The route without a marketplace defaults to `us`. This listing is private to the paired device. It does not connect, renew, register or export credentials.

The selected local Kindle catalog has provider ID `com.amazon.kindle`, `kindle.catalog` capability and a null credential. The optional Audible adapter reports `com.audible`, `audible.positions`, secure-local storage and required reauthorization for transfer. Audible references are bound to the paired device, selected marketplace and adapter store. `available` means stored connection material is available to that adapter; it does not certify live source accuracy or content access. Qualification remains separate.

When the optional source process is unavailable, the companion preserves local catalog access and marks cached source references unavailable. Native import accepts a connection reference only if the current registry has the matching available provider/capability. Endpoint and marketplace changes invalidate pending discovery. Old companions without this capability continue through the earlier import path, with no invented provenance reference. For an old source adapter behind a new companion, the companion identifies the selected adapter connection and its advertised data capability conservatively: a null credential means no credential reference is exposed, not a claim that the provider requires no credentials. It does not guess secret storage or portability. Data reads must still succeed independently.

## SDK contracts

Rust provides strict profile and connection parsers. Native and WASM export equivalent raw-JSON validators; the typed JavaScript facade exposes `validateNativeAuthorityProfile` and `validateProviderConnections`. Pass original JSON to validation so unknown fields and duplicate keys cannot disappear during an intermediate decoder step. Structural validation is never a signing operation.

New optional settings and provenance fields preserve old stored data and old portable archives. The authority and descriptor schemas use Castalia schema identifiers; Book Nook's ontology manifest discovers and checksums the bundled offline copies. Cross-language fixtures cover valid values, unknown fields, Unicode controls, endpoint canonicalization, identifier limits and credential/reference distinctions.

Live issuer approval, Kindle/Audible account qualification, physical iPad input and distribution signing remain separate acceptance gates. The automated evidence uses synthetic identities/accounts and does not satisfy those gates.
