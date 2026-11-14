#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Actual generated host bindings; no Apple app packaging or device changes.
set -euo pipefail
cd "$(dirname "$0")/.."
bash scripts/test-feed-sdk-swift.sh
feed_target="$(cargo metadata --locked --offline --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
feed_check="$feed_target/feed-sdk-swift"
xcrun swiftc -parse-as-library -swift-version 5 -module-cache-path "$feed_check/module-cache" \
  -I "$feed_check/generated" -Xcc "-fmodule-map-file=$feed_check/generated/booknook_nativeFFI.modulemap" \
  -L "$feed_target/debug" -lbooknook_native -Xlinker -rpath -Xlinker "$feed_target/debug" \
  "$feed_check/generated/booknook_native.swift" sdk/swift/Sources/BookNookServices/BookNookFeedSources.swift \
  tests/FeedSourceSDKBindingSmoke.swift -o "$feed_check/source-consumer"
"$feed_check/source-consumer" "$PWD/crates/core/tests/fixtures/feeds-v1"
