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

private final class ForwardingProbe: NSView {
    var keys: [NSEvent] = []
    override var acceptsFirstResponder: Bool { true }
    override func keyDown(with event: NSEvent) { keys.append(event) }
}

@main struct NotebookWhiteboardPanTests {
    @MainActor static func testFocusedArrowKeys() {
        // An actual native responder chain, without ordering a window onscreen
        // or posting events to the application or machine event queues.
        let window = NSWindow(contentRect: CGRect(x: 0, y: 0, width: 320, height: 240),
                              styleMask: [.titled], backing: .buffered, defer: false)
        let probe = ForwardingProbe(frame: CGRect(x: 0, y: 0, width: 320, height: 240))
        let board = NotebookMacBoardView(tools: NotebookWhiteboardTools())
        board.frame = CGRect(x: 12, y: 12, width: 160, height: 140)
        probe.addSubview(board); window.contentView = probe
        board.active = true
        let mark = NotebookInkPoint(x: -26, y: 70)
        let saved = [NotebookInkStroke(points: [mark], color: "#000000", width: 16)]
        board.version = NotebookInkVersion(canvasWidth: 256, canvasHeight: 256,
            canvasSpace: NotebookCanvasSpace(originX: -48, originY: 0), strokes: saved)
        func visible() -> Bool {
            CGRect(origin: board.origin, size: board.bounds.size).contains(CGPoint(x: mark.x, y: mark.y))
        }
        func key(_ scalar: Int, code: UInt16, flags: NSEvent.ModifierFlags = [.numericPad, .function]) -> NSEvent {
            let character = String(UnicodeScalar(scalar)!)
            return NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: flags, timestamp: 0,
                windowNumber: window.windowNumber, context: nil, characters: character,
                charactersIgnoringModifiers: character, isARepeat: false, keyCode: code)!
        }
        let left = key(NSLeftArrowFunctionKey, code: 123), right = key(NSRightArrowFunctionKey, code: 124)
        let up = key(NSUpArrowFunctionKey, code: 126), down = key(NSDownArrowFunctionKey, code: 125)
        precondition(window.makeFirstResponder(board) && window.firstResponder === board && !visible())
        window.sendEvent(left)
        precondition(board.origin == CGPoint(x: -120, y: 0) && visible(),
                     "A focused Scribble Left key must reveal the offscreen mark")
        window.sendEvent(right)
        precondition(board.origin == .zero && !visible())
        window.sendEvent(up)
        precondition(board.origin == CGPoint(x: 0, y: -105))
        window.sendEvent(down)
        precondition(board.origin == .zero)
        precondition(probe.keys.isEmpty, "Consumed arrows leaked into the surrounding note/reader responder")

        let other = [key(NSLeftArrowFunctionKey, code: 123, flags: [.numericPad,.function,.shift]),
                     key(NSRightArrowFunctionKey, code: 124, flags: [.numericPad,.function,.option]),
                     key(NSUpArrowFunctionKey, code: 126, flags: [.numericPad,.function,.control]),
                     key(NSDownArrowFunctionKey, code: 125, flags: [.numericPad,.function,.command]),
                     key(NSHomeFunctionKey, code: 115), key(97, code: 0, flags: [])]
        for event in other { window.sendEvent(event) }
        precondition(board.origin == .zero && probe.keys.map(\.characters) == other.map(\.characters)
                     && probe.keys.map(\.modifierFlags) == other.map(\.modifierFlags),
                     "Modified or non-arrow keys must continue through the native responder chain")

