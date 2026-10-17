# booknook-wasm

The portable Rust core exposed through wasm-bindgen for browser and Node hosts.

Exports include document commands/reconciliation, social approval/receive, audio matching,
Castalia public configuration validation, recommendations and approved encrypted sync packets.
`social_recommendations` audits signed history and projections under independent current pins.
`prepare_sync_batch`, `seal_sync_batch` and `open_sync_batch` preserve the native packet wire;
sealing requires a fresh verified Castalia receipt and opening rechecks historical approval.
Hosts supply symmetric pairing keys, secure custody/consent, paired-peer policy and atomic
replay/outbox/application storage. These pure functions never sign, transmit or persist data.
Retry the saved packet, not a newly encrypted copy. No native signing secret is exposed.

New SDK code is MIT OR Apache-2.0. Bundled dependency inventories retain upstream license boundaries. Synthetic test vectors contain no user credentials.

This crate is part of the Book Nook source workspace. Local sibling dependencies are versioned for future registry release; no registry publication is implied. The workspace packaging script creates and verifies extracted packages together without relying on unpublished registry versions.

`validate_portable_archive` validates and roundtrips the complete native portable envelope,
including original assets and archived social/binding/pending-event records. It retains
embedded signed strings, checks referenced content hashes and relay signatures, and grants
no trust or permission. `validate_archive` still accepts a bare State. Hosts are responsible
for content-format validation and atomic multi-store persistence when restoring outside the
native Desk.

Building this crate for `wasm32-unknown-unknown` now requires a WASM-capable C compiler for
its protocol-only `nostr` / `secp256k1-sys` verification dependency. Apple clang does not
include that backend. Supply `CC_wasm32_unknown_unknown` and `AR_wasm32_unknown_unknown`
for LLVM clang/llvm-ar, or set `BOOKNOOK_ZIG` to a trusted Zig executable when running the
workspace `scripts/build-wasm.sh`. No networking or Nostr relay client enters this API.
