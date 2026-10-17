// SPDX-License-Identifier: MIT OR Apache-2.0
import Foundation
import CryptoKit
#if canImport(BookNookServices)
import BookNookServices
#endif
#if canImport(BookNookNative)
import BookNookNative
#endif

// This same alternate implementation is exercised by the installed-package consumer
// and the reference app's actual repository workflow. It performs no network I/O.
actor PackagedAlternateProvider: BookNookProvider {
    nonisolated let descriptorJSON: String
    nonisolated let id: String
    let bytes: Data
    private var title = "Alternate provider book"
    private var paused = false
    private var failAfterPause = false
    private var pending: CheckedContinuation<Void, Never>?
    private var waiting: CheckedContinuation<Void, Never>?
    private var requests: [String] = []
    init(bytes: Data, id: String = "example.alternate") throws {
        self.id = id; self.bytes = bytes
        descriptorJSON = try BookNookProviderWire.descriptor(id: id, sourceNamespace: "example.alternate",
            capabilities: ["catalog", "content", "positions", "identity", "recommendations"])
    }
    func setTitle(_ text: String) { title = text }
    func pauseNext(fail: Bool = false) { paused = true; failAfterPause = fail }
    func waitForRequest() async {
        if pending != nil { return }
        await withCheckedContinuation { waiting = $0 }
    }
    func release() { pending?.resume(); pending = nil }
    func receivedRequests() -> [String] { requests }
    func execute(requestJSON: String) async throws -> BookNookProviderResponse {
        requests.append(requestJSON)
        if paused {
            paused = false
            // Intentionally ignores cancellation; the real facade must gate commit.
            await withCheckedContinuation { pending = $0; waiting?.resume(); waiting = nil }
            if failAfterPause { failAfterPause = false; throw CancellationError() }
        }
        let request = try providerSmokeObject(requestJSON)
        let operation = request["operation"] as! [String: Any]
        let kind = operation["kind"] as! String
        let result: [String: Any]
        var content = Data()
        switch kind {
        case "catalog": result = ["kind": kind, "catalog": ["format": "normalized", "entries": [[
            "source_id": "edition-1", "title": title, "authors": ["Synthetic writer"], "subjects": ["friendship"],
            "edition_identifiers": [], "language": "en"
        ]]]]
        case "content":
            content = bytes
            result = ["kind": kind, "source_id": operation["source_id"]!, "media_type": operation["media_type"]!,
                "sha256": SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()]
        case "source_positions": result = ["kind": kind, "observations": [["source_id": "edition-1", "value": "chapter:2/line:4", "observed_at": UInt64(Date().timeIntervalSince1970)]]]
        case "identity": result = ["kind": kind, "reference": ["entity_iri": operation["entity_iri"]!, "kind": "Person", "display_name": "Synthetic directory person"]]
        case "recommendations":
            let input = operation["input"] as! [String: Any]
            let candidates = input["candidates"] as! [[String: Any]]
            result = ["kind": kind, "recommendations": candidates.prefix(1).map { ["book_id": $0["book_id"]!, "score": 1, "reasons": ["Alternate source catalog match"]] }]
        default: throw BookNookServiceError.message("Unsupported synthetic source operation")
        }
        return try BookNookProviderWire.response(to: requestJSON, resultJSON: BookNookProviderWire.json(result), contentBytes: content)
    }
}
func providerSmokeObject(_ raw: String) throws -> [String: Any] { try JSONSerialization.jsonObject(with: Data(raw.utf8)) as! [String: Any] }
func providerSmokeState(_ raw: String) throws -> [String: Any] { let object = try providerSmokeObject(raw); return object["state"] as? [String: Any] ?? object }
func providerSmokeBooks(_ raw: String) throws -> [String: [String: Any]] { try providerSmokeState(raw)["books"] as! [String: [String: Any]] }
func providerSmokeReject(_ operation: () async throws -> Void) async throws {
    var rejected = false
    do { try await operation() } catch { rejected = true }
    precondition(rejected, "Invalid/stale/cancelled provider operation was accepted")
}

