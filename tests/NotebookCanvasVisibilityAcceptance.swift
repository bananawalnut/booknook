// SPDX-License-Identifier: MPL-2.0
// Run by the isolated iPad QA entry point, before opening its persistent desk.
// Actual PencilKit lifecycle; synthetic in-memory ink, no hardware-input claim.
#if os(iOS)
import PencilKit
import UIKit

@MainActor func runNotebookCanvasVisibilityAcceptance() async throws {
    func require(_ condition: Bool, _ message: String) throws {
        if !condition { throw AuthorizationError.message(message) }
    }
    let points = [CGPoint(x: 20, y: 20), CGPoint(x: 80, y: 60)]
    let drawing = PKDrawing(strokes: [PKStroke(ink: PKInk(.pen, color: .black), path: PKStrokePath(
        controlPoints: points.enumerated().map { index, point in
            PKStrokePoint(location: point, timeOffset: Double(index) * 0.1, size: CGSize(width: 3, height: 3), opacity: 1, force: 0.5, azimuth: 0, altitude: .pi / 2)
        }, creationDate: Date(timeIntervalSince1970: 0)))])
    let originalBytes = drawing.dataRepresentation()
    let strokes = try JSONDecoder().decode([NotebookInkStroke].self, from: JSONSerialization.data(withJSONObject: portableInk(drawing)))
    let original = NotebookInkVersion(id: "visibility-original", parents: [], reader: "visibility-reader", at: 1,
        passId: "visibility-pass", positions: [], positionObservations: nil, canvasWidth: 768, canvasHeight: 960,
        strokes: strokes, nativeDrawing: originalBytes.base64EncodedString())
    for (editable, active) in [(false, true), (true, false)] {
        var writes: [[String: Any]] = []
        let session = NotebookInkSession(notebookID: "visibility-notebook", blockID: "visibility-block",
            passID: original.passId, reader: original.reader,
            ink: NotebookInk(schema: "booknook.notebook-ink.v1", versions: [original], transcriptions: []),
            persist: { writes.append($0) })
        let host = NotebookInkHost(session: session, tools: NotebookInkTools())
        session.capturePendingStroke?() // The native drawing has not been installed yet.
        host.update(editable: editable, active: active)
        try require(session.pendingCount == 0 && session.versions.count == 1, "Initial hidden/read-only canvas created an erasure")
        let rendered = host.canvas.drawing
        try require(rendered.strokes.count == drawing.strokes.count && rendered.bounds == drawing.bounds &&
                    rendered.strokes[0].path.count == drawing.strokes[0].path.count &&
                    points.indices.allSatisfy { rendered.strokes[0].path[$0].location == drawing.strokes[0].path[$0].location },
                    "Initial hidden/read-only canvas lost displayed strokes")
        // PencilKit can reserialize a loaded drawing differently. Original
        // storage bytes, unlike a renderer's serialization, must stay exact.
        try require(session.currentVersion?.nativeDrawing == original.nativeDrawing, "Initial mount replaced original storage bytes")
        try require(!host.canvas.canBecomeFirstResponder, "Hidden/read-only canvas accepts focus")
        host.update(editable: true, active: true)
        host.canvasViewDidBeginUsingTool(host.canvas)
        host.canvas.drawing = PKDrawing() // Synthetic erasure while a stroke is active.
        host.update(editable: true, active: false)
        try require(session.pendingCount == 1, "Hiding an initialized canvas failed to capture its changed drawing")
        try await session.flush()
        try require(writes.count == 1 && session.versions.count == 2, "Hide/flush duplicated the captured drawing")
        try require(session.versions.first?.nativeDrawing == original.nativeDrawing && session.versions.first?.strokes == strokes,
                    "Visibility changes replaced original handwriting")
        try require(session.currentVersion?.strokes.isEmpty == true && session.currentVersion?.parents == [original.id],
                    "Captured erasure lost its original parent")
    }
}
#endif
