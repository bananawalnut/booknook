// SPDX-License-Identifier: MIT OR Apache-2.0
import Foundation
#if canImport(BookNookServices)
import BookNookServices
#endif
#if canImport(BookNookNative)
import BookNookNative
#endif

private func notebookSmokeRequire(_ condition: Bool, _ message: String = "Notebook smoke assertion failed") {
    precondition(condition, message)
}
private func notebookSmokeObject(_ raw: String) throws -> [String: Any] {
    try JSONSerialization.jsonObject(with: Data(raw.utf8)) as! [String: Any]
}
private func notebookSmokeCommand(_ title: String) throws -> String {
    String(decoding: try JSONSerialization.data(withJSONObject: [
        "op": "add_book", "title": title, "author": "Synthetic", "at": 1
    ], options: [.sortedKeys]), as: UTF8.self)
}
private func notebookSmokeReject(_ operation: () throws -> Void) {
    var rejected = false
    do { try operation() } catch { rejected = true }
    notebookSmokeRequire(rejected, "Notebook reported success after rejected storage operation")
}

/// An actual alternate file-backed host, with all in-process writers serialized
/// by this shared lock. A production multiprocess host needs cross-process CAS.
private final class NotebookFileHost: BookNookNotebookStorage, @unchecked Sendable {
    let url: URL
    private let lock = NSLock()
    private var available = true
    private var failing = false
    private var externalWrite = false
    private var saves = 0
    init(url: URL, initialJSON: String) throws {
        self.url = url
        try Data(initialJSON.utf8).write(to: url, options: .atomic)
    }
    func descriptorJSON() throws -> String {
        lock.lock(); defer { lock.unlock() }
        return #"{"id":"example.swift-file","capabilities":["notebook_storage"],"available":\#(available),"reason":null}"#
    }
    func loadJSON() throws -> String {
        lock.lock(); defer { lock.unlock() }
        return try String(contentsOf: url, encoding: .utf8)
    }
    func compareAndSave(expectedRevision: UInt64, stateJSON: String) throws -> Bool {
        lock.lock()
        let inject = externalWrite
        externalWrite = false
        lock.unlock()
        if inject {
            // A separate engine writes through the same persistent host before
            // the first engine's CAS. No Rust lock spans these host callbacks.
            let other = try BookNookNotebook(storage: self)
            _ = try other.execute(commandJSON: notebookSmokeCommand("External writer 🌿"))
        }
        lock.lock(); defer { lock.unlock() }
        saves += 1
        if failing { throw BookNookServiceError.message("Synthetic disk full") }
        let current = try notebookSmokeObject(String(contentsOf: url, encoding: .utf8))
        guard (current["revision"] as! NSNumber).uint64Value == expectedRevision else { return false }
        try Data(stateJSON.utf8).write(to: url, options: .atomic)
        return true
    }
    func setAvailable(_ value: Bool) { lock.lock(); defer { lock.unlock() }; available = value }
    func setFailing(_ value: Bool) { lock.lock(); defer { lock.unlock() }; failing = value }
    func raceNextSave() { lock.lock(); defer { lock.unlock() }; externalWrite = true }
    func saveCount() -> Int { lock.lock(); defer { lock.unlock() }; return saves }
}

/// Compiled and executed against the independently extracted Swift package.
/// All inputs, paths and writers are synthetic; no application store is accessed.
@MainActor func runPackagedNotebookSmoke(fixtureRoot: URL) throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("booknook-storage-smoke-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let localURL = root.appendingPathComponent("local")
    let local = try BookNookNotebook(localDirectory: localURL)
    let host = try NotebookFileHost(url: root.appendingPathComponent("alternate.json"), initialJSON: local.snapshotJSON())
    let notebook = try BookNookNotebook(storage: host)
    let command = try notebookSmokeCommand("Private Unicode 中文 🌿")
    let response = try notebookSmokeObject(notebook.execute(commandJSON: command))
    notebookSmokeRequire((response["state"] as! [String: Any])["revision"] as! Int == 1)
    let first = try notebook.snapshotJSON()
    let reopened = try BookNookNotebook(storage: host)
    notebookSmokeRequire(try reopened.snapshotJSON() == first)
    // Local storage is separate; no callback operation silently touched SQLite.
    notebookSmokeRequire(try notebookSmokeObject(local.snapshotJSON())["revision"] as! Int == 0)
    _ = try local.execute(commandJSON: command)
    let localReopen = try BookNookNotebook(localDirectory: localURL)
    notebookSmokeRequire(try localReopen.snapshotJSON() == local.snapshotJSON())

    host.raceNextSave()
    notebookSmokeReject { _ = try notebook.execute(commandJSON: notebookSmokeCommand("Must not overwrite")) }
    let afterRace = try notebook.snapshotJSON()
    let state = try notebookSmokeObject(afterRace)
    let titles = (state["books"] as! [String: [String: Any]]).values.map { $0["title"] as! String }
    notebookSmokeRequire(Set(titles) == Set(["Private Unicode 中文 🌿", "External writer 🌿"]))
    notebookSmokeRequire(state["revision"] as! Int == 2)
    notebookSmokeRequire(host.saveCount() == 3, "One initial, one external, one rejected CAS; no automatic retry")
    notebookSmokeRequire(try BookNookNotebook(storage: host).snapshotJSON() == afterRace)

    host.setFailing(true)
    notebookSmokeReject { _ = try notebook.execute(commandJSON: command) }
    host.setFailing(false)
    notebookSmokeRequire(try notebook.snapshotJSON() == afterRace)
    host.setAvailable(false)
    notebookSmokeReject { _ = try notebook.snapshotJSON() }
    notebookSmokeReject { _ = try notebook.execute(commandJSON: command) }
    host.setAvailable(true)
    notebookSmokeRequire(try notebook.snapshotJSON() == afterRace)

    let archive = #"{"schema":"booknook.portable-export.v1","state":\#(afterRace),"assets":{}}"#
    let validated = try validatePortableArchive(archiveJson: archive)
    let envelope = try notebookSmokeObject(validated)
    notebookSmokeRequire(envelope["schema"] as! String == "booknook.portable-export.v1")
    notebookSmokeRequire((envelope["state"] as! NSDictionary).isEqual(to: state))
    notebookSmokeRequire((envelope["assets"] as! [String: Any]).isEmpty)
    notebookSmokeRequire(try validatePortableArchive(archiveJson: validated) == validated)
    let unknown = #"{"extra":true,"schema":"booknook.portable-export.v1","state":\#(afterRace),"assets":{}}"#
    notebookSmokeReject { _ = try validatePortableArchive(archiveJson: unknown) }
    try runPackagedNotebookOrderSmoke(fixtureRoot: fixtureRoot, root: root)
    print("Packaged standalone Notebook: storage errors, durable order review, reopen and v1/v4 archive validation passed.")
}

