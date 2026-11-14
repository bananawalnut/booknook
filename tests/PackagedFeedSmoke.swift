// SPDX-License-Identifier: MIT OR Apache-2.0
import Foundation
import CryptoKit
import BookNookNative
import BookNookServices

private final class PackagedFeedCounter: @unchecked Sendable {
    private let lock = NSLock()
    private var count = 0
    func increment() { lock.lock(); count += 1; lock.unlock() }
    func value() -> Int { lock.lock(); defer { lock.unlock() }; return count }
}
private struct PackagedFeedProvider: BookNookFeedSourceProviderV1 {
    let counter: PackagedFeedCounter
    var descriptorJSON: String { #"{"schema":"booknook.feed-source-provider.v1","version":1,"id":"package.feed-smoke","available":true,"protocols":["public-xml-bytes.v1"]}"# }
    func fetch(requestJSON: String, cancellation: FeedSourceCancellationV1) async throws -> BookNookFeedSourceReplyV1 {
        precondition(!cancellation.isCancelled()); counter.increment()
        let xml = Data("""
        <rss version="2.0"><channel><title>Packaged café</title>
        <item><guid>same</guid><title>Epoch</title><pubDate>Thu, 01 Jan 1970 00:00:00 +0000</pubDate></item>
        <item><guid>same</guid><title>Before epoch</title><pubDate>Wed, 31 Dec 1969 23:59:59 +0000</pubDate></item>
        <item><guid>leap</guid><pubDate>Sat, 31 Dec 2016 23:59:60 +0000</pubDate></item>
        <item><guid>invalid</guid><pubDate>invalid</pubDate></item>
        <item><guid>missing</guid><description><![CDATA[<img src="https://example.net/pixel"> **literal** café]]></description></item>
        </channel></rss>
        """.utf8)
        var response = try JSONSerialization.jsonObject(with: Data(requestJSON.utf8)) as! [String: Any]
        precondition(response["title"] == nil && response["collections"] == nil && response["identity"] == nil)
        response["schema"] = "booknook.feed-source-response.v1"
        response["final_url"] = response.removeValue(forKey: "url")
        for key in ["format", "protocol", "conditional"] { response.removeValue(forKey: key) }
        response["content_type"] = "application/rss+xml"; response["outcome"] = "body"
        response["body_sha256"] = SHA256.hash(data: xml).map { String(format: "%02x", $0) }.joined()
        response["validators"] = ["etag": "\"packaged\"", "last_modified": NSNull()]
        return .init(responseBytes: try JSONSerialization.data(withJSONObject: response), untrustedBody: xml)
    }
}
private func packagedFeedCheck(_ condition: Bool) { precondition(condition) }

/// This file is copied only into the independent consumer of the extracted SDK.
/// It imports public products and has no reference-app/native source dependency.
func runPackagedFeedSmoke() async throws {
    let id = "b41ef5e8-0e06-43e0-aa47-19f382e00c08"
    let section = Data("""
    {"schema":"castalia.feeds-section.v1","version":1,"subscriptions":[{"id":"\(id)","url":"https://example.org/package.xml","format":"rss","title":"Selected label","enabled":true,"collectionIds":[],"disclosure":"selected"}],"collections":[],"preferences":{"order":"newest-first"}}
    """.utf8)
    let empty = Data(try BookNookFeedsV1.emptyConfiguration().utf8)
    let imported = try BookNookFeedsV1.reviewImport(configuration: empty, incoming: section)
    let configuration = Data(try imported.confirm(configuration: empty, incoming: section).utf8)
    imported.close()
    let selection = Data("""
    {"schema":"booknook.feed-export-selection.v1","subscriptionIds":["\(id)"],"collectionIds":[],"memberships":[{"subscriptionId":"\(id)","collectionIds":[]}]}
    """.utf8)
    let export = try BookNookFeedsV1.reviewExport(configuration: configuration, selection: selection)
    let rawSection = Data(try export.confirm(configuration: configuration).utf8)
    _ = try BookNookFeedsV1.validateSection(rawSection)
    precondition(!String(decoding: rawSection, as: UTF8.self).contains("origin"))
    export.close()
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("booknook-packaged-feed-" + UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: root) }
    let counter = PackagedFeedCounter(), provider = PackagedFeedProvider(counter: PackagedFeedCounter())
    let cache = try await BookNookFeedCacheV1.open(directory: root, expectedConfiguration: configuration, provider: PackagedFeedProvider(counter: counter))
    packagedFeedCheck(try await cache.status(sourceID: id).locallyActive == false)
    var refused = false
    do { _ = try await cache.refresh(sourceID: id) } catch { refused = true }
    precondition(refused && counter.value() == 0)
    try await cache.activate(sourceID: id)
    let view = try await cache.refresh(sourceID: id)
    precondition(counter.value() == 1 && view.entries.count == 5 && Set(view.entries.map(\.variantKey)).count == 5)
    precondition(view.entries.contains { $0.published.unixSeconds == 0 } && view.entries.contains { $0.published.unixSeconds == -1 })
    precondition(view.entries.contains { $0.published.nanoseconds == 1_000_000_000 })
    precondition(view.entries.contains { $0.published.status == .missing } && view.entries.contains { $0.published.status == .invalid })
    precondition(view.entries.contains { $0.summary?.representation == .literalMarkup && $0.summary?.text.contains("<img") == true })
    precondition(view.rawSHA256 == view.provenance.rawSHA256 && view.contextSHA256.count == 64)
    try await cache.pause(sourceID: id)
    packagedFeedCheck(try await cache.offline(sourceID: id).cacheToken == view.cacheToken)
    try await cache.close()
    let reopened = try await BookNookFeedCacheV1.open(directory: root, provider: provider)
    packagedFeedCheck(try await reopened.status(sourceID: id).locallyActive == false)
    let offline = try await reopened.offline(sourceID: id)
    precondition(offline.offline && offline.entries.map(\.variantKey) == view.entries.map(\.variantKey) && offline.rawSHA256 == view.rawSHA256)
    precondition(provider.counter.value() == 0)
    try await reopened.close()
    print("Independent extracted Swift feed SDK: raw import/export, inactive callback source, explicit refresh, typed variants/dates/inert text, durable offline reopen passed. No network, caller Tokio, app, device or minimum-OS runtime qualification.")
}
