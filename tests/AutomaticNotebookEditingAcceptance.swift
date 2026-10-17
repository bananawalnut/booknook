// SPDX-License-Identifier: MPL-2.0
#if os(iOS)
import SwiftUI
import UIKit
import PencilKit

/// UIKit-only synthetic acceptance. It mounts the production page in a task app
/// window and calls native text/undo APIs; it does not synthesize physical Pencil,
/// hardware keys, device rotation, or global host input.
@MainActor func runAutomaticNotebookEditingAcceptance() async throws {
    func require(_ value: Bool, _ message: String) throws { if !value { throw InkJournalFailure(message) } }
    func settle() async throws { try await Task.sleep(for: .milliseconds(40)) }
    guard let scene = UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene }).first else {
        throw InkJournalFailure("UIKit editing acceptance needs a connected task-app window scene")
    }
    let previous = scene.windows.first(where: \.isKeyWindow)
    let window = UIWindow(windowScene: scene), controller = UIViewController()
    window.rootViewController = controller
    var text = "", styles: [InlineStyle] = [], callbacks = 0
    let editor = NativeNotebookEditor(text: Binding(get: { text }, set: { text = $0 }),
        styles: Binding(get: { styles }, set: { styles = $0 }), formatRequest: nil,
        changed: { _, _ in callbacks += 1 })
    let coordinator = NativeNotebookEditor.Coordinator(editor)
    let native = editor.makeEditor(coordinator: coordinator, active: true); native.isScrollEnabled = false
    var inkSaves: [[String: Any]] = []
    let session = NotebookInkSession(notebookID: "editing-n", blockID: "editing-b", passID: "editing-p", reader: "editing-r", ink: nil,
                                    persist: { inkSaves.append($0) })
    let tools = NotebookInkTools(), host = NotebookInkHost(session: session, tools: tools)
    let paper = NotebookPageHost(editor: native, ink: host)
    controller.view.addSubview(paper); host.update(editable: true)
    paper.frame = CGRect(x: 0, y: 0, width: 384, height: paper.paperHeight(width: 384))
    paper.layoutIfNeeded(); window.makeKeyAndVisible()
    defer { native.resignFirstResponder(); window.isHidden = true; previous?.makeKey() }
    try require(native.becomeFirstResponder(), "Production text view did not become first responder")
    try await settle()
    let undo = native.undoManager!
    undo.removeAllActions(); undo.beginUndoGrouping()
    native.insertText("Automatic paper café 🦊 日本語.")
    undo.endUndoGrouping(); try await settle()
    let original = native.text!
    try require(text == original && callbacks > 0, "Native Unicode insertion did not deliver its real delegate callback")
    try require(undo.canUndo, "Native text insertion created no undo action")
    undo.undo(); try await settle()
    try require(native.text != original && text == native.text, "Text undo failed through the mounted paper/delegate boundary")
    undo.redo(); try await settle()
    try require(native.text == original && text == original, "Text redo did not restore exact Unicode")

    // A real marked range must survive an older external snapshot. Only committed
    // text may reach the autosave callback; no manual delegate fallback is used.
    native.selectedRange = NSRange(location: (native.text as NSString).length, length: 0)
    let prior = text, callbacksBeforeComposition = callbacks
    native.setMarkedText("仮入力", selectedRange: NSRange(location: 3, length: 0))
    try require(native.markedTextRange != nil, "UIKit did not establish marked text")
    coordinator.applySnapshot(to: native, text: "stale model echo", styles: [])
    try require(native.markedTextRange != nil && native.text.contains("仮入力"), "An old model echo replaced marked text")
    try require(text == prior && callbacks == callbacksBeforeComposition, "Uncommitted composition entered autosave")
    native.insertText("確定文")
    try await settle()
    try require(native.markedTextRange == nil && text == native.text && text.hasSuffix("確定文"), "Committed composition did not reach the real delegate")

    let stroke = PKStroke(ink: PKInk(.pen, color: .black), path: PKStrokePath(controlPoints: [
        PKStrokePoint(location: CGPoint(x: 20, y: 30), timeOffset: 0, size: CGSize(width: 3, height: 3), opacity: 1, force: 0.5, azimuth: 0, altitude: .pi / 2),
        PKStrokePoint(location: CGPoint(x: 80, y: 60), timeOffset: 0.1, size: CGSize(width: 3, height: 3), opacity: 1, force: 0.5, azimuth: 0, altitude: .pi / 2)
    ], creationDate: Date(timeIntervalSince1970: 0)))
    host.canvasViewDidBeginUsingTool(host.canvas); host.canvas.drawing = PKDrawing(strokes: [stroke]); host.canvasViewDidEndUsingTool(host.canvas)
    try await session.flush()
    let originalInk = session.currentVersion!.nativeDrawing, beforeInkUndo = text
    // Register a synthetic native canvas undo operation: no claim is made about
    // the hardware gesture that ordinarily registers PencilKit's operation.
    let inkUndo = host.canvas.undoManager!
    try require(inkUndo !== undo, "Ink and text unexpectedly share an undo stack")
    inkUndo.removeAllActions(); inkUndo.beginUndoGrouping()
    inkUndo.registerUndo(withTarget: host.canvas) { $0.drawing = PKDrawing() }
    inkUndo.endUndoGrouping(); tools.undo?(); try await session.flush()
    try require(text == beforeInkUndo && session.currentVersion!.strokes.isEmpty, "Ink undo changed prose or failed to capture erasure")
    try require(session.versions.contains { $0.nativeDrawing == originalInk }, "Ink undo discarded the saved original")
    try require(native.becomeFirstResponder(), "Text focus could not return after ink")
    undo.removeAllActions(); undo.beginUndoGrouping(); native.insertText(" after ink"); undo.endUndoGrouping()
    try await settle(); let withSuffix = text
    undo.undo(); try await settle()
    try require(text == beforeInkUndo && session.currentVersion!.strokes.isEmpty, "Text undo changed ink or crossed the wrong history")
    undo.redo(); try await settle(); try require(text == withSuffix, "Returning from ink broke text redo")

    // Bounded long-section measurement, with no machine-specific latency ceiling.
    // Actual TextKit sizeThatFits and native insert/undo run on the main actor.
    let longText = String(repeating: "Long native paper: café 日本語, stable original strokes.\n", count: 2200)
    native.resignFirstResponder(); coordinator.applySnapshot(to: native, text: longText, styles: [])
    let started = Date(); var heights: [Double] = []
    for width: CGFloat in [384, 600, 384] {
        let height = paper.paperHeight(width: width)
        try require(height.isFinite && height > 0, "Long paper produced invalid measured geometry")
        paper.frame.size = CGSize(width: width, height: height); paper.setNeedsLayout(); paper.layoutIfNeeded()
        heights.append(height)
    }
    let layoutMilliseconds = Date().timeIntervalSince(started) * 1000
    try require(native.text == longText && host.logicalCanvasSize.height <= 16384, "Long-section layout corrupted text or exceeded portable canvas bounds")
    try require(heights[1] < heights[0], "Long text did not reflow at the wider measured width")
    try require(session.versions.contains { $0.nativeDrawing == originalInk }, "Long prose reflow replaced original ink")
    try require(paper.bounds.height >= host.frame.maxY, "Ink escaped its measured paper bounds")
    if paper.bounds.height > host.frame.maxY + 1 {
        try require(!paper.inkBoundaryButton.isHidden && paper.inkBoundaryButton.frame.minY == host.frame.maxY,
                    "The long-section handwriting boundary is not visibly anchored to its measured canvas")
        var continued = false; paper.continueHandwriting = { continued = true }
        let buttonPoint = CGPoint(x: paper.inkBoundaryButton.frame.midX, y: paper.inkBoundaryButton.frame.midY)
        try require(paper.target(for: buttonPoint, touchType: .direct) === paper.inkBoundaryButton,
                    "The measured continuation control cannot receive a finger tap")
        paper.inkBoundaryButton.sendActions(for: .touchUpInside)
        try require(continued && !native.textContainer.exclusionPaths.isEmpty, "Continuation failed or its control obscures prose")
        let belowCanvas = CGPoint(x: 30, y: host.frame.maxY + 1)
        try require(paper.target(for: belowCanvas, touchType: .pencil) == nil, "Out-of-canvas Pencil input was silently mapped onto another region")
    }
    let metrics: [String: Any] = ["utf16_units": (longText as NSString).length, "widths": [384, 600, 384], "measured_heights": heights,
        "three_layouts_ms": layoutMilliseconds, "canvas_logical_height": host.logicalCanvasSize.height,
        "real_delegate_callbacks": callbacks, "manual_delegate_fallbacks": 0,
        "limits": "Synthetic native text/marked-text/undo and canvas undo registration; no hardware Pencil, palm or physical IME qualification; no performance threshold."]
    print("BOOKNOOK_AUTOMATIC_EDITING " + String(decoding: try JSONSerialization.data(withJSONObject: metrics, options: [.sortedKeys]), as: UTF8.self))
}
#endif