@MainActor func runPackagedProviderSmoke(fixtureRoot: URL) async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("booknook-packaged-provider-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let bytes = try Data(contentsOf: fixtureRoot.appendingPathComponent("provider-sample.epub"))
    let provider = try PackagedAlternateProvider(bytes: bytes)
    let services = try BookNookServices(directory: directory, providers: [provider])
    let first = try await services.importCatalog(providerID: provider.id, sourceID: "selection")
    let books = try providerSmokeBooks(first); precondition(books.count == 1)
    let bookID = books.keys.first!
    _ = try services.desk.execute(commandJson: BookNookProviderWire.json(["op": "update_book", "book_id": bookID,
        "title": "Reader's chosen title", "shelf": "read", "rating": 4, "review": "Private Unicode café 🌿 日本語"]))
    await provider.setTitle("Updated source title")
    let updated = try await services.importCatalog(providerID: provider.id, sourceID: "selection")
    let book = try providerSmokeBooks(updated)[bookID]!
    let updatedBooks = try providerSmokeBooks(updated)
    precondition(updatedBooks.count == 1)
    precondition(book["review"] as? String == "Private Unicode café 🌿 日本語" && book["rating"] as? Int == 4 && book["shelf"] as? String == "read")
    let attached = try await services.importContent(providerID: provider.id, bookID: bookID, sourceID: "edition-1", mediaType: "application/epub+zip")
    let editions = try providerSmokeState(attached)["editions"] as! [String: [String: Any]]
    let assets = editions.values.flatMap { $0["assets"] as! [[String: Any]] }
    precondition(assets.count == 1 && !(assets[0]["spine"] as! [String]).isEmpty)
    let identity = try await services.identity(providerID: provider.id, entityIRI: "urn:example:synthetic-person")
    precondition(identity.executionJSON.contains("Synthetic directory person"))
    // Source positions preserve their own vocabulary and remain explicitly unmatched.
    let opened = try providerSmokeObject(services.desk.execute(commandJson: BookNookProviderWire.json(["op": "open_notebook", "book_id": bookID, "at": UInt64(Date().timeIntervalSince1970)])))
    let notebook = opened["result"] as! [String: Any]
    let positions = try await services.sourcePositions(providerID: provider.id, sourceIDs: ["edition-1"])
    let response = try providerSmokeObject(positions.executionJSON)["response"] as! [String: Any]
    let observations = (response["result"] as! [String: Any])["observations"] as! [[String: Any]]
    let positioned = try await services.attachPosition(positions, observationJSON: BookNookProviderWire.json(observations[0]), passID: notebook["pass_id"] as! String)
    let savedPositions = try providerSmokeState(positioned)["positions"] as! [String: [String: Any]]
    precondition(savedPositions.count == 1)
    let position = savedPositions.values.first!
    precondition((position["location"] as! [String: Any])["value"] as? String == "chapter:2/line:4")
    precondition((position["alignment"] as! [String: Any])["status"] as? String == "unmatched")
    let baseline = try services.desk.snapshot()
    // Generation changes invalidate completed reads and in-flight replies.
    let read = try await services.fetch(providerID: provider.id, operationJSON: "{\"kind\":\"catalog\",\"source_id\":\"selection\"}")
    try await services.replaceProviders([provider])
    try await providerSmokeReject { _ = try await services.commitCatalog(read) }
    await provider.pauseNext()
    let cancelled = Task { try await services.importCatalog(providerID: provider.id, sourceID: "selection") }
    await provider.waitForRequest(); cancelled.cancel(); await provider.release()
    try await providerSmokeReject { _ = try await cancelled.value }
    let afterCancel = try services.desk.snapshot()
    precondition(afterCancel == baseline)
    let limited = BookNookClosureProvider(descriptorJSON: try BookNookProviderWire.descriptor(id: "example.limited", sourceNamespace: "example.limited", capabilities: ["catalog"])) { _ in
        preconditionFailure("Capability denial must occur before adapter execution")
    }
    try await services.register(limited)
    try await providerSmokeReject { _ = try await services.importContent(providerID: "example.limited", bookID: bookID, sourceID: "x", mediaType: "application/pdf") }
    let reopened = try BookNookServices(directory: directory, providers: [provider])
    let reopenedState = try reopened.desk.snapshot()
    precondition(reopenedState == baseline)
    print("Installed public Swift provider: alternate catalog/content/identity/opaque positions, private edits/deduplication, cancellation, stale generation, capability denial and SQLite reopen passed.")
}
