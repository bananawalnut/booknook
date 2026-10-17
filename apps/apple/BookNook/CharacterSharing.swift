// SPDX-License-Identifier: MPL-2.0
import Foundation

@MainActor extension DeskModel {
    /// Native core creates an independent, deliberately selected version snapshot.
    /// The caller may navigate only after this durable draft is acknowledged.
    func prepareCharacterSharing(cardID: String, versionID: String) async throws -> ShareDraft {
        try await drainSaves()
        let result = try await perform(["op": "prepare_character_share", "card_id": cardID,
            "version_id": versionID, "at": Int(Date().timeIntervalSince1970)])
        guard let id = result?.object?["draft_id"] as? String,
              let draft = state?.drafts[id], draft.kind == "selected_note" else {
            throw AuthorizationError.message("The private version draft could not be opened. Your character history is still here.")
        }
        return draft
    }
}
