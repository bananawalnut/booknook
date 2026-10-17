# Book Nook Library Companion

An optional source adapter host, using only Python’s standard library. This is not a Book Nook backend and is not required for offline reading or writing.

Run locally:

```sh
python3 services/library-companion/booknook_companion.py --data ./work/companion
```

In Book Nook → Connections → Library companion, enter the displayed URL and one-use pairing code. The code expires in five minutes. Book Nook stores its device token in Keychain; the companion stores only a SHA-256 token digest. Restarting displays a new pairing code while preserving existing pairings. Disconnect revokes only the current device token.

Use the Kindle catalog file chooser in Connections to select metadata XML, normalized `booknook.kindle-catalog.v1` JSON, or a supported standalone Kindle-for-Mac SQLite snapshot. Only normalized JSON is saved in the companion’s private data directory. Refresh imports the selected source privately. XML recognizes `meta_data` records with ASIN/title/authors, including BOM-marked UTF-8/16/32 text; it rejects DTDs and entities and does not infer timestamps or progress.

SQLite import requires Python 3.11+ with SQLite deserialize support. It reads only explicit ASIN/title/authors from the documented `ZBOOK` metadata layout, entirely in memory. Source data may include samples or dictionaries; a cache does not establish purchases, complete library coverage or access to book content. All positions remain unknown. Unknown schemas and malformed records fail the import without replacing the previous catalog. The [source evidence and preparation instructions](SOURCE-EVIDENCE.md) describe the inspected MIT source, accepted archive layout, bounds and remaining live-source qualification.

Raw SQLite uploads require a literal loopback destination in Book Nook and a loopback-bound server plus an actual loopback client. For a remote companion, normalize the explicitly selected snapshot locally first:

```sh
python3 services/library-companion/booknook_companion.py \
  --export-kindle-catalog '/selected/KindleSnapshot.sqlite' \
  --output '/chosen/new/KindleCatalog.json'
```

This command starts no server, writes a new mode-0600 JSON file and never overwrites existing output. Select that JSON in Book Nook. A WAL-mode source is rejected: quitting Kindle alone may leave a WAL-mode file. Create a consistent SQLite backup and set `journal_mode=DELETE` on the **backup only**, as shown in the source instructions. No automatic filesystem discovery, account login, cookie access, device registration or DRM/content extraction occurs.

Alternatively pass `--kindle-catalog PATH`. A selected SQLite snapshot is checked and normalized before the server starts; restart with a new snapshot to refresh it. Selected JSON/XML files retain read-on-refresh behavior. A successful file upload becomes the active source for subsequent refreshes, even when a startup source or live provider exists. Saved uploads remain active after restart, including with the same `--kindle-catalog` argument. To switch back to the configured file or adapter source, stop the service and remove only the normalized `kindle-catalog.json` selection from its data directory before restarting; this never deletes books already imported into Book Nook.

For iPad access, use a hostname with a trusted HTTPS certificate:

```sh
python3 services/library-companion/booknook_companion.py --data ./work/companion --host 0.0.0.0 --cert certificate.pem --key private-key.pem
```

Book Nook requires HTTPS away from exact loopback. The reference server rejects browser-origin requests, redirects are not followed by the client, and request/response sizes and source execution times are bounded. Its HTTP server is for personal deployment behind appropriate network access controls, not a public multi-tenant service.

## Reusable source adapter protocol

The companion also exposes shared Castalia public connection descriptors through authenticated `GET /v1/provider-connections/{marketplace}` (`us`, `uk`, `de`, `ca`, `au`; the unsuffixed route defaults to `us`). Its manifest advertises `provider_connections_v1`. The response is an array of `castalia.provider-connection.v1` records with opaque references and no credentials. A selected local Kindle catalog has no credential. Optional source adapters advertise `provider.connections.list` and receive server-derived `device_id` plus the selected `marketplace`; they return the same descriptor array. Unknown fields, duplicate IDs and unsafe labels fail validation. On adapter failure, cached account references become unavailable while local catalog access remains usable. See [Castalia connection ownership and portability](../../docs/CASTALIA-CONNECTIONS.md).

