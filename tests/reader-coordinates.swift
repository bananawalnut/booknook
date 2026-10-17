// SPDX-License-Identifier: MPL-2.0
import Foundation
import CoreGraphics
@main struct ReaderCoordinateTests {
    static func same(_ a: CGPoint, _ b: CGPoint) { precondition(abs(a.x-b.x)<0.00001 && abs(a.y-b.y)<0.00001, "Coordinate placement changed: \(a), \(b)") }
    static func main() {
        let scroll = ReaderCoordinates.viewportTransform(viewport: CGRect(x: 20,y: 800,width: 400,height: 300),nativeSize: CGSize(width: 800,height: 600))!
        same(CGPoint(x: 20,y: 800).applying(scroll),.zero)
        same(CGPoint(x: 120,y: 950).applying(scroll),CGPoint(x: 200,y: 300))
        same(CGPoint(x: 200,y: 300).applying(scroll.inverted()),CGPoint(x: 120,y: 950))
        let keyboard = ReaderCoordinates.viewportTransform(viewport: CGRect(x: 0,y: 200,width: 400,height: 200),nativeSize: CGSize(width: 800,height: 600))!
        same(CGPoint(x: 100,y: 300).applying(keyboard),CGPoint(x: 200,y: 200))
        let pdfRotated = ReaderCoordinates.measuredTransform(origin: CGPoint(x: 600,y: 40),xUnit: CGPoint(x: 600,y: 42),yUnit: CGPoint(x: 598,y: 40))!
        let point = CGPoint(x: 125,y: 240)
        same(point.applying(pdfRotated).applying(pdfRotated.inverted()),point)
        same(point.applying(pdfRotated),CGPoint(x: 120,y: 290))
        precondition(ReaderCoordinates.viewportTransform(viewport: .zero,nativeSize: CGSize(width: 10,height: 10)) == nil)
        precondition(ReaderCoordinates.measuredTransform(origin: .zero,xUnit: .zero,yUnit: .zero) == nil)
        print("Native reader coordinates: zoom, scroll, rotated PDF page conversion, inversion and invalid-geometry rejection passed.")
    }
}
