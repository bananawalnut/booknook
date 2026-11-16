#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
set -euo pipefail
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_INCREMENTAL=0
cd "$(dirname "$0")/.."
target_directory="$(python3 scripts/cargo_paths.py)"
if ! command -v wasm-bindgen >/dev/null; then
  echo "Install the matching tool first: cargo install wasm-bindgen-cli --version 0.2.127 --locked" >&2
  exit 1
fi
if [ ! -x sdk/typescript/node_modules/.bin/tsc ]; then
  npm --prefix sdk/typescript ci
fi
cargo build --locked -p booknook-native --bin booknook --example portable-fixture
cargo build --locked -p booknook-core --example contract-probe
bash scripts/build-wasm.sh nodejs
bash scripts/build-wasm.sh web
npm --prefix sdk/typescript run check
sdk/typescript/node_modules/.bin/tsc --target es2022 --module commonjs --moduleResolution node --strict \
  --outDir "$target_directory/sdk-check" sdk/typescript/index.ts
node tests/sdk-storage.cjs "$target_directory/sdk-check/index.js"
node tests/wasm-parity.cjs
node tests/test-conformance.cjs
node tests/castalia-generic-parity.cjs "$target_directory/sdk-check/index.js"
node tests/provider-runtime.cjs "$target_directory/sdk-check/index.js"

node tests/portable-archive-parity.cjs "$target_directory/sdk-check/index.js"

# Actual opaque feed review objects: original bytes, lifecycle and origin closure.
node tests/feed-sdk-parity.cjs sdk/typescript/pkg/nodejs/booknook_wasm.js "$target_directory/sdk-check/index.js"
node tests/feed-sdk-parity.cjs sdk/typescript/pkg/web/booknook_wasm.js "$target_directory/sdk-check/index.js"
sdk/typescript/node_modules/.bin/tsc --noEmit --strict --target es2022 --module nodenext --moduleResolution nodenext tests/FeedSDKTypes.mts

# Actual async source provider, conditional-cache and guarded final-CAS runtime.
node tests/feed-source-sdk.cjs sdk/typescript/pkg/nodejs/booknook_wasm.js "$target_directory/sdk-check/index.js"
sdk/typescript/node_modules/.bin/tsc --noEmit --strict --target es2022 --module nodenext --moduleResolution nodenext tests/FeedSourceSDKTypes.mts

# Pure bounded XML normalization through actual WASM and generated declarations.
node tests/feed-xml-sdk.cjs sdk/typescript/pkg/nodejs/booknook_wasm.js "$target_directory/sdk-check/index.js"
sdk/typescript/node_modules/.bin/tsc --noEmit --strict --target es2022 --module nodenext --moduleResolution nodenext tests/FeedXMLSDKTypes.mts
