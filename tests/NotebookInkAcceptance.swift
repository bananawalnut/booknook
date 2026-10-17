// SPDX-License-Identifier: MPL-2.0
import Foundation

@MainActor func testNotebookInkWorkflow(_ model: DeskModel) async throws {
    try await testNotebookCaptureContextJournal()
    try await testNoteFacesAndOrigins(model)
    let strokes: [[String: Any]] = [["color": "#234e45", "width": 3.0,
        "points": [["x": 40.0, "y": 50.0, "pressure": 0.5, "time": 0.0],
                   ["x": 120.0, "y": 90.0, "pressure": 0.8, "time": 0.1]]]]
    // A real asynchronous failure must retain BOTH drawings and their exact IDs,
    // including a second drawing made while the first is being persisted.
    var reject = true, requests: [[String: Any]] = []
    let session = NotebookInkSession(notebookID: "n", blockID: "b", passID: "p", reader: "r", ink: nil, persist: { command in
        requests.append(command)
        try await Task.sleep(for: .milliseconds(5))
        if reject { throw CocoaError(.fileWriteOutOfSpace) }
    })
    precondition(session.save(canvasWidth: 768, canvasHeight: 960, strokes: strokes, nativeDrawing: "AQID"))
    let originalID = session.currentVersion!.id
    precondition(session.save(canvasWidth: 768, canvasHeight: 960, strokes: [], nativeDrawing: "BAUG"))
    let erasedID = session.currentVersion!.id
    precondition(session.currentVersion!.parents == [originalID] && session.pendingCount == 2)
    do { try await session.flush(); preconditionFailure("Injected persistence failure was hidden") }
    catch { precondition(session.pendingCount == 2 && session.isSaving && session.saveError != nil) }
    reject = false; session.retry(); try await session.flush()
    precondition(requests.count == 3)
    precondition(requests[0]["version_id"] as? String == requests[1]["version_id"] as? String)
    precondition(session.currentVersion!.id == erasedID && session.versions.count == 2 && !session.isSaving)
    precondition(session.versions[0].nativeDrawing == "AQID", "Erasure cannot discard original drawing bytes")

    // An incoming branch during an eraser gesture must not turn that deliberate
    // empty snapshot into the blank-page no-op. Parent A is what was displayed.
    let a = session.versions[0]
    let b = NotebookInkVersion(id: "incoming-branch", parents: [], reader: "remote", at: a.at,
        passId: a.passId, positions: [], positionObservations: nil,
        canvasWidth: a.canvasWidth, canvasHeight: a.canvasHeight, strokes: a.strokes, nativeDrawing: a.nativeDrawing)
    var erasedCommands: [[String: Any]] = []
    let branched = NotebookInkSession(notebookID: "n", blockID: "b", passID: "p", reader: "r",
        ink: NotebookInk(schema: "booknook.notebook-ink.v1", versions: [a], transcriptions: []),
        persist: { erasedCommands.append($0) })
    precondition(branched.currentVersion?.id == a.id)
    branched.accept(NotebookInk(schema: "booknook.notebook-ink.v1", versions: [a, b], transcriptions: []))
    precondition(branched.needsVersionChoice && branched.currentVersion == nil)
    precondition(branched.save(canvasWidth: 768, canvasHeight: 960, strokes: [], nativeDrawing: "BAUG", parents: [a.id]))
    try await branched.flush()
    precondition(erasedCommands.count == 1 && branched.versions.count == 3)
    precondition(branched.currentVersion!.parents == [a.id] && branched.currentVersion!.strokes.isEmpty)
    precondition(branched.heads.count == 2, "The unseen incoming branch must remain unresolved")

    let sameDrawing = NotebookInkSession(notebookID: "n", blockID: "b", passID: "p", reader: "r",
        ink: NotebookInk(schema: "booknook.notebook-ink.v1", versions: [a], transcriptions: []), persist: { _ in })
    precondition(sameDrawing.save(canvasWidth: a.canvasWidth, canvasHeight: a.canvasHeight,
        strokes: a.strokes.map(\.dictionary), nativeDrawing: a.nativeDrawing, parents: []))
    try await sameDrawing.flush()
    precondition(sameDrawing.versions.count == 2 && sameDrawing.heads.count == 2,
        "An explicit different parent must not be silently deduplicated into an unseen head")

    let result = try await model.perform(["op": "add_book", "title": "Handwriting acceptance", "author": "Book Nook QA", "at": 100])
    let bookID = result!.object!["book_id"] as! String
    try await model.selectBook(bookID)
    let notebook = model.notebook!, block = notebook.blocks[0]
    model.saveText(notebook: notebook.id, block: block.id, session: "handwriting-test", baseUpdate: block.update, text: "Typed notes stay here: café 日本語", styles: [])
    try await model.drainSaves()
    let ink = model.inkSession(notebook: model.notebook!, block: model.notebook!.blocks[0])
    precondition(ink === model.inkSession(notebook: model.notebook!, block: model.notebook!.blocks[0]), "A lazy row remount must reuse its pending session")
    precondition(ink.save(canvasWidth: 768, canvasHeight: 960, strokes: strokes, nativeDrawing: "AQID"))
    let inkID = ink.currentVersion!.id
    // drainSaves must include an in-progress canvas capture, then await ink writes.
    var captured = false
    ink.capturePendingStroke = { if !captured { captured = true; _ = ink.save(canvasWidth: 768, canvasHeight: 960, strokes: [], nativeDrawing: "BAUG") } }
    try await model.drainSaves(); ink.capturePendingStroke = nil
    precondition(captured && !model.saving && ink.versions.count == 2)
    try await model.saveInkTranscription(notebook: notebook.id, block: block.id, version: inkID, text: "Reviewed reading: café 日本語", method: "on_device")
    let after = model.state!.notebooks[notebook.id]!.blocks[0]
    precondition(after.text == "Typed notes stay here: café 日本語")
    precondition(after.ink!.versions.count == 2 && after.ink!.versions[0].nativeDrawing == "AQID")
    precondition(after.ink!.transcriptions.count == 1 && after.ink!.transcriptions[0].inkVersionId == inkID)
    precondition(after.ink!.transcriptions[0].text == "Reviewed reading: café 日本語")
    let image = try NotebookInkRecognizer.raster(after.ink!.versions[0])
    precondition(image.width > 0 && image.width <= 2048 && image.height > 0 && image.height <= 2048)
    do { _ = try NotebookInkRecognizer.raster(after.ink!.versions[1]); preconditionFailure("Empty page should not be recognized as text") }
    catch { precondition(bookNookErrorMessage(error).contains("Write something")) }

    let reopened = DeskModel(); await reopened.open()
    let recovered = reopened.state!.notebooks[notebook.id]!.blocks[0]
    precondition(recovered.ink!.versions.map(\.id) == after.ink!.versions.map(\.id))
    precondition(recovered.ink!.versions[0].strokes == after.ink!.versions[0].strokes)
    precondition(recovered.ink!.transcriptions[0].text == after.ink!.transcriptions[0].text)
    let exported = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".json")
    defer { try? FileManager.default.removeItem(at: exported) }
    try await model.repository.export(to: exported)
    let archive = try JSONSerialization.jsonObject(with: Data(contentsOf: exported)) as! [String: Any]
    precondition((archive["state"] as? [String: Any])?["schema"] as? String == "booknook.archive.v3")
    let restoredDirectory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: restoredDirectory) }
    let restoreDesk = try Desk(directory: restoredDirectory.path)
    let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase
    let restored = try decoder.decode(DeskSnapshot.self, from: Data(try restoreDesk.restoreArchive(path: exported.path).utf8))
    let restoredBlock = restored.notebooks[notebook.id]!.blocks[0]
    precondition(restoredBlock.ink!.versions.map(\.id) == after.ink!.versions.map(\.id))
    precondition(restoredBlock.text == after.text && restoredBlock.ink!.transcriptions[0].inkVersionId == inkID)
    print("Notebook ink: failed async save/retry, ordered snapshots, erasure history, stable row session, drain capture, separate reviewed transcript, bounded OCR raster, durable reopen and portable v3 export/reimport passed. Synthetic strokes; actual Pencil and handwriting recognition not qualified.")
}