A separately installed executable is configured with `--provider-command /absolute/path/to/adapter [arguments...]`. No shell is used. Every request starts the executable and sends one JSON object on stdin:

```json
{"schema":"booknook.source-request.v1","operation":"capabilities","parameters":{}}
```

The process returns one JSON object on stdout and exits zero. Nonzero exit, timeout, malformed/duplicate-key JSON, or oversized output becomes an explicit unavailable result. Output/error streams are not logged. Credentials belong to the source adapter’s secure storage and never appear in the portable library.

Operations:

| Operation | Parameters | Response |
|---|---|---|
| `capabilities` | `{}` | `{"schema":"booknook.source-capabilities.v1","capabilities":["kindle.catalog","audible.positions","audible.authorize"],"qualified":false}` |
| `kindle.catalog` | `{}` | Existing `booknook.kindle-catalog.v1` object. |
| `audible.positions` | `{"asins":["B012345678"]}` | `booknook.audible-positions.v1` object below. |
| `account.authorize` | `{"provider":"audible","marketplace":"us"}` | Legacy `{"authorization_url":"https://www.amazon.com/..."}` remains supported. Completion-capable adapters also return `session_id`, Unix-second `expires_at`, and `requires_virtual_device_registration:true`. |
| `account.authorize.complete` | Provider, marketplace, `session_id`, secret `return_url`, `consent_virtual_device:true` | `{"schema":"booknook.account-connected.v1","provider":"audible","connected":true,"positions_qualified":false}` after actual registration and secure storage. |
| `account.authorize.cancel` | Provider, marketplace, `session_id` | `{"schema":"booknook.account-cancelled.v1","cancelled":true}`; never deletes a connected account. |
| `account.disconnect` | Provider and marketplace | `{"schema":"booknook.account-disconnected.v1","provider":"audible","disconnected":true}` after deregistering only that account/device and removing its credentials. |
| `account.registration.reset` | Provider, marketplace, `confirmed_amazon_device_review:true` | `{"schema":"booknook.account-registration-reset.v1","provider":"audible","reset":true}` only for a local uncertain registration with no saved credential, after explicit user review/removal of the unfinished Amazon device. This is not remote deregistration. |
| `audible.positions.probe` | Provider, marketplace and `asins` | `{"schema":"booknook.audible-probe.v1","received_json":true,"positions_qualified":false}`; discards raw JSON and does not save progress. |
| `account.renew` | `{"provider":"audible","marketplace":"us"}` | `{"schema":"booknook.account-renewal.v1","provider":"audible","renewed":true}` only after the adapter actually renews access. Advertise `account.renew` to enable the in-app action. |

HTTP routes are `POST /v1/accounts/authorize`, `/v1/accounts/authorize/complete`, `/v1/accounts/authorize/cancel`, `/v1/accounts/renew`, `/v1/accounts/disconnect`, `/v1/accounts/registration/reset`, and `/v1/audible/positions/probe`. For adapters advertising `account.authorize.complete`, the companion adds `device_id` from the authenticated pairing-token digest to account operations and position lookups. HTTP clients cannot supply it. The optional `marketplace` on `/v1/audible/positions` defaults to `us`; the app sends its selected marketplace. Legacy adapters continue receiving their original parameter shape without added device context. Each new operation has its own capability name.

Only advertise capabilities that really work. Book Nook permits authorization navigation only to the explicit supported Amazon/Audible HTTPS hosts. Renew account credentials inside your adapter; never return a stale fetch time as a new source-progress time.

The companion checks the advertised capability before account login, renewal or position lookup. Book Nook enables those controls only when the paired companion reports that capability. Changing the companion URL clears the previous capability state. The single `qualified` flag is a source adapter's report, not an independent certification by Book Nook.

If an optional source adapter is missing, crashes, times out or returns an invalid capability response, pairing and capability refresh still preserve independently available local Kindle import/catalog capabilities. Provider capabilities are omitted and `qualified` is false. The existing manifest display name includes a fixed “optional source adapter unavailable; check its configuration” suffix, so current clients can surface the issue without changing the closed v1 contract. Live account/position routes return a sanitized unavailable/invalid-source error; they do not pretend to succeed. Capability probes have a three-second process deadline so this fallback arrives before the native request timeout; actual account/source operations retain their 25-second deadline. Check the configured executable and its installed dependencies, then refresh capabilities to retry. An explicitly selected local file or successful upload remains usable during an adapter outage.

