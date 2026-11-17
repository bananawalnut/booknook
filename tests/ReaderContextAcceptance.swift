// SPDX-License-Identifier: MPL-2.0
import Foundation

@MainActor private final class ReaderContextDeliveryGate {
    private var continuation: CheckedContinuation<Void, Never>?
    private(set) var entered = false
    func wait() async { entered = true; await withCheckedContinuation { continuation = $0 } }
    func release() { continuation?.resume(); continuation = nil }
}

@MainActor private func awaitReaderContext(_ condition: () -> Bool) async throws {
    for _ in 0..<500 { if condition() { return }; try await Task.sleep(for: .milliseconds(10)) }
    throw AuthorizationError.message("Reader context acceptance timed out")
}

/// A real, bounded two-chapter EPUB imported through the same native asset path
/// as the app. Its contents are synthetic; no account, network or renderer is used.
private func readerContextEPUB(in directory: URL, label: String) throws -> URL {
    let source = directory.appendingPathComponent(label, isDirectory: true)
    try FileManager.default.createDirectory(at: source.appendingPathComponent("META-INF"), withIntermediateDirectories: true)
    let files = [
        "mimetype": "application/epub+zip",
        "META-INF/container.xml": "<container><rootfiles><rootfile full-path=\"package.opf\"/></rootfiles></container>",
        "package.opf": "<package><manifest><item id=\"one\" href=\"one.xhtml\" media-type=\"application/xhtml+xml\"/><item id=\"two\" href=\"two.xhtml\" media-type=\"application/xhtml+xml\"/></manifest><spine><itemref idref=\"one\"/><itemref idref=\"two\"/></spine></package>",
        "one.xhtml": "<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>\(label) first café passage.</p></body></html>",
        "two.xhtml": "<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>\(label) second passage.</p></body></html>"
    ]
    for (path, text) in files { try Data(text.utf8).write(to: source.appendingPathComponent(path)) }
    let archive = directory.appendingPathComponent("\(label).epub")
    let process = Process(); process.executableURL = URL(fileURLWithPath: "/usr/bin/zip")
    process.currentDirectoryURL = source
    process.arguments = ["-q", "-0", "-X", archive.path] + files.keys.sorted()
    try process.run(); process.waitUntilExit()
    guard process.terminationStatus == 0 else { throw CocoaError(.fileWriteUnknown) }
    return archive
}

