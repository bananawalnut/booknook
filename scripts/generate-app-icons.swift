// SPDX-License-Identifier: MIT OR Apache-2.0
// Convert the original artwork into native Apple asset-catalog renditions.
// Run on macOS: swift scripts/generate-app-icons.swift <repository-root>
import Foundation
import CoreGraphics
import ImageIO
import UniformTypeIdentifiers

let root = URL(fileURLWithPath: CommandLine.arguments.dropFirst().first ?? FileManager.default.currentDirectoryPath)
let artwork = root.appendingPathComponent("assets/branding/booknook-icon.png")
let catalog = root.appendingPathComponent("apps/apple/Assets.xcassets")
guard let source = CGImageSourceCreateWithURL(artwork as CFURL, nil),
      let original = CGImageSourceCreateImageAtIndex(source, 0, nil),
      original.width == original.height,
      let colorSpace = CGColorSpace(name: CGColorSpace.sRGB) else {
    fatalError("Expected square Book Nook source artwork in sRGB")
}

func writeJSON(_ value: [String: Any], at url: URL) throws {
    try JSONSerialization.data(withJSONObject: value, options: [.prettyPrinted, .sortedKeys]).write(to: url)
}

func render(pixels: Int, mac: Bool, to url: URL) throws {
    let alpha: CGImageAlphaInfo = mac ? .premultipliedLast : .noneSkipLast
    guard let context = CGContext(data: nil, width: pixels, height: pixels,
        bitsPerComponent: 8, bytesPerRow: pixels * 4, space: colorSpace,
        bitmapInfo: alpha.rawValue) else { fatalError("Cannot allocate icon bitmap") }
    context.interpolationQuality = .high
    let size = CGFloat(pixels), bounds = CGRect(x: 0, y: 0, width: size, height: size)
    if mac {
        // macOS 14 does not supply the iPad home-screen mask. Keep the
        // conventional inset rounded tile, with transparent outer padding.
        let tile = bounds.insetBy(dx: size * 0.09, dy: size * 0.09)
        let path = CGPath(roundedRect: tile, cornerWidth: size * 0.18,
                          cornerHeight: size * 0.18, transform: nil)
        context.saveGState()
        context.setShadow(offset: CGSize(width: 0, height: -size * 0.012), blur: size * 0.018,
                          color: CGColor(gray: 0, alpha: 0.25))
        context.addPath(path); context.setFillColor(CGColor(gray: 0, alpha: 1)); context.fillPath()
        context.restoreGState()
        context.addPath(path); context.clip(); context.draw(original, in: tile)
    } else {
        // iPad artwork stays opaque and full bleed; the OS rounds its corners.
        context.setFillColor(CGColor(red: 41.0 / 255, green: 63.0 / 255, blue: 56.0 / 255, alpha: 1))
        context.fill(bounds); context.draw(original, in: bounds)
    }
    guard let image = context.makeImage(),
          let destination = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil) else {
        fatalError("Cannot create icon PNG")
    }
    CGImageDestinationAddImage(destination, image, nil)
    guard CGImageDestinationFinalize(destination) else { fatalError("Cannot save icon PNG") }
}

try FileManager.default.createDirectory(at: catalog, withIntermediateDirectories: true)
try writeJSON(["info": ["author": "xcode", "version": 1]], at: catalog.appendingPathComponent("Contents.json"))
for mac in [false, true] {
    let set = catalog.appendingPathComponent(mac ? "AppIconMac.appiconset" : "AppIconPad.appiconset")
    try FileManager.default.createDirectory(at: set, withIntermediateDirectories: true)
    let slots: [(String, Int)] = mac
        ? [16, 32, 128, 256, 512].flatMap { [(String($0), 1), (String($0), 2)] }
        : [("20", 1), ("20", 2), ("29", 1), ("29", 2), ("40", 1), ("40", 2), ("76", 1), ("76", 2), ("83.5", 2), ("1024", 1)]
    var images: [[String: Any]] = []
    for (size, scale) in slots {
        let name = "icon-\(size)@\(scale)x.png", pixels = Int(Double(size)! * Double(scale))
        try render(pixels: pixels, mac: mac, to: set.appendingPathComponent(name))
        images.append(["filename": name, "idiom": mac ? "mac" : size == "1024" ? "ios-marketing" : "ipad",
                       "scale": "\(scale)x", "size": "\(size)x\(size)"])
    }
    try writeJSON(["images": images, "info": ["author": "xcode", "version": 1]], at: set.appendingPathComponent("Contents.json"))
}
print("Generated 20 iPad/macOS app-icon renditions from the original Book Nook artwork.")