```json
{
  "schema":"booknook.audible-positions.v1",
  "observations":[{
    "asin":"B012345678",
    "position_milliseconds":123000,
    "source_updated_at":1700000000,
    "fetched_at":1700000030,
    "duration_milliseconds":3600000,
    "edition_identifier":null,
    "alignment":"unmatched",
    "metadata":{
      "title":"Example book",
      "authors":["Example author"],
      "ebook_identifiers":[{"provider":"kindle","id":"B098765432"}],
      "language":"en"
    }
  }]
}
```

These numbers and identifiers are contract examples, not live observations. Both times are integer Unix seconds. Missing or unqualified source time means **omit the observation**, not substitute fetch time. Audio duration and `metadata` are optional. Metadata fields are also optional; report only source evidence. `ebook_identifiers` must describe an explicitly related ebook and retain its provider namespace. Never relabel the audiobook ASIN as a Kindle ASIN. The legacy opaque `edition_identifier` is retained but is not guessed into a namespace.

The shared Rust matcher ranks exact namespaced ebook identifiers, normalized title plus author, then title alone. It returns all candidates with reasons and language/title conflict warnings; it never selects an edition automatically. In Book Nook, review an observation, choose an edition (or keep it unmatched), choose the reading pass, then explicitly confirm. The saved metadata survives history, export and reimport. `Alignment::Confirmed` means the reader confirmed the edition association; it never means that an audio timestamp has been mapped to ebook text. Old observations with no metadata still load and can be saved unmatched.

Errors return a bounded `{ "code": "source_not_configured", "error": "…" }` object. Codes include `capability_unavailable`, `pairing_denied`, `source_timeout`, `source_failure`, `renewal_failed`, `invalid_source_data`, and `payload_too_large`. Clients interpret known codes into local guidance and never display raw source bodies, which may contain account material. The Swift and Rust clients cap error-body reads at 16 KiB. Renewal failure does not claim renewed access or replace the adapter's credentials.

An [explicitly optional AGPL-3.0-only Audible adapter](../optional-audible-adapter/README.md) now implements browser/MFA completion, virtual registration, actual credential renewal/deregistration, and source-derived position normalization with optional catalog metadata. It is a separate operator installation, absent from native/SDK dependencies. The adapter reports `qualified:false` pending live-account acceptance. Its pinned upstream code is unofficial; no supported Amazon public API is claimed. Live Kindle authorization/cloud catalog access remains unimplemented; selected XML/JSON exports and the bounded current Mac cache snapshot format are available. The companion returns HTTP 503 when requested capabilities are absent.

## Issuer endpoint contract

The native authorization coordinator posts to the exact independently configured Castalia credential endpoint:

```json
{
  "schema":"booknook.credential-request.v1",
  "action_request":{"schema":"booknook.authorized-action.v1","action":"sync.send","actor":"holder-lowerhex","payload_digest":"sha256-lowerhex","resource_version":1,"idempotency_key":"unique-id"},
  "disclosure":{"title":"Sync selected notebook","statements":["Send this encrypted batch to the paired device."]},
  "holder_public_key":"holder-lowerhex",
  "audience":"booknook",
  "caller":{"kind":"terminal","id":"booknook-native"},
  "policy_digest":"configured-sha256-lowerhex"
}
```

The configured issuer returns the **existing** `castalia.credential-presentation-request.v2`. Its `request_bytes_base64` must encode the exact Rust-canonical `action_request`, and its disclosure must equal the submitted disclosure. The native receiver verifies the issuer signature against saved pins, all digests, policy, holder, caller, resource version and expiry. After displaying those verified disclosures, the native custody adapter signs only on “Approve once”. Existing v2 signature domains and closed object shapes are preserved. This endpoint request wrapper is Book Nook’s provider protocol; it is not presented as an existing Castalia server API.
