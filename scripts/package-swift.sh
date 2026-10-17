#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
set -euo pipefail
cd "$(dirname "$0")/.."
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0
export CLANG_MODULE_CACHE_PATH="$PWD/target/swift-sdk-module-cache"
export SWIFTPM_MODULECACHE_OVERRIDE="$PWD/target/swift-sdk-module-cache"
case "${1:-}" in
  --use-built) ;;
  '') for platform in mac ipad-sim ipad-device; do bash scripts/build-apple.sh "$platform"; done ;;
  *) echo 'Use no arguments to build, or --use-built after matching final Apple builds' >&2; exit 2 ;;
esac
python3 scripts/sync-package-resources.py --check
python3 - <<'PY'
from pathlib import Path
import shutil, tomllib
root=Path.cwd()
version=tomllib.loads((root/'Cargo.toml').read_text())['workspace']['package']['version']
output=root/'target/sdk-package-check/swift'
if output.exists(): shutil.rmtree(output)
headers=output/'headers'
headers.mkdir(parents=True)
shutil.copy2(root/'apps/apple/Generated/booknook_nativeFFI.h',headers/'booknook_nativeFFI.h')
shutil.copy2(root/'apps/apple/Generated/booknook_nativeFFI.modulemap',headers/'module.modulemap')
sources=root/'sdk/swift/Sources/BookNookNative'
sources.mkdir(parents=True,exist_ok=True)
shutil.copy2(root/'apps/apple/Generated/booknook_native.swift',sources/'booknook_native.swift')
artifact=root/'sdk/swift/BookNookNative.xcframework'
if artifact.exists(): shutil.rmtree(artifact)
consumer=output/'consumer'
(consumer/'Sources/Smoke').mkdir(parents=True)
(consumer/'Package.swift').write_text('''// swift-tools-version: 5.9
import PackageDescription
let package = Package(name: "BookNookConsumer", platforms: [.macOS(.v14)],
 dependencies: [.package(name: "BookNookSDK", path: "SDK_PATH")],
 targets: [.executableTarget(name: "Smoke", dependencies: [
  .product(name: "BookNookNative", package: "BookNookSDK"),
  .product(name: "BookNookServices", package: "BookNookSDK"),
  .product(name: "CastaliaNative", package: "BookNookSDK")])])
'''.replace('SDK_PATH', f'../extracted/booknook-swift-sdk-{version}'))
(consumer/'Sources/Smoke/Consumer.swift').write_text('''import Foundation
import BookNookNative
@main struct Consumer {
@MainActor static func main() async throws {
guard let fixtures = ProcessInfo.processInfo.environment["BOOKNOOK_SMOKE_FIXTURES"] else {
  fatalError("The acceptance fixture directory must be supplied")
}
let fixtureRoot = URL(fileURLWithPath: fixtures)
try await runPackagedCastaliaSmoke(fixtureRoot: fixtureRoot)
try await runPackagedProviderSmoke(fixtureRoot: fixtureRoot)
try runPackagedNotebookSmoke()
let root = FileManager.default.temporaryDirectory.appendingPathComponent("booknook-sdk-consumer-\\(UUID().uuidString)")
defer { try? FileManager.default.removeItem(at: root) }
let desk = try Desk(directory: root.path)
let connections = try validateProviderConnections(connectionsJson: "[]")
precondition(connections == "[]")
let result = try desk.execute(commandJson: #"{"op":"add_book","title":"Packaged Swift 🌿","author":"Fixture","at":1}"#)
let response = try JSONSerialization.jsonObject(with: Data(result.utf8)) as! [String:Any]
let state = response["state"] as! [String:Any]
let books = state["books"] as! [String:[String:Any]]
precondition(books.values.first?["title"] as? String == "Packaged Swift 🌿")
let reopened = try Desk(directory: root.path)
let firstSnapshot = try desk.snapshot()
let secondSnapshot = try reopened.snapshot()
precondition(secondSnapshot == firstSnapshot)
print("Independent packaged Swift consumer linked native Rust and persisted/reopened a temporary desk.")
}
}
''')
for name in ['PackagedCastaliaSmoke.swift','PackagedProviderSmoke.swift','PackagedNotebookSmoke.swift']:
    shutil.copy2(root/'tests'/name, consumer/'Sources/Smoke'/name)
