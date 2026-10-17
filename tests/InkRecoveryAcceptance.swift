// SPDX-License-Identifier: MPL-2.0
import Foundation

@MainActor private final class InkRecoveryGate {
    var entered = false
    var continuation: CheckedContinuation<Void, Never>?
    func wait() async { entered = true; await withCheckedContinuation { continuation = $0 } }
    func release() { continuation?.resume(); continuation = nil }
}
@MainActor private func awaitInkRecovery(_ ready: () -> Bool) async throws {
    for _ in 0..<1000 { if ready() { return }; try await Task.sleep(for: .milliseconds(10)) }
    throw InkJournalFailure("Ink recovery acceptance timed out")
}

@MainActor func testInkJournalAtomicRules() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("ink-journal-rules-" + UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: root) }
    let journal = InkJournal(deskDirectory: root)
    let record = try InkJournalRecord(id: UUID().uuidString, kind: "reader", payload: ["synthetic": "original bytes"])
    await journal.simulateInterruptedDirectoryBarrier()
    do { try await journal.store(record); preconditionFailure("Synthetic interrupted directory barrier was acknowledged") } catch { }
    // Retry sees the published file, but must still execute the missing barrier.
    await journal.simulateInterruptedDirectoryBarrier()
    do { try await journal.store(record); preconditionFailure("Existing record skipped the directory durability barrier") } catch { }
    try await journal.store(record)
    let changed = try InkJournalRecord(id: record.id, kind: "reader", payload: ["synthetic": "different bytes"])
    do { try await journal.store(changed); preconditionFailure("Conflicting capture overwritten") } catch { }
    do { try await journal.remove(changed); preconditionFailure("Stale completion removed another capture") } catch { }
    let complete = try InkJournalRecord(id: UUID().uuidString, kind: "reader", payload: ["synthetic": "complete temporary capture"])
    let directory = journal.directory
    let temporary = directory.appendingPathComponent("." + UUID().uuidString + ".tmp")
    try complete.encoded().write(to: temporary)
    let partial = directory.appendingPathComponent("." + UUID().uuidString + ".tmp")
    try Data("{partial original".utf8).write(to: partial)
    let (loaded, errors) = try await journal.load()
    precondition(Set(loaded.map(\.id)) == [record.id, complete.id] && errors.count == 1)
    precondition(!FileManager.default.fileExists(atPath: temporary.path) && FileManager.default.fileExists(atPath: partial.path))
    let partialBytes = try Data(contentsOf: partial)
    precondition(partialBytes == Data("{partial original".utf8))
    let invalid = try InkJournalRecord(id: UUID().uuidString, kind: "reader", payload: [
        "pass_id": "p", "edition_id": "e", "asset_fingerprint": "a", "context": ["resource": "original.xhtml", "measured": false],
        "bounds": [false, 0, 10, 10], "strokes": [], "native_drawing": "AQID"])
    try await journal.store(invalid)
    let session = ReaderInkSession(passID: "p", editionID: "e", assetFingerprint: "a", persist: { _ in })
    do { try session.recover(invalid); preconditionFailure("Boolean rectangle coordinate was accepted as a number") } catch { }
    precondition(session.drafts.isEmpty)
    let (retainedInvalid, _) = try await journal.load(); precondition(retainedInvalid.contains { $0.id == invalid.id })
    try await journal.remove(record); try await journal.remove(complete)
    print("Ink journal atomic rules: interrupted directory barrier retried, immutable ID collision and stale acknowledgement rejected, complete temporary record promoted, partial bytes retained with visible failure.")
}

