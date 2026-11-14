// SPDX-License-Identifier: MPL-2.0
import Foundation
import CryptoKit
import Darwin

private actor PublicationTestState {
    var requests = 0
    var waiting = false
    var fail = false
    func begin() -> (Bool, Bool) { requests += 1; return (waiting, fail) }
    func mode(waiting: Bool = false, fail: Bool = false) { self.waiting = waiting; self.fail = fail }
}
private struct PublicationTestProvider: BookNookFeedSourceProviderV1 {
    let state: PublicationTestState
    var descriptorJSON: String { #"{"schema":"booknook.feed-source-provider.v1","version":1,"id":"app.publication-tests","available":true,"protocols":["public-xml-bytes.v1"]}"# }
    func fetch(requestJSON: String, cancellation: FeedSourceCancellationV1) async throws -> BookNookFeedSourceReplyV1 {
        let (wait, fail) = await state.begin()
        if wait { while !cancellation.isCancelled() { await Task.yield() }; throw CancellationError() }
        if fail { throw PublicationFailure.message("Synthetic source unavailable") }
        let xml = Data("""
        <rss version="2.0"><channel><title>External café</title>
        <item><guid>collision</guid><title>First variant</title><pubDate>Thu, 01 Jan 1970 00:00:00 +0000</pubDate></item>
        <item><guid>collision</guid><title>Second variant</title><pubDate>Wed, 31 Dec 1969 23:59:59 +0000</pubDate></item>
        <item><guid>leap</guid><title>Leap second</title><pubDate>Sat, 31 Dec 2016 23:59:60 +0000</pubDate></item>
        <item><guid>invalid</guid><pubDate>not a date</pubDate><description><![CDATA[<img src="https://example.net/pixel"> **literal** café]]></description></item>
        <item><guid>missing</guid><title>Undated</title></item></channel></rss>
        """.utf8)
        var response = try JSONSerialization.jsonObject(with: Data(requestJSON.utf8)) as! [String: Any]
        response["schema"] = "booknook.feed-source-response.v1"
        response["final_url"] = response.removeValue(forKey: "url")
        for key in ["format", "protocol", "conditional"] { response.removeValue(forKey: key) }
        response["content_type"] = "application/rss+xml"; response["outcome"] = "body"
        response["body_sha256"] = SHA256.hash(data: xml).map { String(format: "%02x", $0) }.joined()
        response["validators"] = ["etag": "\"app-test\"", "last_modified": NSNull()]
        return .init(responseBytes: try JSONSerialization.data(withJSONObject: response), untrustedBody: xml)
    }
}
private func publicationReject(_ action: () async throws -> Void) async {
    var rejected = false; do { try await action() } catch { rejected = true }; precondition(rejected)
}
private func publicationCheck(_ condition: Bool) { precondition(condition) }
@MainActor private func publicationUntil(_ predicate: () -> Bool) async throws {
    let deadline = Date().addingTimeInterval(5)
    while !predicate() { precondition(Date() < deadline); try await Task.sleep(nanoseconds: 1_000_000) }
}

@MainActor func testPublicationWorkflow() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("booknook-publications-" + UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: root) }
    let state = PublicationTestState(), provider = PublicationTestProvider(state: PublicationTestState())
    let precisionBytes = Data(#"{"schema":"booknook.feed-configuration.v1","revision":9007199254740991,"feeds":{"schema":"castalia.feeds-section.v1","version":1,"subscriptions":[],"collections":[],"preferences":{"order":"newest-first"}}}"#.utf8)
    let precise = try JSONDecoder().decode(PublicationConfiguration.self, from: Data(try BookNookFeedsV1.validateConfiguration(precisionBytes).utf8))
    precondition(precise.revision == 9_007_199_254_740_991)
    let encodedPrecision = try BookNookFeedsV1.validateConfiguration(JSONEncoder().encode(precise))
    precondition(encodedPrecision.contains("9007199254740991"))
    let incomplete = root.appendingPathComponent("incomplete", isDirectory: true)
    try FileManager.default.createDirectory(at: incomplete, withIntermediateDirectories: true)
    let orphanBytes = Data("preserve orphan WAL".utf8), orphanFile = incomplete.appendingPathComponent("feeds.sqlite-wal")
    try orphanBytes.write(to: orphanFile)
    let unknown = PublicationRepository(directory: incomplete, provider: provider)
    await publicationReject { _ = try await unknown.snapshot() }
    publicationCheck(try Data(contentsOf: orphanFile) == orphanBytes)
    precondition(!FileManager.default.fileExists(atPath: incomplete.appendingPathComponent("feeds.sqlite").path))
    try await unknown.close()
    let repository = PublicationRepository(directory: root.appendingPathComponent("publications"), provider: PublicationTestProvider(state: state))
    // Concurrent callers/windows share one lazy cache owner; opening has no I/O to a source.
    async let first = repository.snapshot(); async let second = repository.snapshot()
    let (a, b) = try await (first, second)
    precondition(a.configuration == b.configuration && a.configuration.feeds.subscriptions.isEmpty)
    publicationCheck(await state.requests == 0)
    let add = try await repository.reviewAdd(title: "Private café", url: "https://example.org/feed.xml", format: "rss")
    try await repository.confirmEdit(add.id)
    let saved = try await repository.snapshot(), source = saved.configuration.feeds.subscriptions[0]
    let requestsBeforeActivation = await state.requests
    precondition(saved.statuses[source.id]?.locallyActive == false && requestsBeforeActivation == 0)
    await publicationReject { _ = try await repository.refresh(source.id, activate: false) }
    try await repository.refresh(source.id, activate: true)
    publicationCheck(await state.requests == 1)
    let before = try await repository.snapshot()
    let noChange = try await repository.reviewEnabled(source.id, enabled: true)
    precondition(!noChange.changes)
    publicationCheck(try await repository.confirmEdit(noChange.id) == false)
    publicationCheck(try await repository.snapshot().token == before.token)
    let empty = Data(#"{"schema":"castalia.feeds-section.v1","version":1,"subscriptions":[],"collections":[],"preferences":{"order":"newest-first"}}"#.utf8)
    let importFile = root.appendingPathComponent("exact-settings.json")
    var duplicateRaw = empty; duplicateRaw.removeLast(); duplicateRaw.append(contentsOf: #", "version": 1}"#.utf8)
    try duplicateRaw.write(to: importFile)
    let exactFileBytes = try await readPublicationImport(importFile)
    precondition(exactFileBytes == duplicateRaw)
    await publicationReject { _ = try await repository.reviewImport(exactFileBytes) }
    let fifo = root.appendingPathComponent("not-a-regular-file.json")
    precondition(mkfifo(fifo.path, S_IRUSR | S_IWUSR) == 0)
    await publicationReject { _ = try await readPublicationImport(fifo) }
    await publicationReject { _ = try await readPublicationImport(URL(string: "https://example.org/settings.json")!) }
    let canceledFileRead = Task { try await readPublicationImport(importFile) }; canceledFileRead.cancel()
    await publicationReject { _ = try await canceledFileRead.value }
    let unchanged = try await repository.reviewImport(empty)
    publicationCheck(try await repository.confirmImport(unchanged.id) == false)
    let preserved = try await repository.snapshot()
    precondition(preserved.token == before.token && preserved.statuses[source.id]?.locallyActive == true)
    let same = try JSONEncoder().encode(before.configuration.feeds)
    let idempotent = try await repository.reviewImport(same)
    publicationCheck(try await repository.confirmImport(idempotent.id) == false)
    await publicationReject { _ = try await repository.reviewImport(Data(#"{"feeds":{}}"#.utf8)) }
    await publicationReject { _ = try await repository.reviewImport(Data(repeating: 32, count: 262_145)) }
    var incoming = before.configuration.feeds; incoming.subscriptions[0].title = "Incoming title"
    let incomingBytes = try JSONEncoder().encode(incoming)
    let conflict = try await repository.reviewImport(incomingBytes)
    precondition(conflict.conflicts.count == 1)
    await publicationReject { _ = try await repository.confirmImport(conflict.id) }
    await publicationReject { _ = try await repository.resolveImport(conflict.id, choices: []) }
    let resolved = try await repository.resolveImport(conflict.id, choices: [.init(target: "subscription", id: source.id, choice: "current")])
    precondition(resolved.conflicts.isEmpty)
    publicationCheck(try await repository.confirmImport(conflict.id) == false)
    let canceled = try await repository.reviewImport(incomingBytes); await repository.cancelReviews()
    await publicationReject { _ = try await repository.confirmImport(canceled.id) }
    let stale = try await repository.reviewImport(incomingBytes)
    await publicationReject { _ = try await repository.reviewExport(.init(subscriptionIds: [source.id], collectionIds: [], memberships: [.init(subscriptionId: source.id, collectionIds: [])])) }
    let edit = try await repository.reviewEligibility(source.id, selected: true)
    try await repository.confirmEdit(edit.id)
    await publicationReject { _ = try await repository.confirmImport(stale.id) }
    let label = UUID().uuidString.lowercased(), orphan = UUID().uuidString.lowercased()
    var labeled = try await repository.snapshot().configuration.feeds
    labeled.collections = [.init(id: label, title: "Selected label"), .init(id: orphan, title: "Private orphan")]
    labeled.subscriptions[0].collectionIds = [label]
    let labelReview = try await repository.reviewImport(JSONEncoder().encode(labeled))
    _ = try await repository.resolveImport(labelReview.id, choices: [.init(target: "subscription", id: source.id, choice: "incoming")])
    publicationCheck(try await repository.confirmImport(labelReview.id))
    let export = try await repository.reviewExport(.init(subscriptionIds: [source.id], collectionIds: [label, orphan], memberships: [.init(subscriptionId: source.id, collectionIds: [label])]))
    precondition(export.section.collections.map(\.id) == [label] && export.section.subscriptions[0].collectionIds == [label])
    let raw = try await repository.confirmExport(export.id)
    _ = try BookNookFeedsV1.validateSection(raw)
    precondition(!String(decoding: raw, as: UTF8.self).contains("origin"))
    let exported = try JSONDecoder().decode(PublicationSection.self, from: raw)
    precondition(exported.collections.count == 1 && !String(decoding: raw, as: UTF8.self).contains("Private orphan"))
    let canceledExport = try await repository.reviewExport(.init(subscriptionIds: [], collectionIds: [], memberships: []))
    await repository.cancelReviews(); await publicationReject { _ = try await repository.confirmExport(canceledExport.id) }
    let index = try await repository.discovery()
    precondition(index.count == 5 && Set(index.map(\.id)).count == 5)
    precondition(index[0].title == "Leap second" && index[1].date.unixSeconds == 0 && index[2].date.unixSeconds == -1)
    precondition(index.contains { $0.date.status == .invalid } && index.contains { $0.date.status == .missing })
    precondition(index[3].firstSourceOrdinal < index[4].firstSourceOrdinal && index[3].date.status == .invalid)
    let view = try await repository.offline(source.id)
    precondition(view.entries.contains { $0.summary?.text.contains("<img") == true })
    let cacheToken = view.cacheToken
    await state.mode(waiting: true)
    let calls = await state.requests, pending = Task { try await repository.refresh(source.id, activate: true) }
    let deadline = Date().addingTimeInterval(5)
    while await state.requests == calls { precondition(Date() < deadline); await Task.yield() }
    pending.cancel(); await publicationReject { try await pending.value }
    publicationCheck(try await repository.offline(source.id).cacheToken == cacheToken)
    await state.mode(fail: true)
    await publicationReject { try await repository.refresh(source.id, activate: true) }
    let outage = try await repository.snapshot()
    precondition(outage.statuses[source.id]?.lastError != nil && outage.statuses[source.id]?.lastBodySuccessUnixSeconds != nil)
    let coordinator = PublicationsModel(repository: repository)
    coordinator.loadSavedArticles(); try await publicationUntil { coordinator.busy == nil }
    precondition(coordinator.entries.count == 5 && coordinator.snapshot != nil && coordinator.error == nil)
    let beforeReadCancel = try await repository.snapshot()
    coordinator.add(title: "Must not reappear", url: "https://example.net/not-added.xml", format: "rss")
    coordinator.cancelReview(); try await publicationUntil { coordinator.busy == nil }
    precondition(coordinator.editReview == nil && coordinator.importReview == nil && coordinator.exportReview == nil)
    publicationCheck(try await repository.snapshot().token == beforeReadCancel.token)
    coordinator.openArticle(index[0]); coordinator.cancelOperation(); try await publicationUntil { coordinator.busy == nil }
    precondition(coordinator.article == nil)
    let ownerA = UUID(), ownerB = UUID()
    let exportSelection = PublicationExportSelection(subscriptionIds: [source.id], collectionIds: [], memberships: [.init(subscriptionId: source.id, collectionIds: [])])
    coordinator.selectExport(exportSelection); try await publicationUntil { coordinator.busy == nil }
    coordinator.acceptExport(presentationID: ownerA); try await publicationUntil { coordinator.busy == nil }
    let firstDelivery = coordinator.exportDelivery!
    precondition(firstDelivery.presentationID == ownerA)
    coordinator.selectExport(exportSelection); try await publicationUntil { coordinator.busy == nil }
    coordinator.acceptExport(presentationID: ownerB); try await publicationUntil { coordinator.busy == nil }
    let secondDelivery = coordinator.exportDelivery!
    precondition(!coordinator.finishExport(firstDelivery.id))
    precondition(coordinator.exportDelivery?.id == secondDelivery.id && secondDelivery.presentationID == ownerB)
    coordinator.finishExport(secondDelivery.id)
    await state.mode(waiting: true)
    let coordinatorCalls = await state.requests
    coordinator.refresh(coordinator.sources[0])
    let coordinatorDeadline = Date().addingTimeInterval(5)
    while await state.requests == coordinatorCalls { precondition(Date() < coordinatorDeadline); await Task.yield() }
    coordinator.cancelOperation(); try await publicationUntil { coordinator.busy == nil }
    precondition(coordinator.snapshot?.statuses[source.id]?.locallyActive == true)
    precondition(coordinator.message?.contains("completed change may already be saved") == true)
    publicationCheck(try await repository.offline(source.id).cacheToken == cacheToken)
    try await repository.replaceProvider(provider)
    publicationCheck(try await repository.snapshot().statuses[source.id]?.locallyActive == false)
    try await repository.close()
    let reopened = PublicationRepository(directory: root.appendingPathComponent("publications"), provider: provider)
    publicationCheck(try await reopened.snapshot().statuses[source.id]?.locallyActive == false)
    publicationCheck(try await reopened.offline(source.id).entries.count == 5)
    try await reopened.close()
    let desk = DeskRepository(directory: root), model = DeskModel(repository: desk, publicationProvider: provider)
    let windowA = model.publications, windowB = model.publications
    async let windowSnapshotA = windowA.repository.snapshot(); async let windowSnapshotB = windowB.repository.snapshot()
    let (window1, window2) = try await (windowSnapshotA, windowSnapshotB)
    precondition(window1.token == window2.token && window1.configuration.feeds.subscriptions[0].id == source.id)
    publicationCheck(try await desk.resolvedDirectory() == root)
    try await model.publications.repository.close()
    print("Publications app workflow: real SDK/cache, shared lazy owner, inactive import, exact/stale/conflicting/canceled review, origin-free export, offline restart, variant/date/literal text, cancellation and outage preservation passed.")
}
