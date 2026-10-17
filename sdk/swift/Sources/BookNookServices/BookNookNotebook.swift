// SPDX-License-Identifier: MIT OR Apache-2.0
import Foundation
#if canImport(BookNookNative)
import BookNookNative
#endif

/// Host-owned portable notebook state, initialized before opening BookNookNotebook.
/// All methods must be thread-safe. A synchronous successful compareAndSave commits
/// durably and atomically with every other writer. False means conflict; throwing
/// means storage failure. Neither failure may partially replace the stored state.
/// This port receives private notebook state; it is distinct from source providers.
public protocol BookNookNotebookStorage: Sendable {
    func descriptorJSON() throws -> String
    func loadJSON() throws -> String
    func compareAndSave(expectedRevision: UInt64, stateJSON: String) throws -> Bool
}

/// The shared Rust command engine with replaceable standalone state storage.
/// Run synchronous storage work off the UI actor. Each call loads fresh state and
/// checks availability; a rejected save is never retried or returned as success.
/// The reference app keeps Desk's composed SQLite/journal path for editing, assets,
/// settings and social state. This object does not replace that composition.
public final class BookNookNotebook: Sendable {
    private let native: NotebookEngine

    public init(storage: any BookNookNotebookStorage) throws {
        native = try NotebookEngine.withStorage(host: NotebookHost(storage: storage))
    }
    /// Use an isolated notebook directory, not a directory shared with an active Desk.
    public init(localDirectory: URL) throws {
        native = try NotebookEngine.local(directory: localDirectory.path)
    }
    public func snapshotJSON() throws -> String { try native.snapshot() }
    public func execute(commandJSON: String) throws -> String { try native.execute(commandJson: commandJSON) }
}

private final class NotebookHost: NotebookStorageHost, Sendable {
    let storage: any BookNookNotebookStorage
    init(storage: any BookNookNotebookStorage) { self.storage = storage }
    func descriptorJson() throws -> String {
        try bridge { try storage.descriptorJSON() }
    }
    func loadJson() throws -> String { try bridge { try storage.loadJSON() } }
    func compareAndSave(expectedRevision: UInt64, stateJson: String) throws -> Bool {
        try bridge { try storage.compareAndSave(expectedRevision: expectedRevision, stateJSON: stateJson) }
    }
    private func bridge<T>(_ operation: () throws -> T) throws -> T {
        do { return try operation() }
        catch let error as NativeError { throw error }
        catch { throw NativeError.Failure(message: error.localizedDescription) }
    }
}
