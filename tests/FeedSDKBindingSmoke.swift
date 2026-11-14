// SPDX-License-Identifier: MIT OR Apache-2.0
import Foundation
#if canImport(BookNookServices)
import BookNookServices
#endif
private func object(_ raw: String) throws -> [String: Any] {
    try JSONSerialization.jsonObject(with: Data(raw.utf8)) as! [String: Any]
}
private func bytes(_ object: Any) throws -> Data { try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]) }
private func rejected(_ operation: () throws -> Void) {
    var failure = false; do { try operation() } catch { failure = true }; precondition(failure)
}
@main struct FeedSDKBindingSmoke {
    static func main() throws {
        let root = URL(fileURLWithPath: CommandLine.arguments[1])
        let corpus = try JSONSerialization.jsonObject(with: Data(contentsOf: root.appendingPathComponent("conformance.json"))) as! [String: Any]
        let cases = corpus["cases"] as! [[String: Any]]
        precondition(cases.count == 102)
        for test in cases {
            let raw = Data((test["raw"] as! String).utf8)
            if test["valid"] as! Bool {
                let output = try BookNookFeedsV1.validateSection(raw)
                let expected = try JSONSerialization.jsonObject(with: raw) as! NSDictionary
                let decoded = try object(output)
                precondition(expected.isEqual(to: decoded), test["id"] as! String)
            } else { rejected { _ = try BookNookFeedsV1.validateSection(raw) } }
        }
        rejected { _ = try BookNookFeedsV1.validateSection(Data([0xc3, 0x28])) }
        let section = try JSONSerialization.jsonObject(with: Data(contentsOf: root.appendingPathComponent("section.json"))) as! [String: Any]
        let current: [String: Any] = ["schema":"booknook.feed-configuration.v1", "revision":0, "feeds":section]
        let currentBytes = try bytes(current)
        var incoming = section
        var subscriptions = incoming["subscriptions"] as! [[String: Any]]
        let sourceID = subscriptions[0]["id"] as! String
        subscriptions[0]["title"] = "Reviewed change"; incoming["subscriptions"] = subscriptions
        let raw = try bytes(incoming)
        let choices = try bytes([["target":"subscription", "id":sourceID, "choice":"incoming"]])
        let review = try BookNookFeedsV1.reviewImport(configuration: currentBytes, incoming: raw)
        let preview = try object(review.preview())
        precondition((preview["conflicts"] as! [Any]).count == 1)
        rejected { _ = try review.confirm(configuration: currentBytes, incoming: raw) }
        let resolved = try review.resolve(configuration: currentBytes, incoming: raw, resolutions: choices)
        let candidate = try object(resolved.confirm(configuration: currentBytes, incoming: raw))
        precondition(candidate["revision"] as! Int == 1)
        var changedRaw = raw; changedRaw.append(0x20)
        rejected { _ = try review.resolve(configuration: currentBytes, incoming: changedRaw, resolutions: choices) }
        rejected { _ = try resolved.confirm(configuration: currentBytes, incoming: changedRaw) }
        var changed = current; changed["revision"] = 1
        rejected { _ = try resolved.confirm(configuration: bytes(changed), incoming: raw) }
        rejected { _ = try BookNookFeedsV1.reviewImport(configuration: bytes(preview), incoming: raw) }
        resolved.close(); rejected { _ = try resolved.preview() }; resolved.close(); review.close()
        rejected { _ = try BookNookFeedsV1.reviewImport(configuration: currentBytes, incoming: Data()) }
        let absent = try BookNookFeedsV1.reviewImport(configuration: currentBytes, incoming: nil)
        let absentCandidate = try object(absent.confirm(configuration: currentBytes, incoming: nil))
        precondition(absentCandidate["revision"] as! Int == 0)
        absent.close()
        let orphan = "b41ef5e8-0e06-43e0-aa47-000000000999"
        var expanded = section
        var collections = expanded["collections"] as! [[String: Any]]
        collections.append(["id":orphan, "title":"Private orphan"]); expanded["collections"] = collections
        let config = try bytes(["schema":"booknook.feed-configuration.v1", "revision":0, "feeds":expanded])
        let selection = try bytes(["schema":"booknook.feed-disclosure-selection.v1", "origin":"https://reader.example.org", "subscriptionIds":[sourceID], "collectionIds":[orphan], "memberships":[["subscriptionId":sourceID, "collectionIds":[]]]] as [String: Any])
        let projection = try BookNookFeedsV1.reviewProjection(configuration: config, selection: selection)
        let output = try projection.confirm(configuration: config, origin:"https://reader.example.org")
        let selected = try object(output)["feeds"] as! [String: Any]
        precondition((selected["collections"] as! [Any]).isEmpty)
        precondition(!output.contains(orphan) && !output.contains("Private orphan"))
        rejected { _ = try projection.confirm(configuration: config, origin:"https://reader.example.org/") }
        projection.close(); rejected { _ = try projection.preview() }
        print("Actual generated UniFFI Swift feed SDK: 102 raw vectors, exact-review conflicts/staleness, opaque lifecycle and origin/collection closure passed.")
    }
}
