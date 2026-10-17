#!/bin/bash
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
case "$platform" in
  mac) scheme=BookNookMac; sdk=macosx; destination='generic/platform=macOS' ;;
  ipad-sim) cargo build --locked -p booknook-native --release --target aarch64-apple-ios-sim; scheme=BookNookPad; sdk=iphonesimulator; destination='generic/platform=iOS Simulator' ;;
  ipad-device) cargo build --locked -p booknook-native --release --target aarch64-apple-ios; scheme=BookNookPad; sdk=iphoneos; destination='generic/platform=iOS' ;;
  *) echo 'Use mac, ipad-sim, or ipad-device' >&2; exit 2 ;;
esac
cd apps/apple
xcodegen generate
xcodebuild -project BookNook.xcodeproj -scheme "$scheme" -sdk "$sdk" -destination "$destination" -configuration Debug -derivedDataPath "Build/$platform" ARCHS=arm64 CODE_SIGNING_ALLOWED=NO build
