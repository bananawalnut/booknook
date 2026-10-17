// SPDX-License-Identifier: MPL-2.0
import SwiftUI

/// A draft belongs to its sheet until persistence explicitly acknowledges it.
/// Success callbacks may clear or dismiss the draft; failures never call them.
@MainActor final class DraftCommitState: ObservableObject {
    @Published private(set) var isSaving = false
    @Published private(set) var error: String?

    func clearError() { if !isSaving { error = nil } }

    func perform(_ operation: () async throws -> Void, onSuccess: () -> Void) async {
        guard !isSaving else { return }
        isSaving = true
        error = nil
        defer { isSaving = false }
        do {
            try await operation()
            onSuccess()
        } catch {
            self.error = "Could not save. Your draft is still here. \(bookNookErrorMessage(error))"
        }
    }
}
