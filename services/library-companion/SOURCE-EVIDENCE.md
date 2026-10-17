# Kindle for Mac metadata snapshot evidence

This is a cache-metadata reader, not an Amazon account API. It does not establish that every owned title is cached, that a cached title is purchased, or that content can be opened. Samples, dictionaries and other cached items are not classified as purchases. No reader's cache or Amazon account was available for live qualification.

The primary source is CHOI Kyong-Rok's [listKindleBooks2](https://github.com/rok-git/listKindleBooks2/tree/6e34d7b2e2ea58e77246166685b6bdc7e8152289), pinned at `6e34d7b2e2ea58e77246166685b6bdc7e8152289` (2026-10-10). Its [Objective-C implementation](https://github.com/rok-git/listKindleBooks2/blob/6e34d7b2e2ea58e77246166685b6bdc7e8152289/listKindleBooks2.m) reads the `ZBOOK` table, takes the display title from `ZDISPLAYTITLE`, and reads ASIN/authors from a binary `ZSYNCMETADATAATTRIBUTES` archive. Its declared license is MIT, copyright 2026 CHOI Kyong-Rok; the [complete notice](UPSTREAM-KINDLE-MIT-NOTICE) accompanies this independent implementation. No upstream executable, author lookup service or runtime dependency is bundled.

Book Nook's parser is original Python. It reads an explicitly selected, standalone SQLite image entirely in memory. It recognizes a real `ZBOOK` table with an ordinary integer primary key `Z_PK`, `ZDISPLAYTITLE` declared `VARCHAR` or `TEXT`, and `ZSYNCMETADATAATTRIBUTES` declared `BLOB`. Views, generated metadata columns and alternate formats are rejected. Additional ordinary table fields are not queried.

Only the published `NSKeyedArchiver` version 100000 layout is recognized. `plistlib` parses inert binary property-list values; the code never uses object unarchiving or instantiates archived classes. A bounded, cycle-rejecting UID traversal recognizes `SyncMetadataAttributes`, Foundation dictionaries/arrays/strings/numbers/data and their mutable container variants. The root must provide an `attributes` dictionary. ASIN must be an explicit 10-character ASCII alphanumeric `ASIN` value. There is no fallback from `ZBOOKID`. Author values may be strings, arrays or the published `author` dictionary wrapper. Only ASIN, display title and authors leave the parser. Other attributes, raw position/recency fields, account data and private tables do not.

The complete source is capped at 32 MiB; metadata blobs at 1 MiB; schema entries and columns at 256; records at 10,000; archive objects at 10,000; traversal depth at 40 and traversals at 50,000 per record. A five-second overall normalization deadline and a bounded SQLite instruction budget stop excessive work. SQLite queries are read-only, extensions are not loaded, attachments are denied and the query authorizer permits only the fixed metadata projection. Invalid records fail the entire import before the previous catalog is replaced. No claim of a complete library is made for an empty or nonempty snapshot.

## Snapshot consistency

Raw bytes from a WAL-mode database can omit committed data in its separate write-ahead log. The parser therefore rejects every WAL-mode header, including a checkpointed database that remains in WAL mode. Quitting Kindle alone does not prove the file is a standalone snapshot. The companion never discovers, opens or copies neighboring `-wal` or `-shm` files.

To prepare a source, explicitly select the Kindle database and use SQLite's backup operation to create a **new** snapshot; SQLite then includes committed WAL data using its normal read transaction. Set `journal_mode=DELETE` on the **backup only** before importing it. For example, after replacing both paths with reader-selected locations and ensuring the backup path does not exist:

```sh
sqlite3 -readonly '/selected/BookData.sqlite' ".backup '/chosen/new/KindleSnapshot.sqlite'"
sqlite3 '/chosen/new/KindleSnapshot.sqlite' 'PRAGMA journal_mode=DELETE;'
python3 services/library-companion/booknook_companion.py \
  --export-kindle-catalog '/chosen/new/KindleSnapshot.sqlite' \
  --output '/chosen/new/KindleCatalog.json'
```

The SQLite CLI is only an operator preparation option, not a Book Nook dependency. Do not change journal mode on the live Kindle database. Do not assume a file copied while Kindle was open contains recent changes. If the selected source is unreadable or its actual metadata schema differs, export/import remains unavailable until a compatible source artifact is supplied; the parser does not infer a replacement schema.

## Interchange and privacy

Output stays `booknook.kindle-catalog.v1` with items containing `asin`, `title`, `authors`, `position:null` and `observed_at:null`. Existing ASIN and normalized import evidence survive the unchanged Rust import. The current interchange has no field identifying a raw cache versus another normalized export; that distinction is disclosed at source selection and documented here, not encoded as an invented timestamp or capability. Live Kindle login and source-qualified progress remain unavailable.

`--kindle-catalog` normalizes a selected SQLite file before serving and retains only normalized metadata in memory; restart with a new snapshot to refresh it. JSON/XML paths retain their existing read-on-refresh behavior. Local exports are new files created with mode 0600 and never overwrite existing output. The server's raw SQLite upload path requires both a loopback-bound server and an actual loopback peer; request headers cannot opt in. The native app further restricts raw uploads to literal loopback destinations and disables proxies for that request. A remote companion can receive the locally normalized JSON; it must not receive the raw database. An accepted upload atomically replaces only a normalized catalog JSON file, never a database copy.

## Validation and remaining qualification

The checked-in [synthetic binary archive](../../tests/companion/fixtures/kindle-metadata.binary.plist) was generated by Apple's real `NSKeyedArchiver` using the included [independent Swift generator](../../tests/companion/fixtures/generate-kindle-metadata.swift). It contains no user records. SQLite and HTTP regressions verify field isolation, archive/schema rejection, bounds, committed WAL backup handling, no-overwrite CLI export, actual loopback upload and failure preservation. These tests validate the parser and transport against the inspected format; they do not qualify a real Kindle app version, reader cache or Amazon session.

The complete companion suite now contains 19 passing tests (6 prior companion/account cases and 13 snapshot/compatibility cases). The adapter-outage regression starts genuine missing, failing and malformed subprocess configurations and verifies pairing, local capabilities and selected catalog access continue; live operations fail with bounded errors and a repaired adapter can advertise its capabilities again. No outage is represented as qualified live access.

Run `python3 -m unittest discover -s tests/companion -v` using Python 3.11+ with SQLite deserialize support. The fixture generator needs macOS Foundation only when regenerating the fixture; ordinary Python tests use the checked-in bytes. No third-party Python package is required.
