#!/bin/bash
set -euo pipefail
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_INCREMENTAL=0
cd "$(dirname "$0")/.."
# Nostr archive verification builds secp256k1 C for WASM. Apple clang omits this
# backend; supply LLVM's CC_wasm32_unknown_unknown/AR_wasm32_unknown_unknown, or Zig.
if [ -n "${BOOKNOOK_ZIG:-}" ]; then
  export CC_wasm32_unknown_unknown="$PWD/scripts/wasm-zig-cc.py"
  export AR_wasm32_unknown_unknown="$PWD/scripts/wasm-zig-ar.py"
  export ZIG_GLOBAL_CACHE_DIR="${ZIG_GLOBAL_CACHE_DIR:-$PWD/target/zig-global-cache}"
  export ZIG_LOCAL_CACHE_DIR="${ZIG_LOCAL_CACHE_DIR:-$PWD/target/zig-local-cache}"
fi
cargo build --locked -p booknook-wasm --release --target wasm32-unknown-unknown
# Match the wasm-bindgen version in Cargo.lock (currently 0.2.127).
wasm_target="${1:-web}"
case "$wasm_target" in web|nodejs) ;; *) echo 'Use web or nodejs' >&2; exit 2 ;; esac
wasm-bindgen target/wasm32-unknown-unknown/release/booknook_wasm.wasm --target "$wasm_target" --out-dir "sdk/typescript/pkg/$wasm_target"
if [ "$wasm_target" = web ]; then
  echo '{"type":"module"}' > sdk/typescript/pkg/web/package.json
else
  echo '{"type":"commonjs"}' > sdk/typescript/pkg/nodejs/package.json
fi
