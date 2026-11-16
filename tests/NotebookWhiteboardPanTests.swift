// SPDX-License-Identifier: MPL-2.0
// The real AppKit board and menu tool closures, with synthetic immutable ink.
import AppKit
import SwiftUI

let readingPaper = Color.white
func notebookInkColor(_ hex: String) -> Color { .black }
struct NotebookCanvasSpace { let originX: CGFloat; let originY: CGFloat }
struct NotebookInkPoint: Equatable { let x: CGFloat; let y: CGFloat }
struct NotebookInkStroke: Equatable {
    let points: [NotebookInkPoint]; let color: String; let width: CGFloat
}
struct NotebookInkVersion {
    let canvasWidth: CGFloat; let canvasHeight: CGFloat; let canvasSpace: NotebookCanvasSpace?
    let strokes: [NotebookInkStroke]
}
final class NotebookInkSession: ObservableObject { var currentVersion: NotebookInkVersion? }

@main struct NotebookWhiteboardPanTests {
    @MainActor static func main() {
        let app = NSApplication.shared; app.setActivationPolicy(.prohibited)
        let tools = NotebookWhiteboardTools()
        let board = NotebookMacBoardView(tools: tools)
        board.frame = CGRect(x: 0, y: 0, width: 400, height: 480)
        let saved = [NotebookInkStroke(points: [NotebookInkPoint(x: -160, y: -140)], color: "#315b48", width: 3)]
        board.version = NotebookInkVersion(canvasWidth: 640, canvasHeight: 700,
            canvasSpace: NotebookCanvasSpace(originX: -200, originY: -160), strokes: saved)
        guard let pan = tools.pan else { preconditionFailure("Scribble menu has no native pan action") }
        let point = CGPoint(x: saved[0].points[0].x, y: saved[0].points[0].y)
        func visible() -> Bool { CGRect(origin: board.origin, size: board.bounds.size).contains(point) }
        pan(.left)
        precondition(board.origin == .zero, "An inactive note accepted a stale pan action")
        board.active = true
        precondition(!visible(), "The synthetic ink must start outside the viewport")
        pan(.left); pan(.up)
        precondition(board.origin == CGPoint(x: -300, y: -360) && visible(), "Menu tools did not reach negative-world ink")
        pan(.right); pan(.down)
        precondition(board.origin == .zero && !visible(), "Inverse pan did not return to origin")
        tools.zoomIn?()
        let zoomedOrigin = board.origin
        pan(.right); pan(.down)
        precondition(board.origin == CGPoint(x: zoomedOrigin.x + 240, y: zoomedOrigin.y + 288),
                     "Pan must move the same screen distance at another zoom")
        tools.home?()
        precondition(board.origin == .zero && !visible(), "Return to origin must still reset pan and zoom")
        for _ in 0..<3_500 { pan(.right) }
        precondition(board.origin.x + board.bounds.width == NotebookWhiteboardGeometry.limit && board.origin.y == 0,
                     "Bounded pan exposed a viewport past the world limit")
        let lastOrigin = board.origin
        board.active = false
        pan(.left)
        precondition(board.origin == lastOrigin && board.version?.strokes == saved, "Hidden pan or navigation changed the saved original")
        print("Scribble menu pan: actual Mac tool closure, negative ink, zoom-relative movement, origin reset, finite bounds, hidden action refusal and unchanged strokes passed.")
    }
}
