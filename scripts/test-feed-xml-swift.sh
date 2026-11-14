#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Fresh generated SDK consumer only; no app/device/signing/packaging changes.
set -euo pipefail
cd "$(dirname "$0")/.."
bash scripts/test-feed-sdk-swift.sh
feed_target="$(cargo metadata --locked --offline --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
feed_check="$feed_target/feed-sdk-swift"
xcrun swiftc -parse-as-library -swift-version 5 -module-cache-path "$feed_check/module-cache" \
  -I "$feed_check/generated" -Xcc "-fmodule-map-file=$feed_check/generated/booknook_nativeFFI.modulemap" \
  -L "$feed_target/debug" -lbooknook_native -Xlinker -rpath -Xlinker "$feed_target/debug" \
  "$feed_check/generated/booknook_native.swift" sdk/swift/Sources/BookNookServices/BookNookFeedXML.swift \
  tests/FeedXMLSDKBindingSmoke.swift -o "$feed_check/xml-consumer"
"$feed_check/xml-consumer" "$PWD/crates/core/tests/fixtures/feed-xml-v1/cases.json"
