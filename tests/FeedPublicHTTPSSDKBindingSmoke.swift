// SPDX-License-Identifier: MIT OR Apache-2.0
import Foundation
import CryptoKit

private func bytes(_ value: Any) throws -> Data {
    try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
}
private func object(_ raw: String) throws -> [String: Any] {
    try JSONSerialization.jsonObject(with: Data(raw.utf8)) as! [String: Any]
}
private func rejected(_ action: () throws -> Void) {
    var failed = false; do { try action() } catch { failed = true }; precondition(failed)
}
private func rejectedAsync(_ action: () async throws -> Void) async {
    var failed = false; do { try await action() } catch { failed = true }; precondition(failed)
}

@main struct FeedPublicHTTPSSDKBindingSmoke {
    static func main() async throws {
        let root = URL(fileURLWithPath: CommandLine.arguments[1])
        var feeds = try JSONSerialization.jsonObject(with: Data(contentsOf: root.appendingPathComponent("section.json"))) as! [String: Any]
        let sources = feeds["subscriptions"] as! [[String: Any]]
        let sourceID = sources[0]["id"] as! String
        let configuration = try bytes(["schema": "booknook.feed-configuration.v1", "revision": 0, "feeds": feeds])

        rejected { _ = try BookNookFeedSourcesV1.publicHTTPS(configuration: Data("{}".utf8)) }
        let runtime = try BookNookFeedSourcesV1.publicHTTPS(configuration: configuration)
        await rejectedAsync { _ = try await runtime.fetch(sourceID: sourceID) }
        try runtime.close()

        // Begin/cancel before dispatch exercises the generated concrete factory
        // without ever requesting DNS or HTTP. Required CI stays entirely offline.
        let direct = try FeedSourceSessionV1.publicHttps(configurationBytes: configuration)
        rejected { _ = try direct.begin(sourceId: sourceID, conditionalBytes: nil) }
        try direct.activate(sourceId: sourceID)
        let cancelled = try direct.begin(sourceId: sourceID, conditionalBytes: nil)
        precondition(cancelled.cancel())
        await rejectedAsync { _ = try await cancelled.execute() }
        try direct.replaceConfiguration(configurationBytes: configuration)
        rejected { _ = try direct.begin(sourceId: sourceID, conditionalBytes: nil) }
        try direct.close()
        print("Actual generated Swift native HTTPS factory: offline construction, malformed configuration, inactive source, pre-dispatch cancellation and revocation passed.")

        // Explicit developer-only public endpoint acceptance. Never enabled in CI.
        if let endpoint = ProcessInfo.processInfo.environment["BOOKNOOK_FEED_ACCEPTANCE_URL"] {
            var selected = sources
            selected[0]["url"] = endpoint
            feeds["subscriptions"] = selected
            let liveConfiguration = try bytes(["schema": "booknook.feed-configuration.v1", "revision": 0, "feeds": feeds])
            let live = try BookNookFeedSourcesV1.publicHTTPS(configuration: liveConfiguration)
            await rejectedAsync { _ = try await live.fetch(sourceID: sourceID) }
            try live.activate(sourceID: sourceID)
            let read = try await live.fetch(sourceID: sourceID)
            let body = read.untrustedBody()
            let response = try object(read.responseJSON())
            let digest = SHA256.hash(data: body).map { String(format: "%02x", $0) }.joined()
            precondition(!body.isEmpty && body.count <= 2 * 1024 * 1024)
            precondition(response["body_sha256"] as! String == digest)
            precondition(response["provider_id"] as! String == "booknook.public-https.v1")
            let request = try object(read.requestJSON())
            let context = try bytes([
                "schema": "booknook.feed-normalization-context.v1",
                "source_id": request["source_id"]!, "provider_id": request["provider_id"]!,
                "requested_url": request["url"]!, "final_url": response["final_url"]!,
                "expected_format": request["format"]!
            ])
            let normalized = try object(normalizeFeedXmlV1(contextBytes: context, xmlBytes: body))
            precondition(normalized["schema"] as! String == "booknook.normalized-feed.v1")
            precondition(normalized["normalizer_profile"] as! String == "booknook.rss2-atom.inert.v1")
            let provenance = normalized["provenance"] as! [String: Any]
            precondition(provenance["raw_sha256"] as! String == digest)
            for field in ["source_id", "provider_id"] { precondition(provenance[field] as! String == request[field] as! String) }
            precondition(provenance["requested_url"] as! String == request["url"] as! String)
            precondition(provenance["final_url"] as! String == response["final_url"] as! String)
            let entries = normalized["entries"] as! [[String: Any]]
            precondition(!entries.isEmpty)
            precondition((entries[0]["identity"] as! [String: Any])["kind"] as! String == "native")
            _ = try read.cacheConditionJSON()
            read.close()
            rejected { _ = try read.cacheConditionJSON() }
            try live.replaceConfiguration(liveConfiguration)
            await rejectedAsync { _ = try await live.fetch(sourceID: sourceID) }
            try live.close()
            print("Opt-in generated Swift production DNS/TLS HTTPS to XML: \(body.count) bytes, \(entries.count) inert entries, sha256 \(digest). No durable storage or rendering claim.")
        }
    }
}