@MainActor func testTranscriptionCancellation(_ model: DeskModel) async throws {
    let added = try await model.perform(["op": "add_book", "title": "Cancellable recognition", "author": "Synthetic QA", "at": 10])
    try await model.selectBook(added!.object!["book_id"] as! String)
    let notebook = model.notebook!, block = notebook.blocks[0], session = model.inkSession(notebook: notebook, block: block)
    precondition(session.save(canvasWidth: 768, canvasHeight: 960,
        strokes: [["color": "#222222", "width": 2.0, "points": [["x": 10.0, "y": 20.0, "pressure": 0.5, "time": 0.0]]]], nativeDrawing: "AQID"))
    try await session.flush(); let original = session.currentVersion!
    let first = InkRecoveryGate(), second = InkRecoveryGate()
    var requests = 0
    let draft = NotebookTranscriptionDraft { source in
        precondition(source.id == original.id)
        requests += 1
        if requests == 1 { await first.wait(); return "Late cancelled result" }
        await second.wait(); return "Current recognition café 日本語"
    }
    draft.recognize(original); try await awaitInkRecovery { first.entered }
    draft.cancelRecognition(); precondition(!draft.isRecognizing)
    draft.text = "My manual review survives cancellation"
    first.release(); try await Task.sleep(for: .milliseconds(20))
    precondition(draft.text == "My manual review survives cancellation" && draft.error == nil)
    // A stale completion also cannot end a newer recognition request.
    draft.text = ""; draft.recognize(original); try await awaitInkRecovery { second.entered }
    precondition(draft.isRecognizing); second.release()
    try await awaitInkRecovery { !draft.isRecognizing }
    precondition(draft.text == "Current recognition café 日本語")
    let failed = await draft.save { _, _ in throw CocoaError(.fileWriteOutOfSpace) }
    precondition(!failed && !draft.isSaving && draft.error != nil && draft.text == "Current recognition café 日本語")
    draft.text = "Reviewed café 日本語"
    let saved = await draft.save { text, method in
        try await model.saveInkTranscription(notebook: notebook.id, block: block.id, version: original.id, text: text, method: method)
    }
    precondition(saved)
    let stored = model.state!.notebooks[notebook.id]!.blocks[0]
    precondition(stored.ink!.versions[0].nativeDrawing == original.nativeDrawing && stored.ink!.versions[0].strokes == original.strokes)
    precondition(stored.text == block.text && stored.ink!.transcriptions.last!.inkVersionId == original.id)
    precondition(stored.ink!.transcriptions.last!.text == "Reviewed café 日本語")
    print("Transcription controller: explicit cancel, ignored late result, manual draft retention, independent subsequent recognition, failed-save review retention and exact-source SQLite transcript passed.")
}

/// Called only by the delivered parent process runner with a fresh explicit desk.
/// Parent SIGKILLs this process after ack.json proves every original is fsynced.
@MainActor func runInkJournalCrashChild(directory: URL) async throws {
    let repository = DeskRepository(directory: directory), model = DeskModel(repository: repository)
    await model.open(); precondition(model.error == nil && model.books.isEmpty)
    let result = try await model.perform(["op": "add_book", "title": "Private ink crash fixture", "author": "Synthetic QA", "at": 100])
    try await model.selectBook(result!.object!["book_id"] as! String)
    let notebook = model.notebook!, block = notebook.blocks[0], edition = model.book!.editionIds[0]
    let journal = try await repository.inkJournal()
    let strokes: [[String: Any]] = [["color": "#234e45", "width": 3.0,
        "points": [["x": 40.0, "y": 50.0, "pressure": 0.5, "time": 0.0], ["x": 120.0, "y": 90.0, "pressure": 0.8, "time": 0.1]]]]
    let notebookInk = NotebookInkSession(notebookID: notebook.id, blockID: block.id, passID: notebook.passId,
        reader: model.state!.localReader, ink: nil, persist: { _ in throw CocoaError(.fileWriteOutOfSpace) }, journal: journal)
    precondition(notebookInk.save(canvasWidth: 768, canvasHeight: 960, strokes: strokes, nativeDrawing: "AQID"))
    let firstID = notebookInk.currentVersion!.id
    precondition(notebookInk.save(canvasWidth: 768, canvasHeight: 960, strokes: [], nativeDrawing: "BAUG"))
    let erasedID = notebookInk.currentVersion!.id
    let anchorGate = InkRecoveryGate()
    let reader = ReaderInkSession(passID: notebook.passId, editionID: edition, assetFingerprint: "fixture-asset",
        persist: { try await model.perform($0) }, journal: journal)
    let context = ReaderInkContext(resource: "original.xhtml", documentURL: "file:///synthetic/original.xhtml",
        contentFingerprint: "fixture-content", layoutFingerprint: "fixture-layout")
    let readerID = reader.capture(context: context, bounds: CGRect(x: 40, y: 50, width: 80, height: 40),
        strokes: strokes, nativeDrawing: "BwgJ", resolver: { _ in await anchorGate.wait(); throw CocoaError(.fileReadUnknown) })
    // A second original must journal even while the first resolver never returns.
    let laterID = reader.capture(context: context, bounds: CGRect(x: 40, y: 50, width: 80, height: 40), strokes: strokes, nativeDrawing: "CgsM")
    try await notebookInk.checkpoint(); try await reader.checkpoint()
    try await awaitInkRecovery { anchorGate.entered && notebookInk.saveError != nil }
    let (records, errors) = try await journal.load()
    precondition(records.count == 4 && errors.isEmpty && notebookInk.allPendingJournaled && reader.allPendingJournaled)
    let ack: [String: Any] = ["notebook": notebook.id, "block": block.id, "pass": notebook.passId, "edition": edition,
        "book": notebook.bookId, "first": firstID, "erased": erasedID, "reader": readerID, "later": laterID,
        "record_ids": records.map(\.id), "first_at": notebookInk.pending[0].at, "erased_at": notebookInk.pending[1].at,
        "journal_acknowledged": true, "anchor_waiting": true, "native_save_failed": true]
    try JSONSerialization.data(withJSONObject: ack, options: [.prettyPrinted, .sortedKeys]).write(to: directory.appendingPathComponent("ack.json"), options: .atomic)
    while true { try await Task.sleep(for: .seconds(30)) }
}

