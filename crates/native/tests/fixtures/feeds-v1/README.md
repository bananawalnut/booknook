# Packaged native feed fixtures

`section.json` and `conformance.json` are byte-identical mirrors of the core crate's
`tests/fixtures/feeds-v1` files. The native crate carries its own copies so its
packaged tests do not depend on source-workspace directory names.
`scripts/sync-package-resources.py --check` verifies the mirrors before packaging;
`scripts/package-rust.sh` tests the actual extracted versioned crates.

The canonical synthetic fixture owner is
[Castalia Wallet `contracts/feeds/v1`](https://github.com/ZenithResearch/castalia-wallet/tree/d688a3859a3fdc602c545790fd7065f9725435de/contracts/feeds/v1).
Its [contract and provenance](https://github.com/ZenithResearch/castalia-wallet/blob/d688a3859a3fdc602c545790fd7065f9725435de/contracts/feeds/v1/README.md)
describe this development reader corpus. These two files are unchanged at the
reviewed Wallet main `e16dc940c8c338a2dd134a9becb3f715e0b02cef`.
The corpus contains 102 raw-input cases (17 valid, 85 invalid), including duplicate
JSON properties; tests must consume the original raw strings. The section example
is synthetic and must not be fetched. These fixtures grant no authority or
activation and do not enable production Castaway v3 writers.

SHA-256:

- `section.json`: `eff3c3c4b15743661c2628d504052211f54c0ec2ab220d6d1d103218569c0ec2`
- `conformance.json`: `07dfd61336d1ccbaafe2d3cf7540255ae5345222366daf4ce4e942b83a369d8f`
