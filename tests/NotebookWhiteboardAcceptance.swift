// SPDX-License-Identifier: MPL-2.0
// Actual UIKit/TextKit/PencilKit, with synthetic native drawing only.
// Gesture delivery from a finger or Pencil is qualified separately by UI tests.
#if os(iOS)
import SwiftUI
import UIKit
import PencilKit

@MainActor func runNotebookWhiteboardAcceptance() async throws {
    func require(_ condition: Bool, _ message: String) throws {
        if !condition { throw AuthorizationError.message(message) }
    }
    let pixelDirectory = FileManager.default.temporaryDirectory.appendingPathComponent("NotebookWhiteboardPixels-" + UUID().uuidString, isDirectory: true)
    try FileManager.default.createDirectory(at: pixelDirectory, withIntermediateDirectories: true)
    // Inspect pixels from the actual mounted PencilKit viewport. PKDrawing.image
    // would bypass the scroll/tile renderer whose signed-coordinate behavior is
    // under test, so it is intentionally not a fallback here.
    func checkViewport(_ board: NotebookWhiteboardHost, name: String, worldRegion: CGRect, expectedInk: Bool) async throws {
        let canvas = board.canvas
        var finalCount = -1
        for attempt in 1...3 {
            try await Task.sleep(nanoseconds: 150_000_000)
            canvas.window?.layoutIfNeeded(); canvas.layoutIfNeeded()
            let size = canvas.bounds.size
            let region = board.viewportRect(forWorld: worldRegion).integral
            try require(CGRect(origin: .zero, size: size).contains(region), "\(name): target ink region is not wholly in the viewport")
            let format = UIGraphicsImageRendererFormat(); format.scale = 1; format.opaque = true
            var rendered = false
            let image = UIGraphicsImageRenderer(size: size, format: format).image { _ in
                rendered = canvas.drawHierarchy(in: CGRect(origin: .zero, size: size), afterScreenUpdates: true)
            }
            if let png = image.pngData() { try png.write(to: pixelDirectory.appendingPathComponent("\(name)-\(attempt).png"), options: .atomic) }
            try require(rendered, "\(name): native viewport did not render completely")
            guard let crop = image.cgImage?.cropping(to: region) else { throw AuthorizationError.message("\(name): viewport crop failed") }
            var pixels = [UInt8](repeating: 0, count: crop.width * crop.height * 4)
            let drawn = pixels.withUnsafeMutableBytes { bytes -> Bool in
                guard let context = CGContext(data: bytes.baseAddress, width: crop.width, height: crop.height,
                    bitsPerComponent: 8, bytesPerRow: crop.width * 4, space: CGColorSpaceCreateDeviceRGB(),
                    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue) else { return false }
                context.draw(crop, in: CGRect(x: 0, y: 0, width: crop.width, height: crop.height)); return true
            }
            try require(drawn, "\(name): viewport pixel decoding failed")
            var red: CGFloat = 0, green: CGFloat = 0, blue: CGFloat = 0, alpha: CGFloat = 0
            try require(UIColor(readingPaper).getRed(&red, green: &green, blue: &blue, alpha: &alpha), "Paper color is not RGB")
            finalCount = stride(from: 0, to: pixels.count, by: 4).reduce(0) { count, i in
                let dark = CGFloat(pixels[i]) / 255 < red - 0.12 && CGFloat(pixels[i + 1]) / 255 < green - 0.12 && CGFloat(pixels[i + 2]) / 255 < blue - 0.12
                return count + (dark ? 1 : 0)
            }
            let metrics: [String: Any] = ["name": name, "attempt": attempt, "ink_pixels": finalCount,
                "expected_ink": expectedInk, "zoom": canvas.zoomScale,
                "offset": [canvas.contentOffset.x, canvas.contentOffset.y], "region": [region.minX, region.minY, region.width, region.height],
                "viewport": [size.width, size.height], "image": pixelDirectory.appendingPathComponent("\(name)-\(attempt).png").path]
            print("BOOKNOOK_WHITEBOARD_PIXELS " + String(decoding: try JSONSerialization.data(withJSONObject: metrics, options: [.sortedKeys]), as: UTF8.self))
            if expectedInk ? finalCount >= 8 : finalCount == 0 { return }
        }
        throw AuthorizationError.message("\(name): actual viewport ink visibility mismatch (\(finalCount) dark pixels)")
    }
    let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first!
    let prior = scene.windows.first { $0.isKeyWindow }
    let window = UIWindow(windowScene: scene)
    let controller = UIViewController(); window.rootViewController = controller
    window.frame = CGRect(x: 0, y: 0, width: 600, height: 700); window.makeKeyAndVisible()
    defer { window.isHidden = true; prior?.makeKey() }
    var text = "A separate note: café 日本語.", styles: [InlineStyle] = [], callbacks = 0
    let source = NativeNotebookEditor(text: Binding(get: { text }, set: { text = $0 }),
        styles: Binding(get: { styles }, set: { styles = $0 }), formatRequest: nil,
        changed: { _, _ in callbacks += 1 })
    let coordinator = NativeNotebookEditor.Coordinator(source)
    let editor = source.makeEditor(coordinator: coordinator, active: true)
    editor.isScrollEnabled = false
    var writes: [[String: Any]] = []
    let session = NotebookInkSession(notebookID: "board-notebook", blockID: "board-note", passID: "board-pass", reader: "board-reader", ink: nil, persist: { writes.append($0) })
    let tools = NotebookWhiteboardTools()
    let board = NotebookWhiteboardHost(session: session, tools: tools)
    let faces = NotebookNoteFacesHost(editor: editor, board: board)
    let notebookScroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 480, height: 480))
    notebookScroll.contentSize = CGSize(width: 480, height: 960)
    controller.view.addSubview(notebookScroll)
    notebookScroll.addSubview(faces); faces.frame = CGRect(x: 0, y: 0, width: 480, height: 480)
    faces.showBoard(false, active: true); faces.layoutIfNeeded()
    try require(board.isHidden && !editor.isHidden, "Normal note face exposes whiteboard instead of prose")
    editor.becomeFirstResponder(); editor.selectedRange = NSRange(location: editor.text.utf16.count, length: 0)
    editor.insertText(" More text.")
    await Task.yield()
    try require(callbacks > 0 && text.hasSuffix(" More text."), "Real native text callback missing")
    let exactText = text, textUndo = editor.undoManager
    coordinator.setPaneActive(false, view: editor); faces.showBoard(true, active: true)
    faces.layoutIfNeeded()
    try require(!editor.isFirstResponder && editor.isHidden && !board.isHidden, "Flip leaves hidden prose active")
    try require(editor.undoManager === textUndo && editor.undoManager !== board.canvas.undoManager, "Flip replaced or shared prose undo")
    try require(board.canvas.drawingPolicy == .anyInput && board.canvas.isScrollEnabled,
                "Finger drawing is disabled or board navigation is unavailable")
    try require(board.canvas.panGestureRecognizer.minimumNumberOfTouches == 2 && board.canvas.panGestureRecognizer.maximumNumberOfTouches == 2,
                "A one-finger stroke can be claimed by canvas panning")
    try require(board.canvas.panGestureRecognizer.allowedTouchTypes == [NSNumber(value: UITouch.TouchType.direct.rawValue)]
                && board.canvas.pinchGestureRecognizer?.isEnabled == true,
                "Two-finger navigation lost direct touch or pinch zoom")
    try require(board.canvas.frame.size == faces.bounds.size, "Whiteboard allocates its document extent as a UIView")

    let positiveRegion = CGRect(x: 15, y: 20, width: 85, height: 60)
    board.setWorldViewportOrigin(.zero)
    try await checkViewport(board, name: "blank-control", worldRegion: positiveRegion, expectedInk: false)
    let stroke = PKStroke(ink: PKInk(.pen, color: UIColor(nookInk)), path: PKStrokePath(controlPoints: [
        PKStrokePoint(location: CGPoint(x: 25, y: 30), timeOffset: 0, size: CGSize(width: 3, height: 3), opacity: 1, force: 0.5, azimuth: 0, altitude: .pi / 2),
        PKStrokePoint(location: CGPoint(x: 90, y: 70), timeOffset: 0.1, size: CGSize(width: 3, height: 3), opacity: 1, force: 0.6, azimuth: 0, altitude: .pi / 2)
    ], creationDate: Date(timeIntervalSince1970: 0)))
    board.canvasViewDidBeginUsingTool(board.canvas)
    try require(!notebookScroll.isScrollEnabled && !board.canvas.panGestureRecognizer.isEnabled
                && board.canvas.pinchGestureRecognizer?.isEnabled == false,
                "Writing can move its canvas or enclosing notebook")
    board.canvas.drawing = board.rendererDrawing(PKDrawing(strokes: [stroke]))
    board.canvasViewDidEndUsingTool(board.canvas)
    try require(notebookScroll.isScrollEnabled && board.canvas.panGestureRecognizer.isEnabled
                && board.canvas.pinchGestureRecognizer?.isEnabled == true,
                "Finishing a stroke does not restore notebook scroll, two-finger pan and pinch zoom")
    try await session.flush()
    let original = session.currentVersion!, bytes = original.nativeDrawing
    try require(writes.count == 1 && original.canvasSpace?.kind == "whiteboard", "First board capture lost its coordinate space or duplicated")
    try require(original.strokes[0].points[0].x == 25 && original.strokes[0].points[0].y == 30,
                "Renderer bias leaked into the saved positive-world original")
    try await checkViewport(board, name: "positive-baseline", worldRegion: positiveRegion, expectedInk: true)
    let originalExtent = board.geometry.extent
    board.setWorldViewportOrigin(CGPoint(x: -1400, y: -1700))
    board.expandVisibleExtent()
    try require(board.geometry.extent.minX < originalExtent.minX && board.geometry.extent.minY < originalExtent.minY, "Pan near left/top does not grow world extent")
    try require(board.canvas.contentOffset.x >= 0 && board.canvas.contentOffset.y >= 0 && board.canvas.contentInset == .zero,
                "Signed world space reached PencilKit through negative tile coordinates")
    func firstWorldPoint() -> CGPoint? {
        guard let first = board.worldDrawing.strokes.first else { return nil }
        return first.path[0].location.applying(first.transform)
    }
    let rendererBeforeNavigation = board.canvas.drawing.dataRepresentation()
    try require(firstWorldPoint() == CGPoint(x: 25, y: 30), "Extent growth translated existing world strokes")
    tools.zoomIn?(); tools.zoomOut?()
    faces.frame.size.width = 350; faces.layoutIfNeeded()
    board.capture(); board.update(active: true); board.capture()
    try require(board.canvas.frame.width == 350 && firstWorldPoint() == CGPoint(x: 25, y: 30), "Resize or zoom rewrote world ink coordinates")
    try require(board.canvas.drawing.dataRepresentation() == rendererBeforeNavigation, "Pan, zoom or resize rewrote the renderer's drawing or undo space")
    try require(session.versions.count == 1 && session.currentVersion?.nativeDrawing == bytes, "Navigation produced a new ink version")

    let negative = PKDrawing(strokes: [stroke]).transformed(using: CGAffineTransform(translationX: -1500, y: -1800)).strokes[0]
    board.canvasViewDidBeginUsingTool(board.canvas)
    board.canvas.drawing = board.rendererDrawing(PKDrawing(strokes: [stroke, negative]))
    board.canvasViewDidEndUsingTool(board.canvas)
    try await session.flush()
    let second = session.currentVersion!
    board.setWorldViewportOrigin(CGPoint(x: -1540, y: -1840))
    board.expandVisibleExtent()
    try await checkViewport(board, name: "negative-world", worldRegion: positiveRegion.offsetBy(dx: -1500, dy: -1800), expectedInk: true)
    try require(notebookDrawing(second).bounds.minX < 0 && second.strokes[1].points[0].x == -1475 && second.strokes[1].points[0].y == -1770,
                "Native or portable saved drawing contains renderer coordinates instead of signed world coordinates")
    let negativeOnly = NotebookInkVersion(id: "negative-recognition", parents: [], reader: "board-reader", at: 1,
        passId: "board-pass", positions: [], positionObservations: nil, canvasWidth: 300, canvasHeight: 300,
        strokes: [second.strokes[1]], nativeDrawing: nil,
        canvasSpace: NotebookCanvasSpace(kind: "whiteboard", originX: -1600, originY: -1900))
    let raster = try NotebookInkRecognizer.raster(negativeOnly)
    try require(raster.width > 0 && raster.height > 0 && raster.width <= 2048 && raster.height <= 2048,
                "Negative-coordinate original cannot be rendered for transcription")
    try require(second.parents == [original.id] && second.strokes.count == 2 && second.strokes[1].points[0].x < 0,
                "Expanded drawing lost original parent or absolute negative coordinates")
    let restoredSession = NotebookInkSession(notebookID: "board-notebook", blockID: "board-note", passID: "board-pass", reader: "board-reader",
        ink: NotebookInk(schema: session.ink?.schema ?? "booknook.notebook-ink.v1", versions: session.versions, transcriptions: []), persist: { _ in })
    let restoredBoard = NotebookWhiteboardHost(session: restoredSession, tools: NotebookWhiteboardTools())
    restoredBoard.update(active: true)
    let restoredPoints = try JSONDecoder().decode([NotebookInkStroke].self,
        from: JSONSerialization.data(withJSONObject: portableInk(restoredBoard.worldDrawing)))
    try require(restoredBoard.canvas.drawingPolicy == .anyInput && restoredBoard.worldDrawing.strokes.count == 2
                && restoredPoints == second.strokes
                && restoredSession.currentVersion?.nativeDrawing == second.nativeDrawing,
                "Reopening the canvas disables finger input or loses saved native/portable ink")
    try require(session.versions.first?.nativeDrawing == bytes && text == exactText, "Drawing changed original ink or hidden prose")
    // Register a synthetic native drawing action, then exercise the same board
    // command used by its menu. Its undo must not reach TextKit's responder chain.
    let previousDrawing = board.rendererDrawing(PKDrawing(strokes: [stroke]))
    board.canvas.undoManager?.registerUndo(withTarget: board.canvas) { target in target.drawing = previousDrawing }
    tools.undo?(); try await session.flush()
    try require(session.currentVersion?.strokes.count == 1 && text == exactText && session.versions.contains { $0.id == second.id },
                "Board undo changed prose or discarded an immutable drawing")
    faces.showBoard(false, active: true); coordinator.setPaneActive(true, view: editor)
    try require(faces.editor === editor && editor.text == exactText && editor.undoManager === textUndo, "Returning to text lost editor identity, prose or undo")
    editor.becomeFirstResponder(); editor.undoManager?.undo(); await Task.yield()
    try require(text == "A separate note: café 日本語.", "Text undo did not survive a board flip")
    editor.undoManager?.redo(); await Task.yield()
    try require(text == exactText && session.currentVersion?.strokes.count == 1, "Text redo changed board history")
    let count = session.versions.count
    faces.showBoard(true, active: true); faces.showBoard(false, active: true)
    await Task.yield()
    try require(session.versions.count == count, "Repeated flips create drawing side effects")

    var geometry = NotebookWhiteboardGeometry()
    _ = geometry.expand(around: CGRect(x: -999_999, y: 999_000, width: 1500, height: 1500))
    try require(NotebookWhiteboardGeometry.valid(geometry.extent) && geometry.extent.minX == -1_000_000 && geometry.extent.maxY == 1_000_000,
                "Whiteboard expansion crosses the portable finite world bound")
    let bounded = geometry
    try require(!geometry.expand(around: CGRect(x: CGFloat.infinity, y: 0, width: 10, height: 10)) && geometry == bounded, "Invalid viewport changed board extent")
    print("BOOKNOOK_WHITEBOARD: actual mounted text/board flip, finger/Pencil policy, two-finger pan and pinch configuration, stroke gesture ownership/restoration, synthetic capture and reload, separate native undo, signed-world extent growth and actual positive/negative viewport pixels, zoom/resize without stroke translation, immutable originals and finite bounds passed. No physical finger or Pencil gesture claim.")
}
#endif
