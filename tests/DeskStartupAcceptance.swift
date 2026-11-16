// SPDX-License-Identifier: MPL-2.0
import Foundation

private enum StartupAcceptanceError: Error { case expectedFailure, gateTimedOut }
private final class StartupOpenProbe: @unchecked Sendable {
    private let lock = NSLock()
    private var count = 0
    let release = DispatchSemaphore(value: 0)
    var calls: Int { lock.lock(); defer { lock.unlock() }; return count }
    func open(_ directory: URL, _ providers: [any BookNookProvider]) throws -> BookNookServices {
        lock.lock(); count += 1; lock.unlock()
        guard release.wait(timeout: .now() + 10) == .success else { throw StartupAcceptanceError.gateTimedOut }
        return try BookNookServices(directory: directory, providers: providers)
    }
}
private final class StartupFailureProbe: @unchecked Sendable {
    private let lock = NSLock()
    private var count = 0
    var calls: Int { lock.lock(); defer { lock.unlock() }; return count }
    func open(_ directory: URL, _ providers: [any BookNookProvider]) throws -> BookNookServices {
        lock.lock(); count += 1; let current = count; lock.unlock()
        if current == 1 { throw StartupAcceptanceError.expectedFailure }
        return try BookNookServices(directory: directory, providers: providers)
    }
}

@MainActor func testDeskStartup() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("booknook-startup-" + UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: root) }
    let probe = StartupOpenProbe()
    let repository = DeskRepository(directory: root, openServices: { try probe.open($0, $1) })
    let model = DeskModel(repository: repository)
    let firstWindow = Task { await model.open() }
    let secondWindow = Task { await model.open() }
    let directConsumer = Task { try await repository.open() }
    let deadline = Date().addingTimeInterval(3)
    while probe.calls == 0, Date() < deadline { try await Task.sleep(nanoseconds: 1_000_000) }
    defer { probe.release.signal() }
    precondition(probe.calls == 1 && model.openingLibrary && !model.libraryReady && model.state == nil)
    // This actor call must complete before the native opener is released. A
    // synchronous FFI open on the actor would instead occupy it until timeout.
    let directory = try await repository.resolvedDirectory()
    precondition(directory == root && model.openingLibrary && model.state == nil)
    firstWindow.cancel()
    probe.release.signal()
    await firstWindow.value; await secondWindow.value
    _ = try await directConsumer.value
    precondition(probe.calls == 1 && model.libraryReady && !model.openingLibrary && model.error == nil)
    let result = try await model.perform(["op": "add_book", "title": "Startup continuity café 読書", "author": "Book Nook", "at": 1])
    let bookID = result!.object!["book_id"] as! String
    let revision = model.state!.revision
    await model.open()
    precondition(probe.calls == 1 && model.state!.revision == revision && model.state!.books[bookID] != nil)
    let reopened = DeskModel(repository: DeskRepository(directory: root))
    await reopened.open()
    precondition(reopened.libraryReady && reopened.error == nil && reopened.state!.books[bookID]?.title == "Startup continuity café 読書")

    let retryRoot = root.appendingPathComponent("retry")
    let failure = StartupFailureProbe()
    let retry = DeskModel(repository: DeskRepository(directory: retryRoot, openServices: { try failure.open($0, $1) }))
    await retry.open()
    precondition(!retry.libraryReady && !retry.openingLibrary && retry.state == nil && retry.libraryOpenError != nil && retry.error != nil)
    await retry.open()
    precondition(failure.calls == 2 && retry.libraryReady && retry.libraryOpenError == nil && retry.error == nil)
    // A failure after the store opens is recovery attention, not a new empty
    // library or an invitation to open a competing store. Preserve the original
    // journal path, block a successful drain, and allow the existing Retry save.
    let recoveryRoot = root.appendingPathComponent("recovery-attention")
    _ = try await DeskRepository(directory: recoveryRoot).open()
    let journal = recoveryRoot.appendingPathComponent("pending-ink-v1")
    let retained = Data("Retain this unexpected journal file".utf8)
    try retained.write(to: journal)
    let recovery = DeskModel(repository: DeskRepository(directory: recoveryRoot))
    await recovery.open()
    precondition(recovery.libraryReady && recovery.libraryOpenError == nil && recovery.saving && recovery.error != nil)
    let beforeRetry = try Data(contentsOf: journal)
    precondition(beforeRetry == retained)
    do { try await recovery.drainSaves(); preconditionFailure("Unreadable recovery must not report a completed save") }
    catch { precondition(recovery.saving) }
    let moved = recoveryRoot.appendingPathComponent("retained-journal-file")
    try FileManager.default.moveItem(at: journal, to: moved)
    recovery.retrySave()
    let retryDeadline = Date().addingTimeInterval(3)
    while recovery.saving, Date() < retryDeadline { try await Task.sleep(nanoseconds: 1_000_000) }
    precondition(!recovery.saving && recovery.error == nil && recovery.libraryReady)
    let afterRetry = try Data(contentsOf: moved)
    precondition(afterRetry == retained)
    print("Desk startup: held native open leaves actor/UI available, concurrent windows/consumer share one open, canceled waiter cannot start a replacement, saved Unicode survives reopen, completed failure permits one explicit retry, and unreadable recovery retains its original, blocks save completion and supports recovery retry.")
}
