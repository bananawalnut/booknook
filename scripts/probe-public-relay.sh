#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
set -euo pipefail
if [ "$#" -ne 2 ] || [ "$1" != "--read-only-public-relay" ]; then
  echo "Usage: bash scripts/probe-public-relay.sh --read-only-public-relay wss://PUBLIC_RELAY" >&2
  echo "Explicitly contacts that public relay. No identity, authentication or publication; never run by normal CI." >&2
  exit 2
fi
cd "$(dirname "$0")/.."
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$(pwd)/target}"
cargo build --locked --manifest-path tests/relay-read-probe/Cargo.toml
exec "$CARGO_TARGET_DIR/debug/booknook-read-only-relay-probe" "$@"