@MainActor func testReaderContextWorkflow(_ model: DeskModel) async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("booknook-reader-context-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let original = try readerContextEPUB(in: directory, label: "original")
    let replacement = try readerContextEPUB(in: directory, label: "replacement")
    var bookIDs: [String] = []
    for title in ["Reader source A", "Reader source B"] {
        let added = try await model.perform(["op": "add_book", "title": title, "author": "Book Nook QA", "at": 100])
        let id = added!.object!["book_id"] as! String
        model.accept(try await model.repository.importContent(book: id, url: original))
        bookIDs.append(id)
    }
    try await model.selectBook(bookIDs[0])
    var resource = "one.xhtml"
    func context() -> ReaderSourceContext {
        let edition = model.state!.editions[model.book!.editionIds.last!]!, asset = edition.assets.last!
        return ReaderSourceContext(passID: model.notebook!.passId, editionID: edition.id,
            assetID: asset.id, fingerprint: asset.sha256, resource: resource)
    }
    let originalContext = context(), originalCount = model.state!.positions.count
    precondition(model.acceptsReaderContext(originalContext))
    var deliveries = 0, navigationStatus = "Current navigation"
    // The production factory is shared by EPUB and PDF callbacks. This receiver
    // deliberately uses current model state, as ReaderPane.handle does, so any
    // leaked old callback would corrupt the current selection/position/error.
    func receiver() -> ([String: Any]) -> Void {
        model.readerMessages(from: context(), currentResource: { resource }) { message in
            deliveries += 1
            switch message["type"] as? String {
            case "selection": model.selection = ["quote": message["quote"] as! String]
            case "selection_cleared": model.selection = nil
            case "error": model.error = message["message"] as? String
            case "navigation_status": navigationStatus = message["message"] as! String
            case "geometry", "pdf_position":
                model.recordPosition(location: ["kind": "text", "edition_id": model.book!.editionIds.last!,
                    "resource": resource, "quote": message["quote"] as! String, "prefix": "", "suffix": ""])
            default: preconditionFailure("Unexpected test message")
            }
        }
    }
    let originalReceiver = receiver()
    originalReceiver(["type": "selection", "quote": "Current A selection"])
    originalReceiver(["type": "geometry", "quote": "Current A position"])
    try await awaitReaderContext { model.state!.positions.count == originalCount + 1 }
    precondition(model.selection?["quote"] as? String == "Current A selection")
    precondition(model.state!.positions.values.contains { $0.passId == originalContext.passID && $0.location.object?["quote"] as? String == "Current A position" })

    func queueOldMessages(_ callback: @escaping ([String: Any]) -> Void) async throws -> (ReaderContextDeliveryGate, Task<Void, Never>) {
        let gate = ReaderContextDeliveryGate()
        let task = Task { @MainActor in
            await gate.wait()
            // EPUB messages do not carry an asset fingerprint. PDF does, but a
            // matching asset must still not authorize a different reading pass.
            for message: [String: Any] in [
                ["type": "selection", "quote": "Stale selection"],
                ["type": "selection_cleared"],
                ["type": "geometry", "quote": "Stale EPUB position"],
                ["type": "pdf_position", "quote": "Stale PDF position", "assetFingerprint": originalContext.fingerprint],
                ["type": "error", "message": "Stale renderer error"],
                ["type": "navigation_status", "message": "Stale navigation"]
            ] { callback(message) }
        }
        try await awaitReaderContext { gate.entered }
        return (gate, task)
    }
    func releaseAndAssertRejected(_ queued: (ReaderContextDeliveryGate, Task<Void, Never>)) async throws {
        model.selection = ["quote": "Keep current selection"]; model.error = "Keep current error"
        navigationStatus = "Keep current navigation"
        let beforeDeliveries = deliveries, beforeRevision = model.state!.revision
        let beforePositions = Set(model.state!.positions.keys)
        queued.0.release(); await queued.1.value
        precondition(deliveries == beforeDeliveries, "Every stale callback must stop before the receiver")
        precondition(model.selection?["quote"] as? String == "Keep current selection")
        precondition(model.error == "Keep current error" && navigationStatus == "Keep current navigation")
        precondition(model.state!.revision == beforeRevision && Set(model.state!.positions.keys) == beforePositions)
    }

    // Same bytes imported in another book: neither the unchanged fingerprint
    // nor the unchanged chapter permits delivery to the new edition/pass.
    let oldBook = try await queueOldMessages(originalReceiver)
    try await model.selectBook(bookIDs[1])
    precondition(context().fingerprint == originalContext.fingerprint && context().editionID != originalContext.editionID)
    try await releaseAndAssertRejected(oldBook)

    let passBefore = context(), oldPass = try await queueOldMessages(receiver())
    let nextPass = try await model.perform(["op": "start_pass", "book_id": bookIDs[1], "at": 200])
    model.selectedNotebookId = nextPass!.object!["id"] as? String
    precondition(context().passID != passBefore.passID && context().assetID == passBefore.assetID)
    try await releaseAndAssertRejected(oldPass)

    let oldChapter = try await queueOldMessages(receiver())
    resource = "two.xhtml"
    try await releaseAndAssertRejected(oldChapter)

    let beforeReplacement = context(), oldAsset = try await queueOldMessages(receiver())
    model.accept(try await model.repository.importContent(book: bookIDs[1], url: replacement))
    precondition(context().fingerprint != beforeReplacement.fingerprint)
    try await releaseAndAssertRejected(oldAsset)

    let current = context()
    for invalid in [
        ReaderSourceContext(passID: current.passID, editionID: current.editionID, assetID: "wrong-asset", fingerprint: current.fingerprint, resource: resource),
        ReaderSourceContext(passID: current.passID, editionID: current.editionID, assetID: current.assetID, fingerprint: "wrong-fingerprint", resource: resource),
        ReaderSourceContext(passID: current.passID, editionID: "wrong-edition", assetID: current.assetID, fingerprint: current.fingerprint, resource: resource)
    ] { precondition(!model.acceptsReaderContext(invalid)) }
    let currentReceiver = receiver()
    model.error = nil
    currentReceiver(["type": "selection", "quote": "Current B selection"])
    currentReceiver(["type": "geometry", "quote": "Current B position"])
    try await awaitReaderContext { model.state!.positions.count == originalCount + 2 }
    precondition(model.selection?["quote"] as? String == "Current B selection" && model.error == nil)
    let reopened = DeskModel(); await reopened.open()
    precondition(reopened.error == nil && reopened.state!.positions.count == originalCount + 2)
    precondition(reopened.state!.positions.values.contains { $0.passId == current.passID && $0.location.object?["quote"] as? String == "Current B position" && $0.location.object?["resource"] as? String == "two.xhtml" })
    precondition(!reopened.state!.positions.values.contains { ($0.location.object?["quote"] as? String)?.hasPrefix("Stale") == true })
    print("Reader callback context: actual native EPUB import, delayed callbacks rejected after book/pass/chapter/asset switches, selection/error/navigation preserved, valid current callbacks and SQLite position reopen passed. Headless callback boundary; no device or renderer interaction.")
}

