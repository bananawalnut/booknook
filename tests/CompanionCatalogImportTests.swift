// SPDX-License-Identifier: MPL-2.0
import Foundation

@MainActor private final class CatalogGate {
    private var parked: CheckedContinuation<Void, Never>?
    private var observer: CheckedContinuation<Void, Never>?
    func suspend() async {
        await withCheckedContinuation { continuation in
            parked = continuation; observer?.resume(); observer = nil
        }
    }
    func waitUntilSuspended() async {
        if parked != nil { return }
        await withCheckedContinuation { observer = $0 }
    }
    func release() { parked?.resume(); parked = nil }
}

@MainActor private final class CatalogSourceProbe: CompanionCatalogSource {
    var calls: [String] = []
    let catalog: Data
    var format = CompanionKindleImport.Format.jsonCatalog
    var uploadFailure: Error?
    var downloadFailure: Error?
    var refreshFailure: Error?
    var downloadGate: CatalogGate?
    var refreshGate: CatalogGate?
    init(_ asin: String, title: String = "Synthetic imported book") throws {
        catalog = try JSONSerialization.data(withJSONObject: ["schema": "booknook.kindle-catalog.v1",
            "items": [["asin": asin, "title": title, "authors": ["Synthetic writer"]]]])
    }
    func importKindleExport(_ bytes: Data) async throws -> CompanionKindleImport.Format {
        calls.append("upload")
        precondition(!bytes.isEmpty)
        if let uploadFailure { throw uploadFailure }
        return format
    }
    func kindleCatalog() async throws -> Data {
        calls.append("catalog")
        // Deliberately cancellation-ignoring: the workflow must check before commit.
        if let downloadGate { await downloadGate.suspend() }
        if let downloadFailure { throw downloadFailure }
        return catalog
    }
    func manifest() async throws -> CompanionManifest {
        calls.append("manifest")
        if let refreshGate { await refreshGate.suspend() }
        if let refreshFailure { throw refreshFailure }
        return CompanionManifest(schema: "booknook.companion.v1", name: "Synthetic source",
            capabilities: ["kindle.catalog.import", "kindle.catalog"], qualified: true)
    }
}