/// A real journal round trip must replay capture-time context, even if the same
/// source observation ID now points to a later page and playback position.
@MainActor func testNotebookCaptureContextJournal() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("note-context-" + UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let journal = InkJournal(deskDirectory: directory)
    func context(page: Int, milliseconds: Int) throws -> NotebookCaptureContext {
        let observations: [[String: Any]] = [
            ["id": "page", "pass_id": "p", "location": ["kind": "page", "edition_id": "edition", "page": page],
             "observed_at": 100, "received_at": 101, "alignment": ["status": "unmatched"]],
            ["id": "audio", "pass_id": "p", "location": ["kind": "audio", "provider": "audible", "source_id": "source", "milliseconds": milliseconds, "chapter": "Chapter one"],
             "observed_at": 95, "received_at": 101, "alignment": ["status": "unmatched"],
             "audio_metadata": ["title": "An original synthetic title", "authors": ["QA"], "ebook_identifiers": [], "language": "en"]]
        ]
        let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase
        let values = try decoder.decode([PositionObservation].self, from: JSONSerialization.data(withJSONObject: observations))
        return NotebookCaptureContext(passId: "p", positions: values.map(\.location), positionObservations: values)
    }
    var available = try context(page: 3, milliseconds: 7000)
    let early = try available.dictionary
    var failedCommands: [[String: Any]] = []
    var firstOrigin: NotebookNoteCreation?
    let session = NotebookInkSession(notebookID: "n", blockID: "b", passID: "p", reader: "r", ink: nil,
        persist: { failedCommands.append($0); throw CocoaError(.fileWriteOutOfSpace) }, journal: journal,
        captureContext: { available }, noteCreation: { capture in
            if firstOrigin == nil { firstOrigin = NotebookNoteCreation(at: capture.at, captureContext: capture.context) }
            return firstOrigin
        })
    let began = session.beginCapture()
    available = try context(page: 12, milliseconds: 34000)
    let space = NotebookCanvasSpace(kind: "whiteboard", originX: -200, originY: -100)
    precondition(session.save(canvasWidth: 1200, canvasHeight: 1600,
        strokes: [["color": "#123456", "width": 2.0, "points": [["x": -20.0, "y": -10.0, "pressure": 0.5, "time": 0.0]]]],
        nativeDrawing: "AQID", canvasSpace: space, capturedContext: began))
    let firstID = session.currentVersion!.id
    precondition(session.save(canvasWidth: 1200, canvasHeight: 1600, strokes: [], nativeDrawing: "BAUG", canvasSpace: space))
    let erasedID = session.currentVersion!.id
    try await session.checkpoint()
    do { try await session.flush(); preconditionFailure("Failed write was hidden") } catch { }
    let (records, errors) = try await journal.load()
    precondition(errors.isEmpty && records.count == 2)
    var replayed: [[String: Any]] = []
    let reopened = NotebookInkSession(notebookID: "n", blockID: "b", passID: "p", reader: "r", ink: nil,
        persist: { replayed.append($0) }, journal: journal,
        captureContext: { preconditionFailure("Recovery requested the current reading position") })
    for record in records { try reopened.recover(record) }
    try await reopened.flush()
    precondition(replayed.count == 2)
    precondition(replayed[0]["version_id"] as? String == firstID && replayed[1]["version_id"] as? String == erasedID)
    precondition(NSDictionary(dictionary: replayed[0]["capture_context"] as! [String: Any]).isEqual(to: early))
    let late = try available.dictionary
    precondition(NSDictionary(dictionary: replayed[1]["capture_context"] as! [String: Any]).isEqual(to: late))
    precondition(NSDictionary(dictionary: failedCommands[0]).isEqual(to: replayed[0]), "Retry changed immutable capture bytes or timestamp")
    precondition(NSDictionary(dictionary: replayed[0]["note_creation"] as! [String: Any]).isEqual(to: replayed[1]["note_creation"] as! [String: Any]))
    precondition(reopened.versions[0].canvasSpace == space && reopened.versions[0].nativeDrawing == "AQID")
    precondition(reopened.versions[1].strokes.isEmpty && reopened.versions[1].parents == [firstID])
    let (remaining, _) = try await journal.load(); precondition(remaining.isEmpty)
    // A pre-context journal snapshot is recovered as explicit unavailable context,
    // never retroactively assigned the current source position.
    var legacy = records[0].payload.object!
    var legacyVersion = legacy["version"] as! [String: Any]
    let legacyID = UUID().uuidString
    legacyVersion["id"] = legacyID; legacyVersion["positions"] = []
    legacyVersion.removeValue(forKey: "position_observations"); legacyVersion.removeValue(forKey: "canvas_space")
    legacy["version"] = legacyVersion
    let old = try InkJournalRecord(id: legacyID, kind: "notebook", payload: legacy)
    let historical = NotebookInkSession(notebookID: "n", blockID: "b", passID: "p", reader: "r", ink: nil,
        persist: { command in
            let context = command["capture_context"] as! [String: Any]
            precondition((context["positions"] as! [Any]).isEmpty && (context["position_observations"] as! [Any]).isEmpty)
        }, captureContext: { preconditionFailure("Old journal adopted a later reading position") })
    try historical.recover(old); try await historical.flush()
    print("Notebook capture context: immutable page/audio observations and audio metadata, signed whiteboard origin, erasure parents and native bytes survive failed write, exact-ID retry and private-journal reopen; old empty context remains unavailable.")
}

