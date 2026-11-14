// SPDX-License-Identifier: MIT OR Apache-2.0
import Foundation
import CryptoKit
import SQLite3

private func check(_ value: Bool, _ message: String = "acceptance failure") { precondition(value, message) }

private final class State: @unchecked Sendable {
    private let lock = NSLock()
    private var callsValue = 0, canceledValue = false, modeValue = 0
    private weak var cacheValue: BookNookFeedCacheV1?
    private var cancellationValue: FeedSourceCancellationV1?
    func enter(_ cancellation: FeedSourceCancellationV1) -> (Int, BookNookFeedCacheV1?) { lock.lock(); defer { lock.unlock() }; callsValue += 1; cancellationValue = cancellation; return (modeValue, cacheValue) }
    func mode(_ mode: Int, cache: BookNookFeedCacheV1? = nil) { lock.lock(); defer { lock.unlock() }; modeValue = mode; cacheValue = cache }
    func calls() -> Int { lock.lock(); defer { lock.unlock() }; return callsValue }
    func tokenCanceled() -> Bool { lock.lock(); defer { lock.unlock() }; return cancellationValue?.isCancelled() ?? false }
    func observedCancellation() { lock.lock(); defer { lock.unlock() }; canceledValue = true }
    func sawCancellation() -> Bool { lock.lock(); defer { lock.unlock() }; return canceledValue }
}
private let xml = Data("""
<rss version="2.0"><channel><title>Calm café</title>
<item><guid>same</guid><title>A</title><pubDate>Thu, 01 Jan 1970 00:00:00 +0000</pubDate></item>
<item><guid>same</guid><title>B</title><pubDate>Wed, 31 Dec 1969 23:59:59 +0000</pubDate></item>
<item><guid>leap</guid><pubDate>Sat, 31 Dec 2016 23:59:60 +0000</pubDate></item>
<item><guid>bad</guid><pubDate>invalid</pubDate></item>
<item><guid>missing</guid><description><![CDATA[<img src="https://example.net/pixel"> café]]></description></item>
</channel></rss>
""".utf8)
private struct Provider: BookNookFeedSourceProviderV1 {
    let state: State
    var id = "reader.swift-cache"
    var descriptorJSON: String { "{\"schema\":\"booknook.feed-source-provider.v1\",\"version\":1,\"id\":\"\(id)\",\"available\":true,\"protocols\":[\"public-xml-bytes.v1\"]}" }
    func fetch(requestJSON: String, cancellation: FeedSourceCancellationV1) async throws -> BookNookFeedSourceReplyV1 {
        let (mode, cache) = state.enter(cancellation)
        if mode == 1 {
            while !cancellation.isCancelled() { await Task.yield() }
            state.observedCancellation(); throw CancellationError()
        }
        if let cache { _ = try await cache.configuration(); if mode == 2 { try await cache.pause(sourceID: sourceID) } }
        var response = try JSONSerialization.jsonObject(with: Data(requestJSON.utf8)) as! [String:Any]
        check(response["title"] == nil && response["collections"] == nil && response["identity"] == nil)
        response["schema"] = "booknook.feed-source-response.v1"
        let url = response.removeValue(forKey: "url")!
        for field in ["format", "protocol", "conditional"] { response.removeValue(forKey: field) }
        response["final_url"] = url; response["content_type"] = "application/rss+xml"; response["outcome"] = "body"
        response["body_sha256"] = SHA256.hash(data: xml).map { String(format: "%02x", $0) }.joined()
        response["validators"] = ["etag": "\"swift\"", "last_modified": NSNull()]
        return .init(responseBytes: try JSONSerialization.data(withJSONObject: response), untrustedBody: xml)
    }
}
private let sourceID = "b41ef5e8-0e06-43e0-aa47-19f382e00c08"
private func configuration() throws -> Data {
    try JSONSerialization.data(withJSONObject: ["schema": "booknook.feed-configuration.v1", "revision": 0, "feeds": ["schema": "castalia.feeds-section.v1", "version": 1,
        "subscriptions": [["id": sourceID, "url": "https://example.org/swift.xml", "format": "rss", "title": "Private label", "enabled": true, "collectionIds": [], "disclosure": "private"]],
        "collections": [], "preferences": ["order": "newest-first"]]], options: [.sortedKeys])
}
private func directory() throws -> URL { let url = FileManager.default.temporaryDirectory.appendingPathComponent("booknook-swift-cache-\(UUID().uuidString)"); try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true); return url }
private func until(_ predicate: () -> Bool) async throws { let end = Date().addingTimeInterval(5); while !predicate() { check(Date() < end, "acceptance deadline"); try await Task.sleep(nanoseconds: 1_000_000) } }
private func rejected(_ action: () async throws -> Void) async { var failed = false; do { try await action() } catch { failed = true }; check(failed) }
private func drainedConfiguration(_ cache: BookNookFeedCacheV1) async throws -> BookNookFeedCacheConfigurationV1 {
    let end = Date().addingTimeInterval(5)
    while true { do { return try await cache.configuration() } catch {
        guard String(describing: error).contains("queue"), Date() < end else { throw error }
        try await Task.sleep(nanoseconds: 1_000_000)
    } }
}
private final class Count: @unchecked Sendable { private let lock = NSLock(); private var value = 0; func increment() { lock.lock(); value += 1; lock.unlock() }; func get() -> Int { lock.lock(); defer { lock.unlock() }; return value } }
private final class Database {
    let pointer: OpaquePointer
    init(_ directory: URL) { var handle: OpaquePointer?; check(sqlite3_open(directory.appendingPathComponent("feeds.sqlite").path, &handle) == SQLITE_OK); pointer = handle! }
    func execute(_ sql: String) { check(sqlite3_exec(pointer, sql, nil, nil, nil) == SQLITE_OK) }
    deinit { sqlite3_close(pointer) }
}
private final class BlockingProvider: BookNookFeedSourceProviderV1, @unchecked Sendable {
    let entered = Count(), release = DispatchSemaphore(value: 0), state = State()
    var descriptorJSON: String { entered.increment(); release.wait(); return Provider(state: state).descriptorJSON }
    func fetch(requestJSON: String, cancellation: FeedSourceCancellationV1) async throws -> BookNookFeedSourceReplyV1 { try await Provider(state: state).fetch(requestJSON: requestJSON, cancellation: cancellation) }
}
@main struct FeedCacheSDKBindingSmoke {
    static func main() async throws {
        let raw = try configuration(), dir = try directory(), state = State(), provider = Provider(state: State())
        let cache = try await BookNookFeedCacheV1.open(directory: dir, expectedConfiguration: raw, provider: Provider(state: state))
        check(try await cache.configuration().raw == raw)
        await rejected { _ = try await cache.refresh(sourceID: sourceID) }; check(state.calls() == 0)
        await rejected { _ = try await BookNookFeedCacheV1.open(directory: dir, provider: Provider(state: state)) }
        try await cache.activate(sourceID: sourceID); state.mode(0, cache: cache)
        let first = try await cache.refresh(sourceID: sourceID)
        check(first.entries.count == 5 && Set(first.entries.map(\.variantKey)).count == 5)
        check(first.entries[0].published.nanoseconds == 1_000_000_000 && first.entries[1].published.unixSeconds == 0 && first.entries[2].published.unixSeconds == -1)
        check(first.entries[3].published.status == .invalid && first.entries[4].published.status == .missing)
        check(first.entries[4].summary?.representation == .literalMarkup && first.entries[4].summary!.text.contains("<img"))
        check(first.provenance.rawSHA256 == first.rawSHA256 && first.provenance.requestedURL == "https://example.org/swift.xml" && first.contextSHA256.count == 64)
        state.mode(2, cache: cache); await rejected { _ = try await cache.refresh(sourceID: sourceID) }
        check(try await cache.offline(sourceID: sourceID).cacheToken == first.cacheToken)
        try await cache.activate(sourceID: sourceID); state.mode(1)
        let calls = state.calls(), pending = Task { try await cache.refresh(sourceID: sourceID) }
        try await until { state.calls() > calls }; pending.cancel()
        check(state.tokenCanceled(), "Swift cancellation must synchronously reach the core token")
        await rejected { _ = try await pending.value }; try await until { state.sawCancellation() }
        check(try await cache.offline(sourceID: sourceID).cacheToken == first.cacheToken)
        state.mode(0)
        // Each real SQLite lock is released immediately after cancel(), without
        // waiting for canceled tasks/native futures to poll or finish first.
        for operation in 0..<3 {
            try await cache.pause(sourceID: sourceID)
            let before = try await cache.configuration(), database = Database(dir), full = Count()
            database.execute("BEGIN IMMEDIATE")
            var tasks: [Task<Bool,Never>] = []
            for _ in 0..<24 {
                tasks.append(Task {
                    do {
                        if operation == 0 { try await cache.activate(sourceID: sourceID) }
                        else if operation == 1 { var changed = raw; changed.append(32); try await cache.replaceConfiguration(expectedToken: before.configurationToken, raw: changed) }
                        else { try await cache.replaceProvider(expectedToken: before.providerToken, provider: provider) }
                        return true
                    } catch { if String(describing: error).contains("queue") { full.increment() }; return false }
                })
            }
            try await until { full.get() > 0 } // witnesses the real bounded queue
            for task in tasks { task.cancel() }
            database.execute("ROLLBACK")
            for task in tasks { check(await task.value == false) }
            let after = try await drainedConfiguration(cache)
            check(before.configurationToken == after.configurationToken && before.providerToken == after.providerToken)
            check(try await cache.status(sourceID: sourceID).locallyActive == false)
        }
        let snapshot = try await cache.configuration()
        await rejected { try await cache.replaceConfiguration(expectedToken: "wrong", raw: raw) }
        var changed = raw; changed.append(32)
        try await cache.replaceConfiguration(expectedToken: snapshot.configurationToken, raw: changed)
        check(try await cache.configuration().raw == changed)
        try await cache.activate(sourceID: sourceID); try await cache.pause(sourceID: sourceID)
        let beforeProvider = try await cache.configuration()
        try await cache.replaceProvider(expectedToken: beforeProvider.providerToken, provider: provider)
        check(try await cache.status(sourceID: sourceID).locallyActive == false)
        try await cache.close()
        let reopened = try await BookNookFeedCacheV1.open(directory: dir, provider: provider)
        check(try await reopened.status(sourceID: sourceID).locallyActive == false)
        let offline = try await reopened.offline(sourceID: sourceID)
        check(offline.offline && offline.entries.map(\.variantKey) == first.entries.map(\.variantKey))
        // A late cancellation cannot turn an already accepted method result into
        // rollback. Native tests additionally stop acknowledgement before repoll.
        let accepted = Task { try await reopened.activate(sourceID: sourceID) }
        try await accepted.value; accepted.cancel(); try await accepted.value
        check(try await reopened.status(sourceID: sourceID).locallyActive)
        try await reopened.close(); try FileManager.default.removeItem(at: dir)
        let openingDir = try directory(), blocking = BlockingProvider()
        let opening = Task { try await BookNookFeedCacheV1.open(directory: openingDir, expectedConfiguration: raw, provider: blocking) }
        try await until { blocking.entered.get() > 0 }; opening.cancel(); blocking.release.signal()
        await rejected { _ = try await opening.value }
        let clean = try await BookNookFeedCacheV1.open(directory: openingDir, provider: Provider(state: State()))
        try await clean.close(); try FileManager.default.removeItem(at: openingDir)
        let publicDir = try directory()
        let publicCache = try await BookNookFeedCacheV1.publicHTTPS(directory: publicDir, expectedConfiguration: raw)
        await rejected { _ = try await publicCache.refresh(sourceID: sourceID) }
        try await publicCache.close(); try FileManager.default.removeItem(at: publicDir)
        print("Actual generated Swift cache facade: offline reopen, exact CAS/provider/pause, callback reentry, typed variants/inert text/date fidelity, core cancellation, real SQLite held-write/queue cancellation, accepted result and canceled-open cleanup passed. No caller Tokio, external fetch, app or device qualification claimed.")
    }
}
