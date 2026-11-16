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
booknook_app_build_dir="${BOOKNOOK_APP_BUILD_DIR:-Build}"
booknook_app_configuration="${BOOKNOOK_APP_CONFIGURATION:-Debug}"
public_build="${BOOKNOOK_PUBLIC_APP_BUILD:-0}"
if [[ "$public_build" == 1 ]]; then
  if [[ "$booknook_app_configuration" != Release || -z "${CARGO_TARGET_DIR:-}" || -z "${BOOKNOOK_APP_BUILD_DIR:-}" ]]; then
    echo 'Public Apple builds require Release and explicit CARGO_TARGET_DIR and BOOKNOOK_APP_BUILD_DIR.' >&2
    exit 2
  fi
  if [[ -n "${RUSTFLAGS:-}" || -n "${CARGO_ENCODED_RUSTFLAGS:-}" ]]; then
    echo 'Public Apple builds need to set their own Rust path remapping flags.' >&2
    exit 2
  fi
fi
target_directory="$(python3 scripts/cargo_paths.py)"
if [[ "$public_build" == 1 ]]; then
  # Resolve a relative DerivedData path the same way as the xcodebuild below.
  if [[ "$booknook_app_build_dir" != /* ]]; then
    booknook_app_build_dir="$PWD/apps/apple/$booknook_app_build_dir"
  fi
  public_flags="$(python3 scripts/apple-public-build-flags.py "$PWD" "$target_directory" "$booknook_app_build_dir")"
  {
    IFS= read -r CARGO_ENCODED_RUSTFLAGS
    IFS= read -r clang_flags
    IFS= read -r xcode_clang_flags
    IFS= read -r xcode_swift_flags
  } <<< "$public_flags"
  export CARGO_ENCODED_RUSTFLAGS CARGO_PROFILE_RELEASE_DEBUG=0
  export CARGO_BUILD_BUILD_DIR="$target_directory"
  export CC_SHELL_ESCAPED_FLAGS=1
  export CFLAGS="${CFLAGS:+$CFLAGS }$clang_flags"
  export CXXFLAGS="${CXXFLAGS:+$CXXFLAGS }$clang_flags"
fi
cargo build --locked -p booknook-native --release
cargo run --locked -p booknook-native --features bindgen --bin uniffi-bindgen -- generate --library "$target_directory/release/libbooknook_native.dylib" --language swift --out-dir apps/apple/Generated
BOOKNOOK_BUILD_PLATFORM="$platform" python3 - "$target_directory" <<'PY'
from pathlib import Path
import hashlib, json, os, re, sys
root = Path.cwd()
platform = os.environ['BOOKNOOK_BUILD_PLATFORM']
assert platform in ['mac', 'ipad-sim', 'ipad-device']
target = Path(sys.argv[1])
output = target / 'apple-release-generated-normalization' / platform
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
library = target / 'release/libbooknook_native.dylib'
(output / 'receipt.json').write_text(json.dumps({'schema': 'booknook.generated-eol-normalization.v1',
    'platform': platform, 'library_sha256': hashlib.sha256(library.read_bytes()).hexdigest(),
    'ascii_eol_only': True, 'files': records}, indent=2) + '\n')
PY
case "$platform" in
  mac) native_library="$target_directory/release/libbooknook_native.a"; scheme=BookNookMac; sdk=macosx; destination='generic/platform=macOS' ;;
  ipad-sim) cargo build --locked -p booknook-native --release --target aarch64-apple-ios-sim; native_library="$target_directory/aarch64-apple-ios-sim/release/libbooknook_native.a"; scheme=BookNookPad; sdk=iphonesimulator; destination='generic/platform=iOS Simulator' ;;
  ipad-device) cargo build --locked -p booknook-native --release --target aarch64-apple-ios; native_library="$target_directory/aarch64-apple-ios/release/libbooknook_native.a"; scheme=BookNookPad; sdk=iphoneos; destination='generic/platform=iOS' ;;
  *) echo 'Use mac, ipad-sim, or ipad-device' >&2; exit 2 ;;
esac
cd apps/apple
xcodegen generate
xcode_args=(-project BookNook.xcodeproj -scheme "$scheme" -sdk "$sdk" -destination "$destination"
  -configuration "$booknook_app_configuration" -derivedDataPath "$booknook_app_build_dir/$platform"
  ARCHS=arm64 CODE_SIGNING_ALLOWED=NO "BOOKNOOK_RUST_LIBRARY=$native_library")
if [[ "$public_build" == 1 ]]; then
  xcode_args+=(DEBUG_INFORMATION_FORMAT=dwarf GCC_GENERATE_DEBUGGING_SYMBOLS=NO
    'BOOKNOOK_PUBLIC_LINK_FLAGS=-Wl,-S'
    "OTHER_CFLAGS=\$(inherited) $xcode_clang_flags"
    "OTHER_CPLUSPLUSFLAGS=\$(inherited) $xcode_clang_flags"
    "OTHER_SWIFT_FLAGS=\$(inherited) $xcode_swift_flags")
fi
xcodebuild "${xcode_args[@]}" build
