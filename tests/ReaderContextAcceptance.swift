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