fixtures=output/'fixtures'
fixtures.mkdir()
shutil.copy2(root/'tests/fixtures/provider-sample.epub',fixtures/'provider-sample.epub')
(output/'version').write_text(version)
PY
headers=target/sdk-package-check/swift/headers
xcrun xcodebuild -create-xcframework \
  -library target/release/libbooknook_native.a -headers "$headers" \
  -library target/aarch64-apple-ios-sim/release/libbooknook_native.a -headers "$headers" \
  -library target/aarch64-apple-ios/release/libbooknook_native.a -headers "$headers" \
  -output sdk/swift/BookNookNative.xcframework
python3 - <<'PY'
from pathlib import Path
import gzip, os, tarfile
root=Path.cwd()
version=(root/'target/sdk-package-check/swift/version').read_text()
output=root/'target/packages'
output.mkdir(parents=True,exist_ok=True)
name=f'booknook-swift-sdk-{version}'
# Stable metadata makes repeated archives reproducible for the same compiled inputs.
def normalized(info):
    info.uid=info.gid=0; info.uname=info.gname=''; info.mtime=0
    return info
with (output/f'{name}.tar.gz').open('wb') as raw:
    with gzip.GzipFile(filename='',mode='wb',fileobj=raw,mtime=0) as compressed:
        with tarfile.open(fileobj=compressed,mode='w') as archive:
            for child in sorted((root/'sdk/swift').iterdir()):
                if child.name.startswith('.'): continue
                archive.add(child,arcname=f'{name}/{child.name}',filter=normalized)
print(f'Self-contained Swift package: {output/name}.tar.gz; nothing published.')
extracted=root/'target/sdk-package-check/swift/extracted'
extracted.mkdir(parents=True,exist_ok=True)
with tarfile.open(output/f'{name}.tar.gz') as archive:
    archive.extractall(extracted,filter='data')
package=extracted/name
# Do not let normalized archive timestamps reuse an older consumer build when
# the same local package path is verified again. Archive contents stay unchanged.
for source in package.rglob('*'):
    if source.is_file(): os.utime(source,None)
for required in ['Package.swift','Sources/BookNookNative/booknook_native.swift',
                 'Sources/BookNookServices/BookNookProvider.swift',
                 'Sources/BookNookServices/BookNookNotebook.swift',
                 'Sources/CastaliaNative/CastaliaSession.swift',
                 'LICENSE-MIT','LICENSE-APACHE','DEPENDENCY-LICENSES.csv']:
    assert (package/required).is_file(), required
for platform in ['macos-arm64','ios-arm64','ios-arm64-simulator']:
    for required in ['Headers/module.modulemap','Headers/booknook_nativeFFI.h','libbooknook_native.a']:
        assert (package/'BookNookNative.xcframework'/platform/required).is_file(), (platform,required)
PY
xcrun swift build --package-path target/sdk-package-check/swift/consumer \
  --scratch-path target/swift-sdk-build --cache-path target/swift-sdk-cache \
  --config-path target/swift-sdk-config --security-path target/swift-sdk-security
# Compile first so the public synthetic 120-second credential is fresh when used.
cargo build --locked -p booknook-native --example castalia-fixture
target/debug/examples/castalia-fixture > target/sdk-package-check/swift/fixtures/castalia-generic-fixture.json
swift_consumer_bin="$(xcrun swift build --show-bin-path --package-path target/sdk-package-check/swift/consumer \
  --scratch-path target/swift-sdk-build --cache-path target/swift-sdk-cache \
  --config-path target/swift-sdk-config --security-path target/swift-sdk-security)"
BOOKNOOK_SMOKE_FIXTURES="$PWD/target/sdk-package-check/swift/fixtures" "$swift_consumer_bin/Smoke"
target/debug/examples/castalia-fixture verify < target/sdk-package-check/swift/fixtures/castalia-swift-receipt.json
echo 'The extracted Swift archive passed independent provider, standalone notebook storage, Castalia custody and native persistence acceptance.'
