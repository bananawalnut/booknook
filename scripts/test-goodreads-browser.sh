#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
set -euo pipefail
export DEVELOPER_DIR=${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}
root=$(cd "$(dirname "$0")/.." && pwd)
scratch=$(mktemp -d "${TMPDIR:-/tmp}/booknook-goodreads.XXXXXXXX")
trap 'rm -rf "$scratch"' EXIT
cache=${BOOKNOOK_SWIFT_MODULE_CACHE:-"$scratch/modules"}
mkdir -p "$cache"
xcrun --sdk macosx swiftc -parse-as-library -module-cache-path "$cache" \
  -target "$(uname -m)-apple-macos14.0" \
  "$root/apps/apple/BookNook/GoodreadsBrowser.swift" "$root/tests/GoodreadsBrowserAcceptance.swift" \
  -o "$scratch/goodreads-acceptance"
store=$(uuidgen)
status=0
"$scratch/goodreads-acceptance" exercise "$store" && "$scratch/goodreads-acceptance" reopen "$store" || status=$?
"$scratch/goodreads-acceptance" cleanup "$store" || status=1
if [[ "$status" -eq 0 ]]; then
  echo "Passed Goodreads navigation, denial and cookie isolation across process restart."
fi
exit "$status"
