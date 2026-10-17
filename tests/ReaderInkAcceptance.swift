// SPDX-License-Identifier: MPL-2.0
import Foundation
import WebKit

@MainActor private final class ReaderInkGate {
    private var continuation: CheckedContinuation<Void, Never>?
    private(set) var entered = false
    func wait() async { entered = true; await withCheckedContinuation { continuation = $0 } }
    func release() { continuation?.resume(); continuation = nil }
}

@MainActor private func waitForReaderInk(_ condition: () -> Bool) async throws {
    for _ in 0..<500 { if condition() { return }; try await Task.sleep(for: .milliseconds(10)) }
    throw AuthorizationError.message("Reader ink acceptance timed out")
}

@MainActor func testReaderInkWorkflow(_ model: DeskModel) async throws {
    let added = try await model.perform(["op": "add_book", "title": "Reader ink lifecycle", "author": "Book Nook QA", "at": 100])
    let bookID = added!.object!["book_id"] as! String
    try await model.selectBook(bookID)
    let passID = model.notebook!.passId, editionID = model.book!.editionIds[0]
    let context = ReaderInkContext(resource: "original.xhtml", documentURL: "file:///synthetic/original.xhtml",
        contentFingerprint: "content-one", layoutFingerprint: "layout-one")
    let bounds = CGRect(x: 40, y: 50, width: 120, height: 20)
    let originalResolution = ReaderInkResolution(anchor: ["kind": "text", "resource": "original.xhtml", "quote": "The original café passage", "prefix": "", "suffix": ""],
        geometry: ["space": "epub_document", "resource": "original.xhtml", "page": NSNull(), "content_fingerprint": "content-one", "width": 640.0, "height": 800.0,
                   "origin_x": 0.0, "origin_y": 0.0, "text_quote": "The original café passage", "text_prefix": "", "text_suffix": "",
                   "text_rects": [["x": 40.0, "y": 50.0, "width": 120.0, "height": 20.0]]], originalContext: "The original café passage")
    let strokes: [[String: Any]] = [["color": "#234e45", "width": 3.0, "points": [
        ["x": bounds.minX, "y": bounds.minY, "pressure": 0.5, "time": 0.0],
        ["x": bounds.maxX, "y": bounds.minY + 8, "pressure": 0.8, "time": 0.1]]]]
    let bytes = Data("opaque retained drawing bytes".utf8).base64EncodedString()
    let anchorGate = ReaderInkGate(), saveGate = ReaderInkGate()
    var rejectSave = true, requests: [String] = []
    let session = ReaderInkSession(passID: passID, editionID: editionID, assetFingerprint: "asset-one", persist: { command in
        requests.append((command["annotation"] as! [String: Any])["id"] as! String)
        if rejectSave { await saveGate.wait(); throw CocoaError(.fileWriteOutOfSpace) }
        try await model.perform(command)
    })
    let sourceOwner = UUID()
    var registrationRead = false
    session.registerResolver(owner: sourceOwner, resource: context.resource) { _ in
        registrationRead = true
        return originalResolution
    }
    session.capturePendingInk[sourceOwner] = { preconditionFailure("Removed source must no longer be captured") }
    let id = session.capture(context: context, bounds: bounds, strokes: strokes, nativeDrawing: bytes, resolver: { _ in
        await anchorGate.wait()
        return originalResolution
    })
    // Remove the source-view registration BEFORE its delayed anchor resolves.
    // The desk queue, rather than a temporary view callback, owns the capture.
    session.unregister(owner: sourceOwner)
    session.captureAll()
    precondition(session.capturePendingInk[sourceOwner] == nil)
    try await waitForReaderInk { anchorGate.entered }
    precondition(session.drafts[0].nativeDrawing == bytes)
    anchorGate.release(); try await waitForReaderInk { saveGate.entered }
    precondition(session.drafts.count == 1 && model.state!.annotations[id] == nil)
    saveGate.release()
    do { try await session.flush(); preconditionFailure("Failed save must remain pending") }
    catch { precondition(session.pendingCount == 1 && session.error != nil && !session.isSaving) }
    precondition(session.drafts[0].id == id && session.drafts[0].nativeDrawing == bytes)
    rejectSave = false; session.retry(); try await session.flush()
    precondition(!registrationRead, "The retained initial resolver must outlive source registration")
    precondition(requests == [id, id], "Retry must reuse the captured annotation ID")
    let saved = model.state!.annotations[id]!
    precondition(saved.passId == passID && saved.nativeDrawing == bytes)
    precondition(saved.anchor.object?["edition_id"] as? String == editionID)
    precondition(saved.anchor.object?["resource"] as? String == "original.xhtml")
    precondition(saved.originalContext.contains("original café"))

    // A source-changing callback must not persist under the original capture's
    // resource or page. Retain it, remove its temporary resolver, and retry only
    // with a newly available resolver for the original resource.
    let wrong = ReaderInkResolution(anchor: ["kind": "text", "resource": "different.xhtml", "quote": "Wrong chapter", "prefix": "", "suffix": ""],
        geometry: ["space": "epub_document", "resource": "different.xhtml", "content_fingerprint": context.contentFingerprint!], originalContext: "Wrong chapter")
    let retained = model.readerInkSession(passID: passID, editionID: editionID, assetFingerprint: "asset-two")
    precondition(retained === model.readerInkSession(passID: passID, editionID: editionID, assetFingerprint: "asset-two"))
    let unresolvedID = retained.capture(context: context, bounds: bounds, strokes: strokes, nativeDrawing: bytes, resolver: { _ in wrong })
    do { try await retained.flush(); preconditionFailure("Changed chapter must not accept the old ink") }
    catch { precondition(retained.pendingCount == 1 && model.state!.annotations[unresolvedID] == nil && model.saving) }
    let geometry = saved.inkGeometry!.object!, anchor = saved.anchor.object!
    retained.registerResolver(owner: UUID(), resource: context.resource) { _ in
        ReaderInkResolution(anchor: anchor, geometry: geometry, originalContext: saved.originalContext)
    }
    retained.retry(); try await model.drainSaves()
    precondition(!model.saving && model.state!.annotations[unresolvedID]?.nativeDrawing == bytes)

    // If exact placement cannot be recovered, an explicit unplaced save makes
    // the original bytes durable without inventing an EPUB quote or geometry.
    let unmeasured = ReaderInkContext(resource: "images.xhtml", documentURL: "file:///synthetic/images.xhtml", measured: false)
    let unplacedID = retained.capture(context: unmeasured, bounds: bounds, strokes: strokes, nativeDrawing: bytes)
    do { try await retained.flush(); preconditionFailure("Unmeasured coordinates cannot be placed") }
    catch { precondition(retained.drafts[0].canSaveUnplaced) }
    retained.saveAsUnplaced(id: unplacedID); try await model.drainSaves()
    let unplaced = model.state!.annotations[unplacedID]!
    precondition(unplaced.inkGeometry == nil && unplaced.anchor.object?["kind"] as? String == "provider")
    precondition(unplaced.anchor.object?["quote"] == nil && unplaced.nativeDrawing == bytes)
    precondition(ReaderUnplacedInkLocation.matches(unplaced, editionID: editionID))
    precondition(!ReaderUnplacedInkLocation.matches(unplaced, editionID: "other-edition"))
    precondition(ReaderUnplacedInkLocation.resource(unplaced) == "images.xhtml")
    // Imported provider values are untrusted. Wrong fingerprints, oversized or
    // incomplete payloads must never masquerade as this edition's saved marker.
    func changedUnplaced(value: String? = nil, source: String? = nil) -> Annotation {
        var anchor = unplaced.anchor.object!
        if let value { anchor["value"] = value }
        if let source { anchor["source_id"] = source }
        return Annotation(id: unplaced.id, passId: unplaced.passId, anchor: try! JSONDecoder().decode(JSONValue.self, from: JSONSerialization.data(withJSONObject: anchor)),
            originalContext: unplaced.originalContext, layoutFingerprint: unplaced.layoutFingerprint,
            nativeDrawing: unplaced.nativeDrawing, strokes: unplaced.strokes, inkGeometry: nil)
    }
    for invalid in [changedUnplaced(source: "wrong-asset"), changedUnplaced(value: "{}"),
                    changedUnplaced(value: String(repeating: " ", count: 16_385))] {
        precondition(!ReaderUnplacedInkLocation.matches(invalid, editionID: editionID))
        precondition(ReaderUnplacedInkLocation.resource(invalid) == nil)
    }
    // A removed PDF overlay can retain original bytes without fabricating page
    // coordinates when PDFKit could not supply a valid transform.
    let pdfSession = model.readerInkSession(passID: passID, editionID: editionID, assetFingerprint: "pdf-asset")
    let pdfID = pdfSession.capture(context: ReaderInkContext(resource: "book.pdf", page: 2, measured: false),
        bounds: bounds, strokes: strokes, nativeDrawing: bytes)
    do { try await pdfSession.flush(); preconditionFailure("Unmeasured PDF ink cannot acquire geometry") }
    catch { precondition(pdfSession.drafts[0].canSaveUnplaced) }
    pdfSession.saveAsUnplaced(id: pdfID); try await model.drainSaves()
    let pdf = model.state!.annotations[pdfID]!
    precondition(pdf.inkGeometry == nil && pdf.anchor.object?["kind"] as? String == "page")
    precondition((pdf.anchor.object?["page"] as? NSNumber)?.intValue == 2 && pdf.nativeDrawing == bytes)
    let exported = FileManager.default.temporaryDirectory.appendingPathComponent("reader-ink-\(UUID().uuidString).json")
    defer { try? FileManager.default.removeItem(at: exported) }
    try await model.repository.export(to: exported)
    let archive = try JSONSerialization.jsonObject(with: Data(contentsOf: exported)) as! [String: Any]
    let portable = (archive["state"] as! [String: Any])["annotations"] as! [String: [String: Any]]
    precondition(portable[unplacedID]?["native_drawing"] as? String == bytes)
    precondition(portable[pdfID]?["native_drawing"] as? String == bytes)
    let reopened = DeskModel(); await reopened.open()
    precondition(reopened.state!.annotations[id]?.nativeDrawing == bytes)
    precondition(reopened.state!.annotations[unresolvedID]?.anchor.object?["resource"] as? String == "original.xhtml")
    precondition(reopened.state!.annotations[unplacedID]?.nativeDrawing == bytes)
    precondition(reopened.state!.annotations[pdfID]?.nativeDrawing == bytes)
    print("Reader ink lifecycle: delayed anchor after source deregistration, failed save retention/retry with stable ID, source drift rejection, retained desk session/drain, explicit unplaced save, exact native bytes/pass/edition/resource and actual SQLite reopen/export passed. Synthetic anchor boundary; no physical Pencil input or UI interaction.")
}
