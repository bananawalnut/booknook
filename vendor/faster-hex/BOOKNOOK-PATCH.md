# Book Nook dependency backport

The upstream faster-hex 0.10.1 registry archive has SHA-256 04839bdf9d8c10f66806fad16b852fc72aab80873aebc3cb69d85b4fa41543ed and records upstream commit c817979830ea5be2eafe0b0c99cb83b017ae2b8f. Source, build script, upstream tests and all license notices are retained verbatim. The upstream development Cargo.lock is omitted; it is not a Book Nook build input. Book Nook changed the heapless requirement from 0.8 to exactly 0.9.4 in the Cargo-normalized and original manifests. The application and relay-probe workspace lockfiles control builds. The Book Nook workspace and relay-probe locks both select the fixed dependency.

This source is a Book Nook path patch for the crates.io faster-hex 0.10.1 requirement used by nostr and nostr-sdk. Its bytes differ from the original registry crate. Cargo applies a registry override only at the root workspace. Consumers building the Book Nook Rust preview must use the supplied patch when resolving source packages. The upstream SPDX license is MIT; third-party notices are in LICENSE-THIRD-PARTY.

[Registry source](https://crates.io/crates/faster-hex/0.10.1) · [Upstream Git revision](https://github.com/nervosnetwork/faster-hex/tree/c817979830ea5be2eafe0b0c99cb83b017ae2b8f) · [Patched heapless release](https://crates.io/crates/heapless/0.9.4)