        precondition(window.makeFirstResponder(probe) && window.firstResponder === probe)
        window.sendEvent(left)
        precondition(board.origin == .zero && probe.keys.last === left, "A different responder owns its arrows")
        board.keyDown(with: right)
        precondition(board.origin == .zero && probe.keys.last === right, "An unfocused Scribble must forward a stray key")
        precondition(window.makeFirstResponder(board))
        board.active = false
        precondition(window.firstResponder !== board && !board.acceptsFirstResponder,
                     "A hidden Scribble must relinquish its own focus")
        board.keyDown(with: left)
        precondition(board.origin == .zero && probe.keys.last === left && board.version?.strokes == saved,
                     "Inactive keys must not change viewport or stored strokes")
        precondition(board.focusRingType == .exterior && board.focusRingMaskBounds.minX > board.bounds.minX
                     && board.focusRingMaskBounds.maxX < board.bounds.maxX, "Native ring mask must stay inside the viewport")
        print("Focused Scribble keys: real hidden-window routing, offscreen reveal/reset, modifiers/notebook forwarding, inactive focus release and unchanged strokes passed.")
    }

    @MainActor static func testViewportClipping() {
        let tools = NotebookWhiteboardTools()
        let board = NotebookMacBoardView(tools: tools)
        board.frame = CGRect(x: 0, y: 0, width: 160, height: 140)
        board.active = true
        let original = [NotebookInkStroke(points: [NotebookInkPoint(x: -26, y: 70)], color: "#000000", width: 16)]
        let drawing = NotebookInkVersion(canvasWidth: 256, canvasHeight: 256,
            canvasSpace: NotebookCanvasSpace(originX: -48, originY: 0), strokes: original)
        // Translate the real draw callback inside a larger un-clipped bitmap.
        // The left rail stands in for neighboring note controls, which are not
        // part of the canvas CGContext clip supplied by this harness.
        let width = 240, height = 180, left = 48, bottom = 20
        func render() -> [UInt8] {
            guard let context = CGContext(data: nil, width: width, height: height,
                bitsPerComponent: 8, bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue),
                let data = context.data else { preconditionFailure("Cannot allocate the Scribble viewport witness") }
            context.setFillColor(red: 0.2, green: 0.6, blue: 0.8, alpha: 1)
            context.fill(CGRect(x: 0, y: 0, width: width, height: height))
            context.translateBy(x: CGFloat(left), y: CGFloat(bottom))
            NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: true)
            board.draw(board.bounds)
            NSGraphicsContext.restoreGraphicsState()
            return Array(UnsafeRawBufferPointer(start: data, count: width * height * 4))
        }
        func railsEqual(_ pixels: [UInt8], _ paper: [UInt8]) -> Bool {
            for y in 0..<height {
                for x in 0..<width where x < left || x >= left + 160 || y < bottom || y >= bottom + 140 {
                    let start = (y * width + x) * 4
                    if pixels[start..<(start + 4)] != paper[start..<(start + 4)] { return false }
                }
            }
            return true
        }

        let paper = render()
        board.version = drawing
        let before = render()
        precondition(board.origin == .zero)
        precondition(railsEqual(before, paper), "Offscreen Scribble ink painted outside its viewport over the note controls")
        precondition(before == paper, "Ink entirely left of origin must not appear inside the board")
        guard let pan = tools.pan, let home = tools.home else { preconditionFailure("Missing Scribble navigation") }
        pan(.left)
        let after = render()
        precondition(board.origin == CGPoint(x: -120, y: 0), "Viewport clipping changed the pan step")
        precondition(railsEqual(after, paper), "Pan must not paint outside the viewport")
        let center = ((height / 2) * width + left + 94) * 4
        precondition(after[center..<(center + 4)] == [0, 0, 0, 255] && after[center..<(center + 4)] != paper[center..<(center + 4)],
                     "The saved circle must become visible in the translated board after one pan")
        home()
        precondition(board.origin == .zero && render() == paper, "Return to origin must hide the same ink without painting the rail")
        precondition(board.bounds.size == CGSize(width: 160, height: 140) && board.version?.strokes == original,
                     "Clipping must not change viewport size or the saved stroke geometry")
        print("Scribble viewport pixels: translated unclipped host leaves the note rail intact, reveals ink after Pan left, and hides it after Return to origin.")
    }

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
        testViewportClipping()
        testFocusedArrowKeys()
    }
}
