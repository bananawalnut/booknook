// SPDX-License-Identifier: MPL-2.0
// Workflow tests use an in-memory store and injected validation. Actual cryptographic
// compatibility and retained live custody are tested by native/recovery_validation.rs.
import Foundation
import Darwin
import Dispatch

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
/// A blocking-open regression must fail within the watchdog instead of wedging CI.
private final class RecoveryFIFODeadline: @unchecked Sendable {
    private let lock = NSLock()
    private var exceeded = false
    private let timer = DispatchSource.makeTimerSource(queue: .global(qos: .userInitiated))
    init(_ fifo: URL) {
        timer.schedule(deadline: .now() + .seconds(2), repeating: .milliseconds(10))
        timer.setEventHandler { [weak self] in
            guard let self else { return }
            self.lock.lock(); self.exceeded = true; self.lock.unlock()
            let writer = Darwin.open(fifo.path, O_WRONLY | O_NONBLOCK | O_CLOEXEC)
            if writer >= 0 { _ = Darwin.close(writer) }
        }
        timer.resume()
    }
    func finish() -> Bool { timer.cancel(); lock.lock(); defer { lock.unlock() }; return exceeded }
    deinit { timer.cancel() }
}

private final class RecoveryReadCancellationGate: RecoveryMaterialStorage, @unchecked Sendable {
    private let lock = NSLock()
    private let release = DispatchSemaphore(value: 0)
    private var entered = false
    private var reads = 0
    func factory() -> any RecoveryMaterialStorage {
        lock.lock(); entered = true; lock.unlock()
        release.wait()
        return self
    }
    var hasEntered: Bool { lock.lock(); defer { lock.unlock() }; return entered }
    var readCount: Int { lock.lock(); defer { lock.unlock() }; return reads }
    func resume() { release.signal() }
    func read(account: String) throws -> Data? { lock.lock(); defer { lock.unlock() }; reads += 1; return nil }
    func saveEncryptedMaterial(_ data: Data, account: String) throws { preconditionFailure("File staging cannot save recovery material") }
}

private func testRecoveryFileBoundary() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("booknook-recovery-file-" + UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let saved = Data("previous synthetic recovery".utf8)
    let store = MemoryRecoveryStorage(saved)
    func rejected(_ url: URL) {
        do { _ = try RecoveryImportCandidate.readFile(url, storage: store); preconditionFailure("Expected recovery file refusal") }
        catch { }
        precondition(store.value == saved && store.writes == 0)
    }
    let fifo = root.appendingPathComponent("no-writer.castalia-recovery")
    precondition(mkfifo(fifo.path, S_IRUSR | S_IWUSR) == 0)
    let deadline = RecoveryFIFODeadline(fifo)
    rejected(fifo)
    precondition(!deadline.finish(), "Recovery reader blocked on a FIFO until the watchdog supplied a writer")
    let fifoLink = root.appendingPathComponent("fifo-link.castalia-recovery")
    try FileManager.default.createSymbolicLink(at: fifoLink, withDestinationURL: fifo)
    rejected(fifoLink)
    rejected(root)
    rejected(URL(fileURLWithPath: "/dev/null"))
    rejected(URL(string: "https://example.invalid/recovery")!)
    rejected(URL(string: "file://remote.example.invalid/recovery")!)
    rejected(URL(string: "file:///tmp/recovery%00suffix")!)
    rejected(root.appendingPathComponent("missing"))
    let file = root.appendingPathComponent("regular.castalia-recovery")
    for invalid in [Data(), Data("{}".utf8), Data([0xff, 0xfe]), Data(repeating: 32, count: RecoveryImportCandidate.maximumBytes + 1)] {
        try invalid.write(to: file); rejected(file)
    }
    let unicode = Data(#"{"schema":"castalia.wallet-custody.v1","ciphertext":"é😀 café 日本語"}"#.utf8)
    var exact = unicode
    exact.append(Data(repeating: 32, count: RecoveryImportCandidate.maximumBytes - exact.count))
    try exact.write(to: file)
    let full = try RecoveryImportCandidate.readFile(file, storage: store)
    precondition(full.data == exact && full.previous == saved && full.filename == file.lastPathComponent)
    try unicode.write(to: file)
    let symlink = root.appendingPathComponent("regular-link.castalia-recovery")
    try FileManager.default.createSymbolicLink(at: symlink, withDestinationURL: file)
    let linked = try RecoveryImportCandidate.readFile(symlink, storage: store)
    precondition(linked.data == unicode && linked.filename == symlink.lastPathComponent)
    let asyncCandidate = try await readRecoveryImportCandidate(file) { MemoryRecoveryStorage(saved) }
    precondition(asyncCandidate.data == unicode && asyncCandidate.previous == saved)
    precondition(store.value == saved && store.writes == 0)
    // The detached call has genuinely started. Hold its storage construction,
    // cancel the parent, then release it: the native read must observe forwarded
    // cancellation before opening the file or consulting the secure-store API.
    let gate = RecoveryReadCancellationGate()
    let running = Task { try await readRecoveryImportCandidate(file) { gate.factory() } }
    let deadlineTime = Date().addingTimeInterval(2)
    while !gate.hasEntered { precondition(Date() < deadlineTime); await Task.yield() }
    running.cancel(); gate.resume()
    do { _ = try await running.value; preconditionFailure("Cancelled running recovery read succeeded") }
    catch is CancellationError { }
    precondition(gate.readCount == 0, "Parent cancellation was not forwarded to the detached SDK reader")
    let cancelled = Task {
        withUnsafeCurrentTask { $0?.cancel() }
        do {
            _ = try RecoveryImportCandidate.readFile(URL(string: "https://example.invalid/cancel")!, storage: store)
            preconditionFailure("Pre-cancelled recovery read succeeded")
        } catch is CancellationError { }
        catch { preconditionFailure("Cancellation must win before URL/I/O handling") }
        do {
            _ = try await readRecoveryImportCandidate(URL(string: "https://example.invalid/cancel")!) { preconditionFailure("Cancelled async read consulted storage") }
            preconditionFailure("Pre-cancelled async recovery read succeeded")
        } catch is CancellationError { }
        catch { preconditionFailure("Cancelled async read must preserve cancellation") }
    }
    await cancelled.value
    print("Recovery actual-file boundary: no-writer FIFO/watchdog, FIFO/regular symlinks, directory/device/nonlocal/missing/empty/envelope/UTF8/size refusal, exact 1 MiB Unicode, asynchronous staging, running-parent cancellation forwarding and pre-cancellation passed; no secure-store writes.")
}

@main struct RecoveryImportTests {
    @MainActor static func main() async throws {
        try await testRecoveryFileBoundary()
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
