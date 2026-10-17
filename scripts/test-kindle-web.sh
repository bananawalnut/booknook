#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
set -euo pipefail
cd "$(dirname "$0")/.."
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
if [ ! -f target/release/libbooknook_native.a ]; then
  echo "Build the native bridge first: bash scripts/build-apple.sh mac" >&2
  exit 1
fi
kindle_test_build=target/ui-workflow-tests
mkdir -p "$kindle_test_build"
xcrun swiftc -parse-as-library -D BOOKNOOK_TEST -swift-version 5 \
  -module-name BookNookKindleTests -module-cache-path "$kindle_test_build/module-cache" \
  -import-objc-header apps/apple/Generated/booknook_nativeFFI.h \
  -I apps/apple/Generated apps/apple/Generated/booknook_native.swift \
  sdk/swift/Sources/BookNookServices/*.swift sdk/swift/Sources/CastaliaNative/*.swift \
  apps/apple/BookNook/*.swift tests/KindleWebTests.swift \
  target/release/libbooknook_native.a -lsqlite3 -lc++ -framework Security \
  -o "$kindle_test_build/kindle-web"
"$kindle_test_build/kindle-web"
