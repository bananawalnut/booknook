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
private func nonStrings(_ literal: String) -> [Any] { [NSNull(),true,1,1.5,[Any](),[literal],[literal:NSNull()]] }
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
        for (field,literal) in [("schema","castalia.feeds-section.v1"),("format","atom"),("disclosure","selected"),("order","newest-first")] {
            for bad in nonStrings(literal) {
                var alias=section
                if field=="schema" {alias[field]=bad}
                else if field=="order" {var preferences=alias["preferences"] as! [String:Any];preferences[field]=bad;alias["preferences"]=preferences}
                else {var sources=alias["subscriptions"] as! [[String:Any]];sources[0][field]=bad;alias["subscriptions"]=sources}
                rejected {_ = try BookNookFeedsV1.validateSection(bytes(alias))}
            }
        }
        for bad in nonStrings("booknook.feed-configuration.v1") {var alias=current;alias["schema"]=bad;rejected {_ = try BookNookFeedsV1.validateConfiguration(bytes(alias))}}
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
        for (field,literal) in [("target","subscription"),("choice","incoming")] {for bad in nonStrings(literal) {
            var alias=try JSONSerialization.jsonObject(with:choices) as! [[String:Any]];alias[0][field]=bad
            rejected {_ = try review.resolve(configuration:currentBytes,incoming:raw,resolutions:bytes(alias))}
        }}
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
        for bad in nonStrings("booknook.feed-disclosure-selection.v1") {var alias=try JSONSerialization.jsonObject(with:selection) as! [String:Any];alias["schema"]=bad;rejected {_ = try BookNookFeedsV1.reviewProjection(configuration:config,selection:bytes(alias))}}
        let projection = try BookNookFeedsV1.reviewProjection(configuration: config, selection: selection)
        let output = try projection.confirm(configuration: config, origin:"https://reader.example.org")
        let selected = try object(output)["feeds"] as! [String: Any]
        precondition((selected["collections"] as! [Any]).isEmpty)
        precondition(!output.contains(orphan) && !output.contains("Private orphan"))
        rejected { _ = try projection.confirm(configuration: config, origin:"https://reader.example.org/") }
        rejected { _ = try BookNookFeedsV1.validateSection(Data(output.utf8)) }
        projection.close(); rejected { _ = try projection.preview() }
        let collectionID = (section["collections"] as! [[String: Any]])[0]["id"] as! String
        let exportChoices: [String: Any] = ["schema":"booknook.feed-export-selection.v1", "subscriptionIds":[sourceID], "collectionIds":[collectionID], "memberships":[["subscriptionId":sourceID,"collectionIds":[collectionID]]]]
        let exportBytes = try bytes(exportChoices)
        let portable = try BookNookFeedsV1.reviewExport(configuration: currentBytes, selection: exportBytes)
        var equivalent = currentBytes; equivalent.append(contentsOf: [0x20, 0x0a])
        let portableOutput = try portable.confirm(configuration: equivalent)
        let portableObject = try object(portableOutput)
        let portablePreview = try portable.preview()
        precondition(NSDictionary(dictionary: portableObject).isEqual(to: section))
        precondition(portablePreview == portableOutput)
        _ = try BookNookFeedsV1.validateSection(Data(portableOutput.utf8))
        let empty = Data(try BookNookFeedsV1.emptyConfiguration().utf8)
        let imported = try BookNookFeedsV1.reviewImport(configuration: empty, incoming: Data(portableOutput.utf8))
        let importedSection = try object(imported.confirm(configuration: empty, incoming: Data(portableOutput.utf8)))["feeds"] as! [String: Any]
        precondition(NSDictionary(dictionary: importedSection).isEqual(to: section)); imported.close()
        rejected { _ = try portable.confirm(configuration: bytes(changed)) }
        var privateSection = section
        var privateSources = section["subscriptions"] as! [[String: Any]]
        privateSources[0]["disclosure"] = "private"; privateSection["subscriptions"] = privateSources
        rejected { _ = try BookNookFeedsV1.reviewExport(configuration: bytes(["schema":"booknook.feed-configuration.v1","revision":0,"feeds":privateSection]), selection: exportBytes) }
        let text = String(decoding: exportBytes, as: UTF8.self)
        for invalid in [text.replacingCharacters(in: text.startIndex...text.startIndex, with:"{\"origin\":\"https://reader.example.org\","), text.replacingCharacters(in: text.startIndex...text.startIndex, with:"{\"subscription\\u0049ds\":[],"), text.replacingOccurrences(of:"\"schema\":\"booknook.feed-export-selection.v1\"", with:"\"schema\":{\"booknook.feed-export-selection.v1\":null}")] {
            rejected { _ = try BookNookFeedsV1.reviewExport(configuration: currentBytes, selection: Data(invalid.utf8)) }
        }
        rejected { _ = try BookNookFeedsV1.reviewExport(configuration: currentBytes, selection: Data([0xc3,0x28])) }
        for sourceIDs in [[sourceID], []] {
            let memberships: [[String: Any]] = sourceIDs.map { ["subscriptionId":$0,"collectionIds":[]] }
            let chosen = try bytes(["schema":"booknook.feed-export-selection.v1","subscriptionIds":sourceIDs,"collectionIds":[orphan],"memberships":memberships] as [String: Any])
            let orphanReview = try BookNookFeedsV1.reviewExport(configuration: config, selection: chosen)
            let result = try orphanReview.confirm(configuration: config)
            precondition(!result.contains(orphan) && !result.contains("Private orphan"))
            let resultObject = try object(result)
            precondition((resultObject["collections"] as! [Any]).isEmpty)
            orphanReview.close()
        }
        portable.close(); rejected { _ = try portable.preview() }; rejected { _ = try portable.confirm(configuration: currentBytes) }; portable.close()
        print("Actual generated UniFFI Swift feed SDK: 102 raw vectors, literal-string enum refusals, exact-review conflicts/staleness, opaque lifecycle, origin/collection closure and origin-free export/reimport passed.")
    }
}
