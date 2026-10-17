// SPDX-License-Identifier: MPL-2.0
import Foundation
import Darwin

/// Private recovery envelope, deliberately excluded from portable exports. Each
/// capture is immutable; its ID and exact bytes also identify acknowledgement.
struct InkJournalRecord: Codable {
    let schema: String
    let id: String
    let createdAt: Double
    let kind: String
    let payload: JSONValue
    init(id: String, kind: String, payload: [String: Any]) throws {
        self.schema = "booknook.private-ink-journal.v1"; self.id = id
        self.createdAt = Date().timeIntervalSince1970; self.kind = kind
        self.payload = try JSONDecoder().decode(JSONValue.self, from: JSONSerialization.data(withJSONObject: payload))
    }
    func encoded() throws -> Data {
        guard schema == "booknook.private-ink-journal.v1", UUID(uuidString: id) != nil,
              createdAt.isFinite, createdAt >= 0, ["notebook", "reader"].contains(kind), payload.object != nil else {
            throw InkJournalFailure("A pending ink record has an unsupported or damaged format. Its file was retained.")
        }
        let encoder = JSONEncoder(); encoder.outputFormatting = [.sortedKeys]
        let data = try encoder.encode(self)
        guard data.count <= InkJournal.maximumRecordBytes else { throw InkJournalFailure("The pending ink capture exceeds the local recovery limit.") }
        return data
    }
}
struct InkJournalFailure: LocalizedError {
    let message: String
    init(_ message: String) { self.message = message }
    var errorDescription: String? { message }
}

