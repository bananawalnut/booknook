// swift-tools-version: 5.9
// SPDX-License-Identifier: MIT OR Apache-2.0
import PackageDescription

let package = Package(
    name: "BookNookSDK",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [
        .library(name: "BookNookNative", targets: ["BookNookNative"]),
        .library(name: "BookNookServices", targets: ["BookNookServices"]),
        .library(name: "CastaliaNative", targets: ["CastaliaNative"])
    ],
    targets: [
        .binaryTarget(name: "booknook_nativeFFI", path: "BookNookNative.xcframework"),
        .target(name: "BookNookNative", dependencies: ["booknook_nativeFFI"],
            linkerSettings: [.linkedLibrary("sqlite3"), .linkedLibrary("c++"), .linkedFramework("Security")]),
        .target(name: "BookNookServices", dependencies: ["BookNookNative"]),
        .target(name: "CastaliaNative", dependencies: ["BookNookNative"])
    ]
)
