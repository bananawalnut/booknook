// SPDX-License-Identifier: MPL-2.0
import Foundation
import CoreGraphics

/// Pure coordinate operations; all inputs originate in DOM or PDFKit measurements.
enum ReaderCoordinates {
    static func viewportTransform(viewport: CGRect, nativeSize: CGSize) -> CGAffineTransform? {
        let values = [viewport.minX, viewport.minY, viewport.width, viewport.height, nativeSize.width, nativeSize.height]
        guard values.allSatisfy(\.isFinite), viewport.width > 0, viewport.height > 0, nativeSize.width > 0, nativeSize.height > 0 else { return nil }
        // CSS pixels remain isotropic. Keyboard occlusion can shorten the visual
        // viewport without shrinking WKWebView; height must not stretch ink.
        let sx = nativeSize.width / viewport.width, sy = nativeSize.width / viewport.width
        return CGAffineTransform(a: sx, b: 0, c: 0, d: sy, tx: -viewport.minX * sx, ty: -viewport.minY * sy)
    }
    static func measuredTransform(origin: CGPoint, xUnit: CGPoint, yUnit: CGPoint) -> CGAffineTransform? {
        let t = CGAffineTransform(a: xUnit.x-origin.x, b: xUnit.y-origin.y, c: yUnit.x-origin.x, d: yUnit.y-origin.y, tx: origin.x, ty: origin.y)
        guard [t.a,t.b,t.c,t.d,t.tx,t.ty].allSatisfy(\.isFinite), abs(t.a*t.d-t.b*t.c) > 0.000001 else { return nil }
        return t
    }
}
