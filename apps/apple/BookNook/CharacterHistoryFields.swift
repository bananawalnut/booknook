// SPDX-License-Identifier: MPL-2.0
import Foundation

struct CharacterHistoryField: Identifiable {
    let title: String
    let values: [String]
    var id: String { title }
}

/// Complete, untruncated readable projection of a saved interpretation.
func characterHistoryFields(_ value: Interpretation, nameForCharacter: (String) -> String) -> [CharacterHistoryField] {
    func field(_ title: String, _ values: [String]) -> CharacterHistoryField {
        CharacterHistoryField(title: title, values: values.isEmpty ? ["None saved"] : values)
    }
    return [
        field("Description", value.description.isEmpty ? [] : [value.description]),
        field("Traits", value.traits),
        field("Relationships", value.relationships.map { "\(nameForCharacter($0.target)): \($0.description)" }),
        field("Observations", value.observations),
        field("Open questions", value.questions),
        field("Supporting passages", value.passages.map(characterHistoryPassage))
    ]
}

private func characterHistoryPassage(_ value: JSONValue) -> String {
    guard let location = value.object else { return "Saved source reference" }
    switch location["kind"] as? String {
    case "text":
        let quote = location["quote"] as? String ?? "Saved passage"
        return [quote, location["resource"] as? String].compactMap { $0 }.joined(separator: "\n")
    case "page": return "PDF page \((location["page"] as? NSNumber)?.stringValue ?? "—")"
    case "audio":
        let seconds = ((location["milliseconds"] as? NSNumber)?.intValue ?? 0) / 1000
        let time = "Audio \(seconds / 3600):\(String(format: "%02d", (seconds / 60) % 60)):\(String(format: "%02d", seconds % 60))"
        return [time, location["chapter"] as? String].compactMap { $0 }.joined(separator: " · ")
    default: return "\(location["provider"] as? String ?? "Source"): \(location["value"] as? String ?? "Saved location")"
    }
}

/// The reviewed parent set is fixed when the editor opens. A later incoming
/// interpretation remains a conflict until it is separately reviewed.
struct CharacterReconciliation {
    let parents: [String]
    private(set) var startingVersion: String?
    init(parents: [String]) {
        self.parents = parents
        startingVersion = parents.count == 1 ? parents.first : nil
    }
    var canSave: Bool { startingVersion.map { parents.contains($0) } ?? false }
    @discardableResult mutating func select(_ id: String) -> Bool {
        guard parents.contains(id) else { return false }
        startingVersion = id
        return true
    }
}

func reconciliationPassages(_ interpretations: [Interpretation]) -> [JSONValue] {
    var seen = Set<Data>()
    return interpretations.flatMap(\.passages).filter { passage in
        guard let key = try? JSONSerialization.data(withJSONObject: passage.any, options: [.sortedKeys, .fragmentsAllowed]) else { return true }
        return seen.insert(key).inserted
    }
}