/// All blocking filesystem work runs on this actor, never in a drawing callback.
/// A record is acknowledged only after file + directory fsync. This proves crash
/// recovery after acknowledgement, not survival of an unacknowledged OS write or
/// power loss. No records are silently repaired, truncated, or discarded.
actor InkJournal {
    static let maximumRecordBytes = 128 * 1024 * 1024
    let directory: URL
    #if BOOKNOOK_TEST
    private var failNextDirectoryBarrier = false
    func simulateInterruptedDirectoryBarrier() { failNextDirectoryBarrier = true }
    #endif
    init(deskDirectory: URL) { directory = deskDirectory.appendingPathComponent("pending-ink-v1", isDirectory: true) }
    private func prepare() throws {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true,
                                                attributes: [.posixPermissions: 0o700])
        let values = try directory.resourceValues(forKeys: [.isDirectoryKey, .isSymbolicLinkKey])
        guard values.isDirectory == true, values.isSymbolicLink != true else { throw InkJournalFailure("The ink recovery directory is not a private directory.") }
    }
    private func url(_ id: String) throws -> URL {
        guard UUID(uuidString: id) != nil else { throw InkJournalFailure("The pending ink identifier is invalid.") }
        return directory.appendingPathComponent(id + ".json")
    }
    private func read(_ url: URL) throws -> Data {
        let values = try url.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey, .fileSizeKey])
        guard values.isRegularFile == true, values.isSymbolicLink != true,
              let size = values.fileSize, size <= Self.maximumRecordBytes else { throw InkJournalFailure("A pending ink file is not a bounded regular file. It was retained.") }
        return try Data(contentsOf: url)
    }
    func load() throws -> ([InkJournalRecord], [String]) {
        try prepare()
        let candidates = try FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: [.isRegularFileKey])
        let files = candidates.filter { $0.pathExtension == "json" || $0.pathExtension == "tmp" }
        guard files.count <= 10_000 else { throw InkJournalFailure("Too many pending ink records to recover safely. All files were retained.") }
        var records: [InkJournalRecord] = [], errors: [String] = []
        for file in files {
            do {
                let record = try JSONDecoder().decode(InkJournalRecord.self, from: read(file))
                _ = try record.encoded()
                if file.pathExtension == "tmp" {
                    // A process may die after writing/fsync but before publication.
                    // Promote only a fully decoded, bounded original; partial files
                    // remain visible recovery errors, never silently deleted.
                    try store(record)
                    try FileManager.default.removeItem(at: file); try syncDirectory()
                } else if file.lastPathComponent != record.id + ".json" {
                    throw InkJournalFailure("A pending ink filename does not match its capture. It was retained.")
                }
                if let existing = records.first(where: { $0.id == record.id }) {
                    guard try existing.encoded() == record.encoded() else { throw InkJournalFailure("Conflicting ink recovery originals were retained.") }
                } else { records.append(record) }
            } catch { errors.append("A pending ink record could not be recovered; its original file remains on this device. \(error.localizedDescription)") }
        }
        // Parent/capture links, rather than wall-clock order, preserve a queue
        // even if the system clock changes between completed strokes.
        var remaining = records.sorted { ($0.createdAt, $0.id) < ($1.createdAt, $1.id) }, ordered: [InkJournalRecord] = []
        while !remaining.isEmpty {
            let ids = Set(remaining.map(\.id))
            guard let index = remaining.firstIndex(where: { record in
                let payload = record.payload.object ?? [:]
                let dependencies = record.kind == "notebook"
                    ? ((payload["version"] as? [String: Any])?["parents"] as? [String] ?? [])
                    : (payload["previous_capture_id"] as? String).map { [$0] } ?? []
                return dependencies.allSatisfy { !ids.contains($0) }
            }) else {
                errors.append("Pending ink has cyclic capture dependencies. Its original files were retained."); break
            }
            ordered.append(remaining.remove(at: index))
        }
        return (ordered, errors)
    }
    func store(_ record: InkJournalRecord) throws {
        try prepare(); let destination = try url(record.id), bytes = try record.encoded()
        if FileManager.default.fileExists(atPath: destination.path) {
            guard try read(destination) == bytes else { throw InkJournalFailure("A different pending capture already uses this ID. Neither record was replaced.") }
            let descriptor = Darwin.open(destination.path, O_RDONLY | O_NOFOLLOW)
            guard descriptor >= 0 else { throw posixError() }
            defer { Darwin.close(descriptor) }
            guard fsync(descriptor) == 0 else { throw posixError() }
            try syncDirectory()
            return
        }
        let temporary = directory.appendingPathComponent(".\(UUID().uuidString).tmp")
        let descriptor = Darwin.open(temporary.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, 0o600)
        guard descriptor >= 0 else { throw posixError() }
        defer { Darwin.close(descriptor); try? FileManager.default.removeItem(at: temporary) }
        try bytes.withUnsafeBytes { buffer in
            var offset = 0
            while offset < buffer.count {
                let written = Darwin.write(descriptor, buffer.baseAddress!.advanced(by: offset), buffer.count - offset)
                if written < 0 && errno == EINTR { continue }
                guard written > 0 else { throw posixError() }; offset += written
            }
        }
        guard fsync(descriptor) == 0 else { throw posixError() }
        // A no-replace hard link publishes a complete inode atomically. Unlike
        // rename-overwrite it cannot destroy a newer capture from another window.
        if Darwin.link(temporary.path, destination.path) != 0 {
            guard errno == EEXIST, try read(destination) == bytes else { throw posixError() }
        }
        try syncDirectory()
    }
    func remove(_ record: InkJournalRecord) throws {
        let destination = try url(record.id)
        guard FileManager.default.fileExists(atPath: destination.path) else { return }
        guard try read(destination) == record.encoded() else { throw InkJournalFailure("A changed pending capture was retained instead of acknowledging stale data.") }
        guard Darwin.unlink(destination.path) == 0 else { throw posixError() }
        try syncDirectory()
    }
    private func syncDirectory() throws {
        #if BOOKNOOK_TEST
        if failNextDirectoryBarrier { failNextDirectoryBarrier = false; throw InkJournalFailure("Synthetic interrupted directory barrier") }
        #endif
        let descriptor = Darwin.open(directory.path, O_RDONLY | O_NOFOLLOW)
        guard descriptor >= 0 else { throw posixError() }
        defer { Darwin.close(descriptor) }
        guard fsync(descriptor) == 0 else { throw posixError() }
    }
    private func posixError() -> Error { NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
}
