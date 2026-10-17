// SPDX-License-Identifier: MPL-2.0
import Foundation
import CoreGraphics

@main struct NotebookInkGeometryTests {
    static func main() {
        let geometry = NotebookCanvasGeometry(logicalSize: NotebookCanvasGeometry.defaultSize)
        // A mark at a known logical location stays there when the pane changes
        // width. No drawing coordinates are rewritten by the viewport helper.
        let point = CGPoint(x: 384, y: 480)
        let portrait = geometry.scale(in: CGSize(width: 384, height: 480))
        let landscape = geometry.scale(in: CGSize(width: 576, height: 720))
        precondition(portrait == 0.5 && landscape == 0.75)
        precondition(point.applying(CGAffineTransform(scaleX: portrait, y: portrait)) == CGPoint(x: 192, y: 240))
        precondition(point.applying(CGAffineTransform(scaleX: landscape, y: landscape)) == CGPoint(x: 288, y: 360))
        precondition(geometry.scale(in: CGSize(width: 576, height: 480)) == 0.5, "Aspect ratio must not stretch ink")
        precondition(geometry.scale(in: .zero) == 0)
        // The same notebook/reader children receive mirrored positions, with
        // unchanged sizes and a real divider, including a narrow proposal.
        for size in [CGSize(width: 1101, height: 720), CGSize(width: 1, height: 100), .zero] {
            let left = NotebookPaneFrames(size: size, notebookOnRight: false)
            let right = NotebookPaneFrames(size: size, notebookOnRight: true)
            precondition(left.notebook.size == right.notebook.size && left.reader.size == right.reader.size)
            precondition(left.notebook.minX == 0 && right.reader.minX == 0)
            precondition(left.notebook.maxX == left.divider.minX && left.divider.maxX == left.reader.minX)
            precondition(right.reader.maxX == right.divider.minX && right.divider.maxX == right.notebook.minX)
            precondition(abs(left.reader.maxX - size.width) < 0.000001 && abs(right.notebook.maxX - size.width) < 0.000001)
        }
        for width in [320.0, 640, 740, 820, 1024, 1440] {
            for fraction in [-1.0, 0.1, 0.35, 0.5, 0.8, 2, .nan] {
                let size = CGSize(width: width, height: 700)
                let left = NotebookPaneFrames(size: size, notebookOnRight: false, notebookFraction: fraction)
                let right = NotebookPaneFrames(size: size, notebookOnRight: true, notebookFraction: fraction)
                precondition(left.notebook.width > 0 && left.reader.width > 0, "A small width must never hide either context")
                precondition(left.notebook.width.isFinite && left.reader.width.isFinite)
                precondition(left.notebook.size == right.notebook.size && left.reader.size == right.reader.size, "Swapping sides must preserve the selected pane widths")
                precondition(abs(left.notebook.width + left.divider.width + left.reader.width - width) < 0.000001)
            }
        }
        print("Notebook geometry: fixed ink coordinates, proportional scaling, adjustable clamped split, both contexts at narrow widths and mirrored sides preserving width passed.")
    }
}
