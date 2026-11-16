// SPDX-License-Identifier: MPL-2.0
import Foundation
#if canImport(CastaliaNative)
import CastaliaNative
#endif

// The reference app consumes the public custody/recovery workflow unchanged.
typealias RecoveryMaterialStorage = CastaliaSecureMaterialStorage
typealias RecoveryImportCandidate = CastaliaRecoveryCandidate
typealias RecoveryImportError = CastaliaRecoveryError
typealias RecoveryImportSession = CastaliaRecoveryImportSession

/// Keep selected-file I/O off the UI actor; a cancelled read cannot publish a
/// late candidate. The public SDK still owns descriptor/envelope validation.
func readRecoveryImportCandidate(_ url: URL,
    storage: @escaping @Sendable () -> any RecoveryMaterialStorage) async throws -> RecoveryImportCandidate {
    try Task.checkCancellation()
    let reader = Task.detached(priority: .userInitiated) {
        try Task.checkCancellation()
        return try RecoveryImportCandidate.readFile(url, storage: storage())
    }
    return try await withTaskCancellationHandler {
        let candidate = try await reader.value
        try Task.checkCancellation()
        return candidate
    } onCancel: { reader.cancel() }
}

struct SyncPairingDraft {
    var deviceID = ""
    var label = ""
    var secret = ""
    var selectedPeer = ""
}

struct SyncPrerequisites {
    let configured: Bool
    let recoveryStored: Bool
    let expectedHolder: String?
    let unlockedHolder: String?
    var identityUnlocked: Bool { expectedHolder != nil && expectedHolder == unlockedHolder }
    var canApprove: Bool { configured && recoveryStored && identityUnlocked }
}
