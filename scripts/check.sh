#!/bin/bash
set -euo pipefail
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_INCREMENTAL=0
cd "$(dirname "$0")/.."
python3 scripts/sync-package-resources.py --check
cargo fmt --all --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
node --check apps/apple/Resources/reader.js
node --check apps/apple/Resources/reader-geometry.js
node tests/reader-geometry.cjs
python3 scripts/generate-contracts.py --check
python3 tests/ontology-host.py