private func runPackagedNotebookOrderSmoke(fixtureRoot: URL, root: URL) throws {
    let fixture = fixtureRoot.appendingPathComponent("notebook-order.json")
    let cases = try notebookSmokeObject(String(contentsOf: fixture, encoding: .utf8))["cases"] as? [[String: Any]]
    guard let reviewCase = cases?.first(where: { $0["id"] as? String == "atomic-review" }),
          let initial = reviewCase["state"],
          let command = reviewCase["input"] as? [String: Any],
          let selected = command["selected"] as? [String],
          let notebookID = command["notebook_id"] as? String else {
        throw BookNookServiceError.message("The packaged notebook order fixture is incomplete")
    }
    let initialJSON = String(decoding: try JSONSerialization.data(withJSONObject: initial), as: UTF8.self)
    let host = try NotebookFileHost(url: root.appendingPathComponent("review.json"), initialJSON: initialJSON)
    let notebook = try BookNookNotebook(storage: host)
    let before = try Data(contentsOf: host.url)
    let reviewJSON = try notebook.orderReviewsJSON()
    let reviews = try JSONSerialization.jsonObject(with: Data(reviewJSON.utf8)) as? [[String: Any]]
    notebookSmokeRequire(reviews?.count == 1)
    guard let review = reviews?.first,
          let choices = review["choices"] as? [[String]] else {
        throw BookNookServiceError.message("The packaged notebook returned no order choices")
    }
    notebookSmokeRequire(review["notebook_id"] as? String == notebookID)
    notebookSmokeRequire(review["evidence"] as? String == command["evidence"] as? String)
    notebookSmokeRequire(review["heads"] as? [String] == command["heads"] as? [String])
    notebookSmokeRequire(choices.count == 2 && choices.contains(selected))
    notebookSmokeRequire(try Data(contentsOf: host.url) == before && host.saveCount() == 0)

    var stale = command
    stale["evidence"] = String(repeating: "0", count: 64)
    let staleJSON = String(decoding: try JSONSerialization.data(withJSONObject: stale), as: UTF8.self)
    notebookSmokeReject { _ = try notebook.execute(commandJSON: staleJSON) }
    notebookSmokeRequire(try Data(contentsOf: host.url) == before && host.saveCount() == 0)

    let commandJSON = String(decoding: try JSONSerialization.data(withJSONObject: command), as: UTF8.self)
    _ = try notebook.execute(commandJSON: commandJSON)
    let reopened = try BookNookNotebook(storage: host)
    let snapshot = try reopened.snapshotJSON()
    let state = try notebookSmokeObject(snapshot)
    let notebooks = state["notebooks"] as? [String: [String: Any]]
    let blocks = notebooks?[notebookID]?["blocks"] as? [[String: Any]]
    notebookSmokeRequire(blocks?.compactMap { $0["id"] as? String } == selected)
    notebookSmokeRequire(state["schema"] as? String == "booknook.archive.v4")
    notebookSmokeRequire(try reopened.orderReviewsJSON() == "[]" && host.saveCount() == 1)
    let archive = #"{"schema":"booknook.portable-export.v1","state":\#(snapshot),"assets":{}}"#
    let restored = try notebookSmokeObject(validatePortableArchive(archiveJson: archive))
    notebookSmokeRequire((restored["state"] as? NSDictionary)?.isEqual(to: state) == true)
}