@MainActor func verifyInkJournalCrashRecovery(directory: URL) async throws {
    try await testInkJournalAtomicRules()
    let ack = try JSONSerialization.jsonObject(with: Data(contentsOf: directory.appendingPathComponent("ack.json"))) as! [String: Any]
    let notebookID = ack["notebook"] as! String, blockID = ack["block"] as! String
    let repository = DeskRepository(directory: directory), model = DeskModel(repository: repository)
    await model.open(); if let error = model.error { throw InkJournalFailure(error) }
    let session = model.inkSession(notebook: model.state!.notebooks[notebookID]!, block: model.state!.notebooks[notebookID]!.blocks.first { $0.id == blockID }!)
    try await session.flush()
    let versions = model.state!.notebooks[notebookID]!.blocks.first { $0.id == blockID }!.ink!.versions
    precondition(versions.map(\.id) == [ack["first"] as! String, ack["erased"] as! String])
    precondition(versions[0].at == (ack["first_at"] as! NSNumber).uint64Value && versions[1].at == (ack["erased_at"] as! NSNumber).uint64Value)
    precondition(versions[0].nativeDrawing == "AQID" && versions[1].nativeDrawing == "BAUG" && versions[1].parents == [versions[0].id])
    precondition(versions[0].strokes.count == 1 && versions[1].strokes.isEmpty)
    let recovered = model.readerInkSession(passID: ack["pass"] as! String, editionID: ack["edition"] as! String, assetFingerprint: "fixture-asset")
    precondition(recovered.pendingCount == 2 && recovered.allPendingJournaled)
    precondition(recovered.drafts.map(\.nativeDrawing) == ["BwgJ", "CgsM"])
    precondition(recovered.drafts.allSatisfy { $0.context.resource == "original.xhtml" && $0.context.layoutFingerprint == "fixture-layout" && $0.canSaveUnplaced })
    // Navigate while the original is safely journaled, then explicitly finalize.
    try await model.selectBook(ack["book"] as! String)
    for id in recovered.drafts.map(\.id) {
        recovered.saveAsUnplaced(id: id)
        try await awaitInkRecovery { !recovered.isSaving }
    }
    try await model.drainSaves()
    for (key, bytes) in [("reader", "BwgJ"), ("later", "CgsM")] {
        let value = model.state!.annotations[ack[key] as! String]!
        precondition(value.nativeDrawing == bytes && value.passId == ack["pass"] as? String && value.inkGeometry == nil)
        precondition(ReaderUnplacedInkLocation.matches(value, editionID: ack["edition"] as! String))
    }
    let journal = try await repository.inkJournal(), (remaining, errors) = try await journal.load()
    precondition(remaining.isEmpty && errors.isEmpty)
    let export = directory.appendingPathComponent("recovered-export.json"); try await repository.export(to: export)
    let reopened = DeskModel(repository: DeskRepository(directory: directory)); await reopened.open()
    precondition(reopened.error == nil && reopened.state!.annotations[ack["reader"] as! String]!.nativeDrawing == "BwgJ")
    precondition(reopened.state!.notebooks[notebookID]!.blocks[0].ink!.versions.count == 2)
    print("Actual SIGKILL recovery: four fsynced captures, failed notebook save + erase chain, blocked reader anchor + later capture, exact IDs/parents/native bytes/context, explicit unplaced finalization, empty journal, SQLite reopen and portable export passed.")
}
