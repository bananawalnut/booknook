#!/bin/bash
# SPDX-License-Identifier: MPL-2.0
set -euo pipefail
cd "$(dirname "$0")/.."
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
recovery_test_build="${BOOKNOOK_RECOVERY_TEST_BUILD:-target/recovery-tests}"
mkdir -p "$recovery_test_build"
xcrun swiftc -warnings-as-errors -parse-as-library -module-cache-path "$recovery_test_build/module-cache" \
  sdk/swift/Sources/CastaliaNative/CastaliaContracts.swift \
  sdk/swift/Sources/CastaliaNative/CastaliaRecovery.swift \
  apps/apple/BookNook/RecoveryImport.swift tests/RecoveryImportTests.swift -o "$recovery_test_build/workflow"
"$recovery_test_build/workflow"
xcrun swiftc -warnings-as-errors -parse-as-library -module-cache-path "$recovery_test_build/module-cache" \
  apps/apple/BookNook/ConnectionSave.swift apps/apple/BookNook/ConnectorFailure.swift apps/apple/BookNook/CompanionContracts.swift tests/ConnectionSaveTests.swift -o "$recovery_test_build/connections"
"$recovery_test_build/connections"
