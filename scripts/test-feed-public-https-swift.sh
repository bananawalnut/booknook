#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Fresh host-only generated binding; default smoke never requests DNS or HTTP.
set -euo pipefail
cd "$(dirname "$0")/.."
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0
feed_target="$(cargo metadata --locked --offline --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
feed_check="$feed_target/feed-sdk-swift"
mkdir -p "$feed_check/generated" "$feed_check/module-cache" "$feed_check/clang-cache"
export CLANG_MODULE_CACHE_PATH="$feed_check/clang-cache"
cargo build --locked --offline -p booknook-native --features bindgen --lib --bin uniffi-bindgen
"$feed_target/debug/uniffi-bindgen" generate --library "$feed_target/debug/libbooknook_native.dylib" --language swift --out-dir "$feed_check/generated"
xcrun swiftc -parse-as-library -swift-version 5 -module-cache-path "$feed_check/module-cache" \
  -I "$feed_check/generated" -Xcc "-fmodule-map-file=$feed_check/generated/booknook_nativeFFI.modulemap" \
  -L "$feed_target/debug" -lbooknook_native -Xlinker -rpath -Xlinker "$feed_target/debug" \
  "$feed_check/generated/booknook_native.swift" sdk/swift/Sources/BookNookServices/BookNookFeedSources.swift \
  tests/FeedPublicHTTPSSDKBindingSmoke.swift -o "$feed_check/public-https-consumer"
"$feed_check/public-https-consumer" "$PWD/crates/core/tests/fixtures/feeds-v1"
