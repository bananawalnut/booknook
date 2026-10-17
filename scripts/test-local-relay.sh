#!/bin/bash
# SPDX-License-Identifier: MPL-2.0
set -euo pipefail
cd "$(dirname "$0")/.."
if [ ! -x "${BOOKNOOK_RELAY_BINARY:-}" ]; then
  echo "Set BOOKNOOK_RELAY_BINARY to the pinned independent nostr-rs-relay executable; see docs/LOCAL-RELAY-VALIDATION.md." >&2
  exit 1
fi
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0
cargo test --locked -p booknook-native --lib independent_relay_roundtrip_outage_and_durable_retry -- --ignored --nocapture
