// SPDX-License-Identifier: MPL-2.0
import Foundation
import CoreGraphics

/// A virtual document extent, not a view allocation. Stroke coordinates always
/// remain in this absolute world space as the visible window moves or expands.
struct NotebookWhiteboardGeometry: Equatable {
    static let limit: CGFloat = 1_000_000
    private(set) var extent: CGRect
    init(extent: CGRect = CGRect(x: 0, y: 0, width: 1024, height: 1024)) {
        let safe = Self.valid(extent) ? extent : CGRect(x: 0, y: 0, width: 1024, height: 1024)
        self.extent = safe.union(CGRect(x: 0, y: 0, width: 1, height: 1))
    }
    static func valid(_ rect: CGRect) -> Bool {
        [rect.minX, rect.minY, rect.width, rect.height, rect.maxX, rect.maxY].allSatisfy(\.isFinite)
            && rect.width > 0 && rect.height > 0
            && rect.minX >= -limit && rect.minY >= -limit && rect.maxX <= limit && rect.maxY <= limit
    }
    static func visibleWorld(offset: CGPoint, zoom: CGFloat, viewport: CGSize) -> CGRect? {
        guard zoom.isFinite, zoom > 0, offset.x.isFinite, offset.y.isFinite,
              viewport.width.isFinite, viewport.height.isFinite, viewport.width > 0, viewport.height > 0 else { return nil }
        return CGRect(x: offset.x / zoom, y: offset.y / zoom, width: viewport.width / zoom, height: viewport.height / zoom)
    }
    @discardableResult mutating func expand(around viewport: CGRect) -> Bool {
        guard [viewport.minX, viewport.minY, viewport.width, viewport.height, viewport.maxX, viewport.maxY].allSatisfy(\.isFinite),
              viewport.width > 0, viewport.height > 0 else { return false }
        let padded = viewport.insetBy(dx: -viewport.width, dy: -viewport.height)
        let desired = extent.union(padded)
        let x = max(-Self.limit, desired.minX), y = max(-Self.limit, desired.minY)
        let next = CGRect(x: x, y: y, width: min(Self.limit, desired.maxX) - x, height: min(Self.limit, desired.maxY) - y)
        guard Self.valid(next), next != extent else { return false }
        extent = next; return true
    }
    func contains(_ point: CGPoint) -> Bool {
        point.x.isFinite && point.y.isFinite && abs(point.x) <= Self.limit && abs(point.y) <= Self.limit
    }
}
