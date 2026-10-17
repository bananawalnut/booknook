// SPDX-License-Identifier: MPL-2.0
// Actual UIKit/TextKit/PencilKit instances, synthetic touches routed through the
// production boundary and synthetic strokes. No hardware Pencil or keyboard claim.
#if os(iOS)
import SwiftUI
import UIKit
import PencilKit

@MainActor func runAutomaticNotebookPageAcceptance() async throws {
    func require(_ condition: Bool, _ message: String) throws {
        if !condition { throw AuthorizationError.message(message) }
    }
    var text = "", styles: [InlineStyle] = [], edits: [String] = []
    let editor = NativeNotebookEditor(text: Binding(get: { text }, set: { text = $0 }),
        styles: Binding(get: { styles }, set: { styles = $0 }), formatRequest: nil,
        changed: { value, _ in edits.append(value) })
    let coordinator = NativeNotebookEditor.Coordinator(editor)
    let nativeText = editor.makeEditor(coordinator: coordinator, active: true)
    nativeText.isScrollEnabled = false
    var writes: [[String: Any]] = []
    let session = NotebookInkSession(notebookID: "automatic-notebook", blockID: "automatic-block",
        passID: "automatic-pass", reader: "automatic-reader", ink: nil,
        persist: { writes.append($0) })
    let ink = NotebookInkHost(session: session, tools: NotebookInkTools())
    let page = NotebookPageHost(editor: nativeText, ink: ink)
    ink.update(editable: true)
    page.frame = CGRect(x: 0, y: 0, width: 384, height: page.paperHeight(width: 384))
    page.layoutIfNeeded()
    let point = CGPoint(x: 70, y: 80)
    let pencil = page.target(for: point, touchType: .pencil)
    let finger = page.target(for: point, touchType: .direct)
    try require(pencil === ink.canvas || pencil?.isDescendant(of: ink.canvas) == true, "First Pencil contact is not routed to the mounted canvas")
    try require(finger === nativeText || finger?.isDescendant(of: nativeText) == true, "Finger tap is intercepted by ink instead of TextKit")
    try require(ink.canvas.isUserInteractionEnabled && session.versions.isEmpty, "Blank paper requires an input-mode switch")
    try require(!nativeText.isScrollEnabled, "A nested prose scroll viewport competes with notebook scrolling")

    // The actual capture delegate path must retain the first completed drawing;
    // changing between input destinations never reconstructs either native view.
    let stroke = PKStroke(ink: PKInk(.pen, color: .black), path: PKStrokePath(controlPoints: [
        PKStrokePoint(location: CGPoint(x: 25, y: 30), timeOffset: 0, size: CGSize(width: 3, height: 3), opacity: 1, force: 0.5, azimuth: 0, altitude: .pi / 2),
        PKStrokePoint(location: CGPoint(x: 90, y: 70), timeOffset: 0.1, size: CGSize(width: 3, height: 3), opacity: 1, force: 0.6, azimuth: 0, altitude: .pi / 2)
    ], creationDate: Date(timeIntervalSince1970: 0)))
    let drawing = PKDrawing(strokes: [stroke])
    ink.canvasViewDidBeginUsingTool(ink.canvas)
    ink.canvas.drawing = drawing
    ink.canvasViewDidEndUsingTool(ink.canvas)
    try await session.flush()
    try require(writes.count == 1 && session.currentVersion?.strokes.count == 1, "First completed stroke was lost or duplicated")
    let original = session.currentVersion!
    let bytes = original.nativeDrawing
    try require(original.parents.isEmpty, "First ink snapshot invented a prior version")

    nativeText.insertText("Typed beside original ink: café 日本語.")
    coordinator.textViewDidChange(nativeText)
    try require(text == "Typed beside original ink: café 日本語." && edits.last == text, "Existing TextKit callback no longer captures typed Unicode")
    try require(session.currentVersion?.nativeDrawing == bytes && session.versions.count == 1, "Typing changed original ink history")
    // Geometry derives from measured text and logical page dimensions, not screen offsets.
    let before = page.paperHeight(width: 384)
    installNotebookContent(String(repeating: "A long section flows on the same paper.\n", count: 100), styles: [], in: nativeText)
    let after = page.paperHeight(width: 384)
    try require(after > before, "Long prose is clipped instead of extending its notebook section")
    page.frame.size = CGSize(width: 384, height: after); page.layoutIfNeeded()
    try require(ink.canvas.drawing.strokes[0].path[0].location == CGPoint(x: 25, y: 30), "Page growth moved logical ink")
    page.frame.size = CGSize(width: 600, height: page.paperHeight(width: 600)); page.layoutIfNeeded()
    try require(ink.canvas.drawing.strokes[0].path[0].location == CGPoint(x: 25, y: 30) && session.currentVersion?.nativeDrawing == bytes,
                "Resizing replaced or reflowed original strokes")
    try require(page.target(for: CGPoint(x: -1, y: 0), touchType: .pencil) == nil, "Paper accepts input outside its bounds")

    // A new source head can arrive before updateUIView applies it. During tool
    // contact, both the visible drawing and its measuring coordinate system must
    // remain the old version; the local snapshot branches from that version.
    let branchSession = NotebookInkSession(notebookID: "incoming-notebook", blockID: "incoming-block",
        passID: "incoming-pass", reader: "local-reader",
        ink: NotebookInk(schema: "booknook.notebook-ink.v1", versions: [original], transcriptions: []), persist: { _ in })
    let branchText = editor.makeEditor(coordinator: NativeNotebookEditor.Coordinator(editor), active: true)
    branchText.isScrollEnabled = false
    let branchInk = NotebookInkHost(session: branchSession, tools: NotebookInkTools())
    let branchPage = NotebookPageHost(editor: branchText, ink: branchInk)
    branchInk.update(editable: true)
    branchPage.frame = CGRect(x: 0, y: 0, width: 384, height: branchPage.paperHeight(width: 384))
    branchPage.layoutIfNeeded()
    let visibleBounds = branchInk.canvas.bounds, visibleTransform = branchInk.canvas.transform
    let visiblePageHeight = branchPage.bounds.height
    branchInk.canvasViewDidBeginUsingTool(branchInk.canvas)
    let incoming = NotebookInkVersion(id: "larger-unseen-head", parents: [original.id], reader: "remote-reader",
        at: original.at, passId: original.passId, positions: [], positionObservations: nil,
        canvasWidth: original.canvasWidth * 2, canvasHeight: original.canvasHeight * 3,
        strokes: original.strokes, nativeDrawing: original.nativeDrawing)
    branchSession.accept(NotebookInk(schema: "booknook.notebook-ink.v1", versions: [original, incoming], transcriptions: []))
    try require(branchSession.currentVersion?.id == incoming.id, "Incoming regression did not produce a new model head")
    branchInk.update(editable: true)
    branchPage.frame.size.height = branchPage.paperHeight(width: 384)
    branchPage.setNeedsLayout(); branchPage.layoutIfNeeded()
    try require(branchPage.bounds.height == visiblePageHeight && branchInk.canvas.bounds == visibleBounds && branchInk.canvas.transform == visibleTransform,
                "An unapplied head changed the visible mid-stroke coordinate system")
    let translated = drawing.transformed(using: CGAffineTransform(translationX: 30, y: 50)).strokes[0]
    let localDrawing = PKDrawing(strokes: [stroke, translated])
    branchInk.canvas.drawing = localDrawing
    let localBytes = branchInk.canvas.drawing.dataRepresentation().base64EncodedString()
    branchInk.canvasViewDidEndUsingTool(branchInk.canvas)
    try await branchSession.flush()
    try require(branchSession.currentVersion?.parents == [original.id] && branchSession.currentVersion?.nativeDrawing == localBytes,
                "The completed drawing lost original bytes or adopted an unseen parent")
    try require(branchSession.currentVersion?.canvasWidth == original.canvasWidth && branchSession.currentVersion?.canvasHeight == original.canvasHeight,
                "The completed drawing was saved with unapplied dimensions")
    try require(branchSession.versions.contains { $0.id == incoming.id }, "The incoming interpretation was discarded")
    try await runAutomaticNotebookEditingAcceptance()
    print("Automatic notebook page: actual native Pencil/finger routing, mounted blank canvas, first synthetic stroke, separate Unicode prose, immutable original, measured growth/resize and unapplied-head geometry/parent isolation passed. No physical input claim.")
}
#endif