@MainActor func testNoteFacesAndOrigins(_ model: DeskModel) async throws {
    let result = try await model.perform(["op": "add_book", "title": "A note in context", "author": "Synthetic QA", "at": 100])
    let bookID = result!.object!["book_id"] as! String
    try await model.selectBook(bookID)
    let notebook = model.notebook!, blank = notebook.blocks[0]
    let edition = model.state!.books[bookID]!.editionIds[0]
    precondition(blank.note == nil && blank.face == "text")
    try await model.setNoteFace(notebook: notebook.id, block: blank.id, face: "whiteboard")
    let flippedBlank = model.state!.notebooks[notebook.id]!.blocks[0]
    precondition(flippedBlank.face == "whiteboard" && flippedBlank.ink == nil && flippedBlank.text.isEmpty)
    precondition(flippedBlank.note?.created.source == "pending" && flippedBlank.note?.created.at == nil,
        "Looking at an empty whiteboard must not pretend a note was written")
    try await model.setNoteFace(notebook: notebook.id, block: blank.id, face: "text")
    let now = UInt64(Date().timeIntervalSince1970)
    try await model.perform(["op": "observe_position", "observation": ["id": "note-audio", "pass_id": notebook.passId,
        "location": ["kind": "audio", "provider": "audible", "source_id": "synthetic", "milliseconds": 7000, "chapter": "One"],
        "observed_at": now - 5, "received_at": now, "alignment": ["status": "unmatched"],
        "audio_metadata": ["title": "A note in context", "authors": ["Synthetic QA"], "language": "en"]]])
    model.recordPosition(location: ["kind": "page", "edition_id": edition, "page": 3])
    model.saveText(notebook: notebook.id, block: blank.id, session: "context-first-input", baseUpdate: blank.update, text: "First café", styles: [])
    // Neither position acknowledgement nor text coalescing has run between these
    // synchronous callbacks. The second text must retain the first input origin.
    model.recordPosition(location: ["kind": "page", "edition_id": edition, "page": 12])
    model.saveText(notebook: notebook.id, block: blank.id, session: "context-first-input", baseUpdate: blank.update, text: "First café 日本語, continued", styles: [])
    try await model.drainSaves()
    let written = model.state!.notebooks[notebook.id]!.blocks[0]
    let origin = written.note!.created
    precondition(origin.source == "recorded" && origin.at != nil)
    precondition(origin.positions.contains { ($0.object?["page"] as? NSNumber)?.intValue == 3 })
    precondition(!origin.positions.contains { ($0.object?["page"] as? NSNumber)?.intValue == 12 })
    precondition(origin.positions.contains { ($0.object?["milliseconds"] as? NSNumber)?.intValue == 7000 })
    precondition(origin.positionObservations.first { $0.id == "note-audio" }?.audioMetadata?.object?["title"] as? String == "A note in context")
    let ink = model.inkSession(notebook: model.state!.notebooks[notebook.id]!, block: written)
    let captured = ink.beginCapture()
    model.recordPosition(location: ["kind": "page", "edition_id": edition, "page": 20])
    let space = NotebookCanvasSpace(kind: "whiteboard", originX: -400, originY: -300)
    precondition(ink.save(canvasWidth: 2400, canvasHeight: 1800,
        strokes: [["color": "#654321", "width": 3.0, "points": [["x": -100.0, "y": -90.0, "pressure": 0.5, "time": 0.0]]]],
        nativeDrawing: "AQID", canvasSpace: space, capturedContext: captured))
    try await model.drainSaves()
    let originalDrawing = ink.currentVersion!
    precondition(originalDrawing.positions.contains { ($0.object?["page"] as? NSNumber)?.intValue == 12 })
    precondition(!originalDrawing.positions.contains { ($0.object?["page"] as? NSNumber)?.intValue == 20 })
    let ids = model.state!.notebooks[notebook.id]!.blocks.map(\.id)
    try await model.setNoteFace(notebook: notebook.id, block: blank.id, face: "whiteboard")
    try await model.setNoteFace(notebook: notebook.id, block: blank.id, face: "text")
    let after = model.state!.notebooks[notebook.id]!.blocks[0]
    precondition(after.text == written.text && after.ink!.versions[0].nativeDrawing == "AQID")
    precondition(after.ink!.versions[0].canvasSpace == space && after.note!.created.at == origin.at)
    precondition(model.state!.notebooks[notebook.id]!.blocks.map(\.id) == ids, "Flipping a note reordered the notebook")
    let added = try await model.createNote(notebook: notebook.id, face: "whiteboard")
    let explicit = model.state!.notebooks[notebook.id]!.blocks.last!
    precondition(explicit.id == added && explicit.face == "whiteboard" && explicit.ink == nil && explicit.text.isEmpty)
    precondition(explicit.note!.created.source == "recorded" && explicit.note!.created.at != nil)
    let reopened = DeskModel(); await reopened.open()
    let restored = reopened.state!.notebooks[notebook.id]!.blocks
    precondition(restored.map(\.id) == ids + [added] && restored[0].face == "text" && restored.last!.face == "whiteboard")
    precondition(restored[0].ink!.versions[0].canvasSpace == space && restored[0].ink!.versions[0].nativeDrawing == "AQID")
    precondition(restored[0].note!.created.positions.contains { ($0.object?["page"] as? NSNumber)?.intValue == 3 })
    print("Note model: empty flips stay pending, first Unicode input origin survives coalescing/page changes, independent audiobook context preserved, tool-start drawing context immutable, world-origin ink/prose/order retained through flip and reopen, explicit new whiteboard creation recorded.")
}