/// Exercise the production first-input context and persist it through the actual
/// native Desk. All timestamps, account references and EPUB bytes are synthetic.
@MainActor func testSourceReadingHistory() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("booknook-source-history-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let desk = root.appendingPathComponent("desk")
    let model = DeskModel(repository: DeskRepository(directory: desk)); await model.open()
    precondition(model.error == nil)
    let added = try await model.perform(["op": "add_book", "title": "Source history", "author": "Writer", "at": 100])
    let book = added!.object!["book_id"] as! String
    let earlier = model.state!.passes.values.first { $0.bookId == book }!.id
    let epub = try readerContextEPUB(in: root, label: "position-history")
    model.accept(try await model.repository.importContent(book: book, url: epub))
    let edition = model.state!.books[book]!.editionIds.last!
    let next = try await model.perform(["op": "start_pass", "book_id": book, "at": 200])
    let notebook = next!.object!["id"] as! String
    let pass = model.state!.notebooks[notebook]!.passId

    func observe(_ id: String, _ location: [String: Any], at: UInt64, received: UInt64,
                 connection: String? = nil, passID: String? = nil) async throws {
        var observation: [String: Any] = ["id": id, "pass_id": passID ?? pass, "location": location,
            "observed_at": at, "received_at": received, "alignment": ["status": "unmatched"]]
        if let connection { observation["provider_connection_id"] = connection }
        try await model.perform(["op": "observe_position", "observation": observation])
    }
    func audio(_ time: UInt64) -> [String: Any] {
        ["kind": "audio", "provider": "audible", "source_id": "B012345678", "milliseconds": time]
    }
    try await observe("legacy-before", audio(1000), at: 209, received: 220)
    let initial = try await model.createNote(notebook: notebook)
    try await observe("legacy-new", audio(2000), at: 230, received: 300)
    try await observe("a-tie", audio(4000), at: 250, received: 390, connection: "audible.a")
    try await observe("a-winner", audio(5000), at: 250, received: 260, connection: "audible.a")
    try await observe("b-only", audio(6000), at: 210, received: 220, connection: "audible.b")
    try await observe("other-kind", ["kind": "provider", "provider": "audible", "source_id": "B012345678",
        "value": "opaque location"], at: 230, received: 240, connection: "audible.a")
    try await observe("earlier-page", ["kind": "page", "edition_id": edition, "page": 3], at: 250, received: 260)
    try await observe("current-text", ["kind": "text", "edition_id": edition, "resource": "one.xhtml",
        "quote": "position-history first café passage.", "prefix": "", "suffix": ""], at: 251, received: 260)
    try await observe("different-pass", audio(9100), at: 310, received: 320, connection: "audible.a", passID: earlier)

    let expected = ["legacy-new", "a-winner", "b-only", "other-kind", "current-text"]
    let context = try model.captureReadingContext(passID: pass)
    precondition(context.positionObservations.map(\.id) == expected, "Each connection must preserve its own latest source observation")
    let saved = try await model.createNote(notebook: notebook)
    let reopened = DeskModel(repository: DeskRepository(directory: desk)); await reopened.open()
    precondition(reopened.error == nil)
    let blocks = reopened.state!.notebooks[notebook]!.blocks
    let original = blocks.first { $0.id == initial }!.note!.created.positionObservations
    let observations = blocks.first { $0.id == saved }!.note!.created.positionObservations
    precondition(original.map(\.id) == ["legacy-before"] && original[0].providerConnectionId == nil)
    precondition(observations.map(\.id) == expected)
    precondition(observations.map(\.providerConnectionId) == [nil, "audible.a", "audible.b", "audible.a", nil])
    precondition(observations[1].receivedAt == 260 && observations[2].observedAt == 210)
    print("Source reading history: per-connection audio/provider provenance, source-time tie order, edition and pass isolation, and exact native reopen passed.")
}
