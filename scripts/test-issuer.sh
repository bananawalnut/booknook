#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0
export BOOKNOOK_ISSUER_HTTP_SERVICE="$PWD/services/castalia-issuer/booknook_issuer.py"
cargo test --locked -p booknook-native --test reference_issuer -- --include-ignored
