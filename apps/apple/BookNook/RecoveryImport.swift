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
