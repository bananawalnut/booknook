// SPDX-License-Identifier: MPL-2.0
// Workflow tests use an in-memory store and injected validation. Actual cryptographic
// compatibility and retained live custody are tested by native/recovery_validation.rs.
import Foundation

private final class MemoryRecoveryStorage: RecoveryMaterialStorage {
    var value: Data?
    var writes = 0
    var failWrites = false
    init(_ value: Data?) { self.value = value }
    func read(account: String) throws -> Data? { value }
    func saveEncryptedMaterial(_ data: Data, account: String) throws {
        if failWrites { throw RecoveryImportError.message("Synthetic secure-store failure") }
        value = data; writes += 1
    }
}
@MainActor private final class ValidationGate {
    var continuation: CheckedContinuation<String, Error>?
    func wait() async throws -> String { try await withCheckedThrowingContinuation { continuation = $0 } }
    func finish(_ key: String) { continuation?.resume(returning: key); continuation = nil }
}
@main struct RecoveryImportTests {
    @MainActor static func main() async throws {
        let old = Data(#"{"schema":"castalia.wallet-custody.v1","ciphertext":"old synthetic envelope"}"#.utf8)
        let candidateBytes = Data(#"{"schema":"castalia.wallet-custody.v1","ciphertext":"candidate synthetic envelope"}"#.utf8)
        let expected = String(repeating: "a", count: 64)
        let other = String(repeating: "b", count: 64)
        func candidate(_ storage: MemoryRecoveryStorage) throws -> RecoveryImportCandidate {
            try RecoveryImportCandidate(data: candidateBytes, filename: "candidate.castalia-recovery", storage: storage)
        }
        func session(_ storage: MemoryRecoveryStorage, holder: String? = nil, validate: @escaping (Data, String) async throws -> String) throws -> RecoveryImportSession {
            try RecoveryImportSession(candidate: candidate(storage), storage: storage, configuredHolder: { holder }, validate: validate)
        }
        func fails(_ action: () async throws -> String) async {
            do { _ = try await action(); preconditionFailure("Expected failure") } catch { }
        }
        let store = MemoryRecoveryStorage(old)
        let staged = try candidate(store)
        precondition(staged.replacesSavedFile && store.value == old && store.writes == 0)
        do { _ = try RecoveryImportCandidate(data: Data("{}".utf8), filename: "invalid", storage: store); preconditionFailure("Invalid staging succeeded") } catch { }
        precondition(store.value == old)

        let wrongPassphrase = try session(store, holder: expected) { _, _ in throw RecoveryImportError.message("Synthetic invalid passphrase") }
        wrongPassphrase.passphrase = "wrong"
        await fails { try await wrongPassphrase.validateAndSave() }
        precondition(store.value == old && wrongPassphrase.passphrase.isEmpty && !wrongPassphrase.validating)

        let mismatch = try session(store, holder: expected) { _, _ in other }
        mismatch.passphrase = "synthetic"
        await fails { try await mismatch.validateAndSave() }
        precondition(store.value == old && store.writes == 0 && mismatch.passphrase.isEmpty)

        let failedWrite = try session(store, holder: expected) { _, _ in expected }
        failedWrite.passphrase = "synthetic"; store.failWrites = true
        await fails { try await failedWrite.validateAndSave() }
        precondition(store.value == old && store.writes == 0 && failedWrite.passphrase.isEmpty)
        store.failWrites = false

        var validationCalled = false
        let cancelled = try session(store) { _, _ in validationCalled = true; return expected }
        cancelled.passphrase = "synthetic"; cancelled.cancel()
        await fails { try await cancelled.validateAndSave() }
        precondition(!validationCalled && store.value == old && cancelled.passphrase.isEmpty)

        let gate = ValidationGate()
        let duringValidation = try session(store, holder: expected) { _, _ in try await gate.wait() }
        duringValidation.passphrase = "synthetic"
        let running = Task { try await duringValidation.validateAndSave() }
        while gate.continuation == nil { await Task.yield() }
        duringValidation.cancel(); gate.finish(expected)
        await fails { try await running.value }
        precondition(store.value == old && store.writes == 0 && duringValidation.passphrase.isEmpty)

        let settingsGate = ValidationGate()
        var settingsReads = 0
        let taskCancelled = RecoveryImportSession(candidate: try candidate(store), storage: store, configuredHolder: {
            settingsReads += 1
            return settingsReads == 1 ? expected : try await settingsGate.wait()
        }, validate: { _, _ in expected })
        taskCancelled.passphrase = "synthetic"
        let cancelledTask = Task { try await taskCancelled.validateAndSave() }
        while settingsGate.continuation == nil { await Task.yield() }
        cancelledTask.cancel(); settingsGate.finish(expected)
        await fails { try await cancelledTask.value }
        precondition(store.value == old && store.writes == 0 && taskCancelled.passphrase.isEmpty)

        var holderReads = 0
        let changedSettings = RecoveryImportSession(candidate: try candidate(store), storage: store, configuredHolder: {
            holderReads += 1; return holderReads == 1 ? expected : other
        }, validate: { _, _ in expected })
        changedSettings.passphrase = "synthetic"
        await fails { try await changedSettings.validateAndSave() }
        precondition(store.value == old && store.writes == 0)

        let changedStore = try session(store, holder: expected) { _, _ in store.value = Data("replacement from another window".utf8); return expected }
        changedStore.passphrase = "synthetic"
        await fails { try await changedStore.validateAndSave() }
        precondition(store.value == Data("replacement from another window".utf8) && store.writes == 0)
        store.value = old

        let valid = try session(store, holder: expected) { bytes, passphrase in
            precondition(bytes == candidateBytes && passphrase == "synthetic"); return expected
        }
        valid.passphrase = "synthetic"
        let imported = try await valid.validateAndSave()
        precondition(imported == expected && store.value == candidateBytes && store.writes == 1 && valid.passphrase.isEmpty && !valid.validating)
        await fails { try await valid.validateAndSave() }

        precondition(!SyncPrerequisites(configured: false, recoveryStored: false, expectedHolder: nil, unlockedHolder: nil).canApprove)
        precondition(!SyncPrerequisites(configured: true, recoveryStored: true, expectedHolder: expected, unlockedHolder: other).canApprove)
        precondition(!SyncPrerequisites(configured: true, recoveryStored: true, expectedHolder: expected, unlockedHolder: nil).canApprove)
        precondition(SyncPrerequisites(configured: true, recoveryStored: true, expectedHolder: expected, unlockedHolder: expected).canApprove)
        print("Recovery workflow: staging, invalid file/passphrase, holder mismatch, secure-store failure, cancellation before/during validation and settings reread, changed settings/storage, deliberate replacement and sync prerequisites passed (in-memory store; no live Keychain).")
    }
}
