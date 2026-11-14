#!/bin/bash
# SPDX-License-Identifier: MPL-2.0
set -euo pipefail
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_INCREMENTAL=0
cd "$(dirname "$0")/.."
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
export MACOSX_DEPLOYMENT_TARGET=14.0
export IPHONEOS_DEPLOYMENT_TARGET=17.0
platform="${1:-mac}"
cargo build --locked -p booknook-native --release
cargo run --locked -p booknook-native --features bindgen --bin uniffi-bindgen -- generate --library target/release/libbooknook_native.dylib --language swift --out-dir apps/apple/Generated
BOOKNOOK_BUILD_PLATFORM="$platform" python3 - <<'PY'
from pathlib import Path
import hashlib, json, os, re
root = Path.cwd()
platform = os.environ['BOOKNOOK_BUILD_PLATFORM']
assert platform in ['mac', 'ipad-sim', 'ipad-device']
output = root / 'target/apple-release-generated-normalization' / platform
raw_output = output / 'raw'
raw_output.mkdir(parents=True, exist_ok=True)
records = {}
for name in ['booknook_native.swift', 'booknook_nativeFFI.h', 'booknook_nativeFFI.modulemap']:
    path = root / 'apps/apple/Generated' / name
    raw = path.read_bytes()
    assert len(raw) <= 2 * 1024 * 1024, 'Generated text exceeds normalization bound'
    # UniFFI templates emit horizontal whitespace at line ends. Preserve every
    # other byte, line ending and generated symbol; never format source tokens.
    normalized = re.sub(rb'[ \t]+(?=\r?$)', b'', raw, flags=re.MULTILINE)
    assert raw.count(b'\n') == normalized.count(b'\n')
    for before, after in zip(raw.splitlines(keepends=True), normalized.splitlines(keepends=True)):
        assert re.sub(rb'[ \t]+(?=\r?$)', b'', before) == after
    (raw_output / name).write_bytes(raw)
    path.write_bytes(normalized)
    assert path.read_bytes() == normalized
    records[name] = {'raw_sha256': hashlib.sha256(raw).hexdigest(),
                     'normalized_sha256': hashlib.sha256(normalized).hexdigest(),
                     'removed_ascii_eol_bytes': len(raw) - len(normalized)}
library = root / 'target/release/libbooknook_native.dylib'
(output / 'receipt.json').write_text(json.dumps({'schema': 'booknook.generated-eol-normalization.v1',
    'platform': platform, 'library_sha256': hashlib.sha256(library.read_bytes()).hexdigest(),
    'ascii_eol_only': True, 'files': records}, indent=2) + '\n')
PY
case "$platform" in
  mac) scheme=BookNookMac; sdk=macosx; destination='generic/platform=macOS' ;;
  ipad-sim) cargo build --locked -p booknook-native --release --target aarch64-apple-ios-sim; scheme=BookNookPad; sdk=iphonesimulator; destination='generic/platform=iOS Simulator' ;;
  ipad-device) cargo build --locked -p booknook-native --release --target aarch64-apple-ios; scheme=BookNookPad; sdk=iphoneos; destination='generic/platform=iOS' ;;
  *) echo 'Use mac, ipad-sim, or ipad-device' >&2; exit 2 ;;
esac
cd apps/apple
xcodegen generate
booknook_app_build_dir="${BOOKNOOK_APP_BUILD_DIR:-Build}"
booknook_app_configuration="${BOOKNOOK_APP_CONFIGURATION:-Debug}"
xcodebuild -project BookNook.xcodeproj -scheme "$scheme" -sdk "$sdk" -destination "$destination" -configuration "$booknook_app_configuration" -derivedDataPath "$booknook_app_build_dir/$platform" ARCHS=arm64 CODE_SIGNING_ALLOWED=NO build
