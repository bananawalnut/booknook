#!/bin/bash
# SPDX-License-Identifier: MPL-2.0
set -euo pipefail
cd "$(dirname "$0")/.."
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
if [ ! -f target/release/libbooknook_native.a ]; then
  echo "Build the native bridge first: bash scripts/build-apple.sh mac" >&2
  exit 1
fi
workflow_test_build=target/ui-workflow-tests
mkdir -p "$workflow_test_build"
xcrun swiftc -parse-as-library -D BOOKNOOK_TEST -swift-version 5 \
  -module-name BookNookFlowTests -module-cache-path "$workflow_test_build/module-cache" \
  -import-objc-header apps/apple/Generated/booknook_nativeFFI.h \
  -I apps/apple/Generated apps/apple/Generated/booknook_native.swift \
  sdk/swift/Sources/BookNookServices/*.swift sdk/swift/Sources/CastaliaNative/*.swift \
  apps/apple/BookNook/*.swift tests/DeskWorkflowTests.swift tests/CompanionCatalogImportTests.swift tests/CastaliaProfileImportTests.swift tests/InkRecoveryAcceptance.swift tests/NotebookInkAcceptance.swift tests/ReaderInkAcceptance.swift tests/ReaderInkAnchorAcceptance.swift tests/ReaderContextAcceptance.swift tests/PackagedProviderSmoke.swift tests/ProviderWorkflowTests.swift \
  target/release/libbooknook_native.a -lsqlite3 -lc++ -framework Security \
  -o "$workflow_test_build/desk"
if [ "${BOOKNOOK_WORKFLOW_BUILD_ONLY:-0}" = "1" ]; then exit 0; fi
"$workflow_test_build/desk"
"$workflow_test_build/desk" --reader-ink-anchor