@MainActor func testCompanionCatalogImportWorkflow() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("booknook-catalog-flow-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    let previous = ProcessInfo.processInfo.environment["BOOKNOOK_DATA_DIR"]
    defer {
        if let previous { setenv("BOOKNOOK_DATA_DIR", previous, 1) } else { unsetenv("BOOKNOOK_DATA_DIR") }
        try? FileManager.default.removeItem(at: root)
    }
    setenv("BOOKNOOK_DATA_DIR", root.path, 1)
    let model = DeskModel(); await model.open()
    precondition(model.error == nil && model.books.isEmpty)
    let file = root.appendingPathComponent("selected-catalog.json")
    try Data(#"{"schema":"booknook.kindle-catalog.v1","items":[]}"#.utf8).write(to: file)
    let workflow = CompanionCatalogImportWorkflow()
    let first = try CatalogSourceProbe("B000000001", title: "First imported title")
    var committedMessages: [String] = []
    let complete = try await workflow.run(file: file, source: first, model: model, didImport: { committedMessages.append($0) })
    precondition(first.calls == ["upload", "catalog", "manifest"] && !workflow.isRunning)
    precondition(complete.capabilities.contains("kindle.catalog") && complete.qualified)
    precondition(committedMessages.count == 1 && committedMessages[0].contains("private"))
    precondition(model.books.contains { $0.title == "First imported title" })

    let refreshFailure = try CatalogSourceProbe("B000000002", title: "Saved before failed refresh")
    refreshFailure.format = .sqliteSnapshot
    refreshFailure.refreshFailure = ConnectionInputError.message("Synthetic refresh unavailable")
    let partial = try await workflow.run(file: file, source: refreshFailure, model: model, didImport: { message in
        precondition(message.contains("No reading progress") && message.contains("privately"))
        precondition(model.books.contains { $0.title == "Saved before failed refresh" })
    })
    precondition(partial.capabilities.isEmpty && !partial.qualified)
    precondition(partial.status.contains("was imported") && partial.status.contains("Refresh capabilities") && partial.status.contains("Synthetic refresh unavailable"))

    // Postcommit cancellation must not turn a durable import into a failed import.
    let afterCommit = try CatalogSourceProbe("B000000003", title: "Saved before cancelled refresh")
    let refreshGate = CatalogGate(); afterCommit.refreshGate = refreshGate
    let cancelledRefresh = Task { try await workflow.run(file: file, source: afterCommit, model: model) }
    await refreshGate.waitUntilSuspended()
    precondition(workflow.isRunning && model.books.contains { $0.title == "Saved before cancelled refresh" })
    cancelledRefresh.cancel(); refreshGate.release()
    let cancelledResult = try await cancelledRefresh.value
    precondition(cancelledResult.capabilities.isEmpty && !cancelledResult.qualified && !workflow.isRunning)
    precondition(cancelledResult.status == "Your catalog was imported. Choose Refresh capabilities to update available companion actions.")

    // Cancellation-ignoring download must not initiate a local write afterward.
    let beforeCommit = try CatalogSourceProbe("B000000004", title: "Must not be imported")
    let downloadGate = CatalogGate(); beforeCommit.downloadGate = downloadGate
    let baseline = model.state!.revision
    let cancelledDownload = Task { try await workflow.run(file: file, source: beforeCommit, model: model) }
    await downloadGate.waitUntilSuspended()
    cancelledDownload.cancel(); downloadGate.release()
    do { _ = try await cancelledDownload.value; preconditionFailure("Cancelled download committed") }
    catch is CancellationError { }
    precondition(beforeCommit.calls == ["upload", "catalog"] && model.state!.revision == baseline && !workflow.isRunning)

    for failure in ["read", "upload", "catalog", "persist"] {
        let source = try CatalogSourceProbe("B000000005")
        if failure == "upload" { source.uploadFailure = ConnectionInputError.message("Synthetic upload failure") }
        if failure == "catalog" { source.downloadFailure = ConnectionInputError.message("Synthetic download failure") }
        let failing = CompanionCatalogImportWorkflow(readFile: { url in
            if failure == "read" { throw CocoaError(.fileReadNoPermission) }
            return try await CompanionKindleImport.readFile(url)
        }, persist: { desk, catalog in
            // Actual native validation/SQLite rejection, not a mirrored persistence stub.
            try await CompanionCatalogImportWorkflow.persistCatalog(failure == "persist" ? Data("invalid catalog".utf8) : catalog, model: desk)
        })
        do {
            _ = try await failing.run(file: file, source: source, model: model, didImport: { _ in preconditionFailure("Uncommitted import reported success") })
            preconditionFailure("Expected \(failure) failure")
        } catch { }
        precondition(model.state!.revision == baseline && !failing.isRunning && !source.calls.contains("manifest"))
        if failure == "read" { precondition(source.calls.isEmpty) }
        if failure == "upload" { precondition(source.calls == ["upload"]) }
    }

    // Delay a real committed SQLite snapshot, then complete another real save.
    // Replacing accept(snapshot) with state = snapshot fails this regression.
    let commitGate = CatalogGate()
    let delayed = CompanionCatalogImportWorkflow(persist: { desk, catalog in
        let persisted = try await CompanionCatalogImportWorkflow.persistCatalog(catalog, model: desk)
        await commitGate.suspend()
        return persisted
    })
    let delayedSource = try CatalogSourceProbe("B000000006", title: "Delayed committed catalog")
    let delayedTask = Task { try await delayed.run(file: file, source: delayedSource, model: model) }
    await commitGate.waitUntilSuspended()
    let competing = try CatalogSourceProbe("B000000007")
    do { _ = try await delayed.run(file: file, source: competing, model: model); preconditionFailure("Concurrent import accepted") }
    catch { precondition(bookNookErrorMessage(error).contains("Wait for the current")) }
    precondition(competing.calls.isEmpty && delayed.isRunning)
    let added = try await model.perform(["op": "add_book", "title": "Newer concurrent edit", "author": "Synthetic writer", "at": 500])
    let newerID = added!.object!["book_id"] as! String
    let newerRevision = model.state!.revision
    commitGate.release()
    _ = try await delayedTask.value
    precondition(model.state!.revision == newerRevision && model.state!.books[newerID] != nil,
                 "Delayed catalog response replaced a newer model snapshot")
    precondition(model.books.contains { $0.title == "Delayed committed catalog" })
    precondition(!delayed.isRunning && delayedSource.calls == ["upload", "catalog", "manifest"])

    let reopened = DeskModel(); await reopened.open()
    // Opening the model may create the first notebook; its revision can advance.
    precondition(reopened.state!.revision >= newerRevision && reopened.state!.books[newerID] != nil)
    for title in ["First imported title", "Saved before failed refresh", "Saved before cancelled refresh", "Delayed committed catalog"] {
        precondition(reopened.books.contains { $0.title == title }, "Committed import must survive SQLite reopen")
    }
    precondition(!reopened.books.contains { $0.title == "Must not be imported" })
    print("Actual catalog-import workflow: captured-source sequence, postcommit refresh failure/cancellation with preserved SQLite data, precommit cancellation, stage failures, concurrent rejection, delayed-snapshot monotonicity and reopen passed (synthetic source; no transport, Keychain or UI).")
}
