// SPDX-License-Identifier: MPL-2.0
// Isolated projections keep this test independent of the whole app and UniFFI.
// Integrated app builds verify the production domain types used by these helpers.
import Foundation
struct JSONValue { let object: [String: Any]?; var any: Any { object as Any? ?? NSNull() } }
struct CharacterRelationship { let target: String; let description: String }
struct Interpretation {
    var description = ""
    var traits: [String] = []
    var relationships: [CharacterRelationship] = []
    var observations: [String] = []
    var questions: [String] = []
    var passages: [JSONValue] = []
}
func bookNookErrorMessage(_ error: Error) -> String { error.localizedDescription }

@main struct NotebookWorkflowTests {
    @MainActor static func main() async {
        let commit = DraftCommitState()
        var draft = "Mara's earlier question: café 日本語", dismissed = false, submissions = 0
        await commit.perform({
            submissions += 1
            precondition(commit.isSaving && commit.error == nil)
            // A second tap while persistence is pending must not run another write.
            await commit.perform({ submissions += 100 }, onSuccess: { dismissed = true })
            throw NSError(domain: "BookNook.synthetic-rejection", code: 1, userInfo: [NSLocalizedDescriptionKey: "Version was rejected"])
        }, onSuccess: { draft = ""; dismissed = true })
        precondition(!commit.isSaving && !dismissed && submissions == 1)
        precondition(draft == "Mara's earlier question: café 日本語")
        precondition(commit.error?.contains("Version was rejected") == true)
        await commit.perform({
            precondition(commit.isSaving && commit.error == nil)
            submissions += 1
        }, onSuccess: { draft = ""; dismissed = true })
        precondition(!commit.isSaving && commit.error == nil && dismissed && draft.isEmpty && submissions == 2)

        let quote = String(repeating: "A passage about café 日本語 and changing impressions. ", count: 12)
        var original = Interpretation()
        original.description = "Reserved at first"
        original.traits = ["Observant", "Careful"]
        original.relationships = [.init(target: "librarian", description: "Uncertain trust")]
        original.observations = ["Waits by the door"]
        original.questions = ["Why did she return?"]
        original.passages = [.init(object: ["kind": "text", "quote": quote, "resource": "chapter-3.xhtml"])]
        let fields = characterHistoryFields(original, nameForCharacter: { $0 == "librarian" ? "The librarian" : $0 })
        precondition(fields.map(\.title) == ["Description", "Traits", "Relationships", "Observations", "Open questions", "Supporting passages"])
        precondition(fields[2].values == ["The librarian: Uncertain trust"])
        precondition(fields[3].values == original.observations && fields[4].values == original.questions)
        precondition(fields[5].values == [quote + "\nchapter-3.xhtml"], "History must not truncate a supporting passage")
        var newer = original; newer.questions = ["What changed?"]
        _ = characterHistoryFields(newer, nameForCharacter: { $0 })
        precondition(fields[4].values == ["Why did she return?"], "An earlier interpretation remains independently readable")
        precondition(characterHistoryFields(Interpretation(), nameForCharacter: { $0 }).allSatisfy { $0.values == ["None saved"] })
        var review = CharacterReconciliation(parents: ["left", "right"])
        precondition(!review.canSave && review.startingVersion == nil, "A conflict must never choose a starting interpretation silently")
        precondition(!review.select("unseen") && !review.canSave)
        precondition(review.select("left") && review.canSave)
        precondition(review.parents == ["left", "right"] && !review.parents.contains("arrived-later"), "Only explicitly reviewed sources can be reconciled")
        precondition(CharacterReconciliation(parents: ["single"]).canSave)
        var alternate = original
        alternate.passages = [original.passages[0], .init(object: ["kind": "text", "quote": "Other reader's evidence", "resource": "chapter-12.xhtml"])]
        let allPassages = reconciliationPassages([original, alternate])
        precondition(allPassages.count == 2)
        precondition(allPassages.contains { $0.object?["quote"] as? String == quote })
        precondition(allPassages.contains { $0.object?["quote"] as? String == "Other reader's evidence" })
        print("Notebook workflows: failed-save draft retention, busy duplicate suppression, retry acknowledgement, all character-history fields, untruncated Unicode passages, and independent earlier versions, explicit conflict starting-version choice, reviewed-parent isolation and all-source passage preservation passed.")
    }
}
