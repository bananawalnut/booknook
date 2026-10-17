#!/bin/bash
# SPDX-License-Identifier: MPL-2.0
set -euo pipefail
cd "$(dirname "$0")/.."
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
reader_test_build="${BOOKNOOK_READER_TEST_BUILD:-target/reading-tests}"
mkdir -p "$reader_test_build"
node --check apps/apple/Resources/reader.js
node --check apps/apple/Resources/reader-geometry.js
node tests/reader-geometry.cjs
xcrun swiftc -module-cache-path "$reader_test_build/module-cache" apps/apple/BookNook/ReaderGeometry.swift tests/reader-coordinates.swift -o "$reader_test_build/coordinates"
"$reader_test_build/coordinates"
xcrun swiftc -parse-as-library -module-cache-path "$reader_test_build/module-cache" apps/apple/BookNook/NotebookInkGeometry.swift tests/NotebookInkGeometryTests.swift -o "$reader_test_build/notebook-ink-geometry"
"$reader_test_build/notebook-ink-geometry"
xcrun swiftc -parse-as-library -module-cache-path "$reader_test_build/module-cache" tests/ReaderBridgeTests.swift -o "$reader_test_build/bridge"
"$reader_test_build/bridge"
xcrun swiftc -parse-as-library -module-cache-path "$reader_test_build/module-cache" apps/apple/BookNook/NativeNotebookEditor.swift tests/NotebookEditorTests.swift -o "$reader_test_build/editor"
"$reader_test_build/editor"
xcrun swiftc -parse-as-library -module-cache-path "$reader_test_build/module-cache" tests/ReaderTestSupport.swift apps/apple/BookNook/ReaderPDF.swift tests/PDFReaderTests.swift -o "$reader_test_build/pdf"
"$reader_test_build/pdf"
xcrun swiftc -parse-as-library -module-cache-path "$reader_test_build/module-cache" apps/apple/BookNook/DraftCommitState.swift apps/apple/BookNook/CharacterHistoryFields.swift tests/NotebookWorkflowTests.swift -o "$reader_test_build/workflows"
"$reader_test_build/workflows"
xcrun swiftc -parse-as-library -module-cache-path "$reader_test_build/module-cache" apps/apple/BookNook/ReaderActions.swift apps/apple/BookNook/ReaderWebBridge.swift tests/ReaderNavigationTests.swift -o "$reader_test_build/navigation"
"$reader_test_build/navigation"
