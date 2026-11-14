// SPDX-License-Identifier: MPL-2.0
import Foundation

struct PublicationSnapshot: Sendable {
    let configuration: PublicationConfiguration
    let token: String
    let statuses: [String: BookNookFeedCacheStatusV1]
}
struct PublicationIndexDate: Sendable {
    let status: BookNookFeedCacheDateV1.Status
    let raw: String?
    let unixSeconds: Int64?
    let nanoseconds: UInt32?
    init(_ date: BookNookFeedCacheDateV1) {
        status = date.status; raw = date.raw.map { String($0.prefix(160)) }
        unixSeconds = date.unixSeconds; nanoseconds = date.nanoseconds
    }
}
struct PublicationIndexEntry: Identifiable, Sendable {
    let sourceID: String
    let variantKey: String
    let sourceTitle: String
    let title: String
    let authors: String
    let date: PublicationIndexDate
    let sourceOrder: Int
    let firstSourceOrdinal: UInt32
    var id: String { sourceID + ":" + variantKey }
    static func newest(_ a: Self, _ b: Self) -> Bool {
        if a.date.status == .parsed, b.date.status == .parsed {
            if a.date.unixSeconds != b.date.unixSeconds { return a.date.unixSeconds! > b.date.unixSeconds! }
            if a.date.nanoseconds != b.date.nanoseconds { return a.date.nanoseconds! > b.date.nanoseconds! }
        } else if (a.date.status == .parsed) != (b.date.status == .parsed) { return a.date.status == .parsed }
        if a.sourceOrder != b.sourceOrder { return a.sourceOrder < b.sourceOrder }
        if a.firstSourceOrdinal != b.firstSourceOrdinal { return a.firstSourceOrdinal < b.firstSourceOrdinal }
        return a.id < b.id
    }
}

/// One controller/open task per Desk-owned repository. Opaque reviews stay on
/// this actor; previews are display values and never become admission permits.
actor PublicationRepository {
    private let resolveDirectory: @Sendable () async throws -> URL
    private let provider: (any BookNookFeedSourceProviderV1)?
    private struct Opening { let id: UUID; let task: Task<BookNookFeedCacheV1, Error> }
    private var opening: Opening?
    private var controller: BookNookFeedCacheV1?
    private var closed = false
    private struct ImportSession {
        let id: UUID
        let token: String
        let incoming: Data
        let original: BookNookFeedImportReviewV1
        var resolved: BookNookFeedImportReviewV1?
    }
    private struct EditSession { let preview: PublicationEditPreview; let token: String; let raw: Data }
    private struct ExportSession { let id: UUID; let token: String; let review: BookNookFeedExportReviewV1 }
    private var importing: ImportSession?
    private var editing: EditSession?
    private var exporting: ExportSession?
    init(directory: URL, provider: (any BookNookFeedSourceProviderV1)? = nil) {
        resolveDirectory = { directory }; self.provider = provider
    }
    init(desk: DeskRepository, provider: (any BookNookFeedSourceProviderV1)? = nil) {
        resolveDirectory = { try await desk.resolvedDirectory().appendingPathComponent("publications", isDirectory: true) }
        self.provider = provider
    }
    private func cache() async throws -> BookNookFeedCacheV1 {
        guard !closed else { throw PublicationFailure.message("Publications are closed.") }
        if let controller { return controller }
        let pending: Opening
        if let opening { pending = opening }
        else {
            let resolve = resolveDirectory, provider = provider
            let task = Task {
                let directory = try await resolve()
                let database = directory.appendingPathComponent("feeds.sqlite")
                let expected: Data?
                if FileManager.default.fileExists(atPath: database.path) { expected = nil }
                else {
                    // Only initialize an absent/empty store. Preserve an unknown
                    // directory, orphan WAL or corrupt existing DB for recovery.
                    if FileManager.default.fileExists(atPath: directory.path), !(try FileManager.default.contentsOfDirectory(atPath: directory.path)).isEmpty {
                        throw PublicationFailure.message("Publication storage is incomplete or unknown. It was preserved; no empty configuration was written.")
                    }
                    expected = Data(try BookNookFeedsV1.emptyConfiguration().utf8)
                }
                if let provider { return try await BookNookFeedCacheV1.open(directory: directory, expectedConfiguration: expected, provider: provider) }
                return try await BookNookFeedCacheV1.publicHTTPS(directory: directory, expectedConfiguration: expected)
            }
            pending = .init(id: UUID(), task: task); opening = pending
        }
        do {
            let value = try await pending.task.value
            guard !closed else { throw PublicationFailure.message("Publications are closed.") }
            controller = value
            if opening?.id == pending.id { opening = nil }
            try Task.checkCancellation()
            return value
        } catch { if opening?.id == pending.id { opening = nil }; throw error }
    }
    private func typed(_ current: BookNookFeedCacheConfigurationV1) throws -> PublicationConfiguration {
        let canonical = try BookNookFeedsV1.validateConfiguration(current.raw)
        return try JSONDecoder().decode(PublicationConfiguration.self, from: Data(canonical.utf8))
    }
    func snapshot() async throws -> PublicationSnapshot {
        let cache = try await cache(), current = try await cache.configuration(), config = try typed(current)
        var statuses: [String: BookNookFeedCacheStatusV1] = [:]
        for source in config.feeds.subscriptions { statuses[source.id] = try await cache.status(sourceID: source.id) }
        guard try await cache.configuration().configurationToken == current.configurationToken else { throw PublicationFailure.message("Publication settings changed. Reload this view.") }
        return .init(configuration: config, token: current.configurationToken, statuses: statuses)
    }
    func reviewImport(_ incoming: Data) async throws -> PublicationImportPreview {
        try Task.checkCancellation()
        guard !incoming.isEmpty, incoming.count <= 262_144 else { throw PublicationFailure.message("Feed settings exceed 256 KiB or are empty.") }
        _ = try BookNookFeedsV1.validateSection(incoming)
        let current = try await cache().configuration()
        try Task.checkCancellation()
        let review = try BookNookFeedsV1.reviewImport(configuration: current.raw, incoming: incoming)
        importing?.original.close(); importing?.resolved?.close()
        let session = ImportSession(id: UUID(), token: current.configurationToken, incoming: incoming, original: review)
        importing = session
        return try preview(session)
    }
    private func preview(_ session: ImportSession) throws -> PublicationImportPreview {
        let raw = try (session.resolved ?? session.original).preview()
        guard raw.utf8.count <= 1_048_576 else { throw PublicationFailure.message("Feed review exceeds its display limit.") }
        let wire = try JSONDecoder().decode(PublicationImportWire.self, from: Data(raw.utf8))
        guard wire.schema == "booknook.feed-import-review.v1" else { throw PublicationFailure.message("Unsupported feed review.") }
        return .init(id: session.id, conflicts: wire.conflicts, candidate: wire.candidate)
    }
    func resolveImport(_ id: UUID, choices: [PublicationResolution]) async throws -> PublicationImportPreview {
        guard var session = importing, session.id == id else { throw PublicationFailure.message("This import review is closed.") }
        let current = try await cache().configuration()
        try Task.checkCancellation()
        guard current.configurationToken == session.token, importing?.id == id else { throw PublicationFailure.message("Settings changed. Start a new import review.") }
        let raw = try JSONEncoder().encode(choices)
        let resolved = try session.original.resolve(configuration: current.raw, incoming: session.incoming, resolutions: raw)
        session.resolved?.close(); session.resolved = resolved; importing = session
        return try preview(session)
    }
    /// False means canonical-equal: no CAS, no generation change, no lost activation.
    func confirmImport(_ id: UUID) async throws -> Bool {
        guard let session = importing, session.id == id else { throw PublicationFailure.message("This import review is closed.") }
        let cache = try await cache(), current = try await cache.configuration()
        guard current.configurationToken == session.token, importing?.id == id else { throw PublicationFailure.message("Settings changed. Start a new import review.") }
        let raw = Data(try (session.resolved ?? session.original).confirm(configuration: current.raw, incoming: session.incoming).utf8)
        let unchanged = try BookNookFeedsV1.validateConfiguration(raw) == BookNookFeedsV1.validateConfiguration(current.raw)
        if !unchanged { try await cache.replaceConfiguration(expectedToken: session.token, raw: raw) }
        if importing?.id == id { session.original.close(); session.resolved?.close(); importing = nil }
        return !unchanged
    }
    private func reviewEdit(_ title: String, detail: String, mutate: (inout PublicationConfiguration) throws -> Void) async throws -> PublicationEditPreview {
        let current = try await cache().configuration()
        try Task.checkCancellation()
        let previous = try typed(current)
        var value = previous; try mutate(&value)
        let changes = value != previous
        if changes {
            guard value.revision < UInt64.max else { throw PublicationFailure.message("Publication revision is exhausted.") }
            value.revision += 1
        }
        let raw = Data(try BookNookFeedsV1.validateConfiguration(JSONEncoder().encode(value)).utf8)
        let preview = PublicationEditPreview(id: UUID(), title: title, detail: detail, candidate: value, changes: changes)
        editing = .init(preview: preview, token: current.configurationToken, raw: raw)
        return preview
    }
    func reviewAdd(title: String, url: String, format: String) async throws -> PublicationEditPreview {
        try await reviewEdit("Add this publication?", detail: "It will be saved privately and inactive. This does not contact its website.") { value in
            value.feeds.subscriptions.append(.init(id: UUID().uuidString.lowercased(), url: url, format: format, title: title, enabled: true, collectionIds: [], disclosure: "private"))
        }
    }
    func reviewRemove(_ id: String) async throws -> PublicationEditPreview {
        try await reviewEdit("Remove this publication?", detail: "Its saved articles will also be removed from this device.") { value in
            guard value.feeds.subscriptions.contains(where: { $0.id == id }) else { throw PublicationFailure.message("Publication no longer exists.") }
            value.feeds.subscriptions.removeAll { $0.id == id }
        }
    }
    func reviewEligibility(_ id: String, selected: Bool) async throws -> PublicationEditPreview {
        try await reviewEdit(selected ? "Allow selected settings exports?" : "Keep these settings private?", detail: "This changes export eligibility only. An export still requires a separate selection and confirmation.") { value in
            guard let index = value.feeds.subscriptions.firstIndex(where: { $0.id == id }) else { throw PublicationFailure.message("Publication no longer exists.") }
            value.feeds.subscriptions[index].disclosure = selected ? "selected" : "private"
        }
    }
    func reviewEnabled(_ id: String, enabled: Bool) async throws -> PublicationEditPreview {
        try await reviewEdit(enabled ? "Allow this source to be enabled locally?" : "Disable this source in settings?", detail: "This changes the portable enabled setting. Saving still leaves every source paused; no website is contacted.") { value in
            guard let index = value.feeds.subscriptions.firstIndex(where: { $0.id == id }) else { throw PublicationFailure.message("Publication no longer exists.") }
            value.feeds.subscriptions[index].enabled = enabled
        }
    }
    @discardableResult func confirmEdit(_ id: UUID) async throws -> Bool {
        guard let edit = editing, edit.preview.id == id else { throw PublicationFailure.message("This settings review is closed.") }
        let cache = try await cache(), current = try await cache.configuration()
        guard current.configurationToken == edit.token, editing?.preview.id == id else { throw PublicationFailure.message("Settings changed. Review the edit again.") }
        let unchanged = try BookNookFeedsV1.validateConfiguration(edit.raw) == BookNookFeedsV1.validateConfiguration(current.raw)
        if !unchanged { try await cache.replaceConfiguration(expectedToken: edit.token, raw: edit.raw) }
        if editing?.preview.id == id { editing = nil }
        return !unchanged
    }
    func reviewExport(_ selection: PublicationExportSelection) async throws -> PublicationExportPreview {
        let current = try await cache().configuration()
        try Task.checkCancellation()
        let review = try BookNookFeedsV1.reviewExport(configuration: current.raw, selection: JSONEncoder().encode(selection))
        let raw = Data(try review.preview().utf8)
        guard raw.count <= 262_144 else { review.close(); throw PublicationFailure.message("Export exceeds its size limit.") }
        let section = try JSONDecoder().decode(PublicationSection.self, from: raw)
        exporting?.review.close(); let id = UUID()
        exporting = .init(id: id, token: current.configurationToken, review: review)
        return .init(id: id, section: section)
    }
    func confirmExport(_ id: UUID) async throws -> Data {
        guard let session = exporting, session.id == id else { throw PublicationFailure.message("This export review is closed.") }
        let current = try await cache().configuration()
        try Task.checkCancellation()
        guard current.configurationToken == session.token, exporting?.id == id else { throw PublicationFailure.message("Settings changed. Select an export again.") }
        let raw = Data(try session.review.confirm(configuration: current.raw).utf8)
        session.review.close(); exporting = nil
        return raw
    }
    func cancelReviews() {
        importing?.original.close(); importing?.resolved?.close(); exporting?.review.close()
        importing = nil; editing = nil; exporting = nil
    }
    func refresh(_ id: String, activate: Bool) async throws {
        let cache = try await cache()
        if activate { try await cache.activate(sourceID: id) }
        _ = try await cache.refresh(sourceID: id)
    }
    func pause(_ id: String) async throws { try await cache().pause(sourceID: id) }
    func offline(_ id: String) async throws -> BookNookFeedCacheViewV1 { try await cache().offline(sourceID: id) }
    func replaceProvider(_ provider: any BookNookFeedSourceProviderV1) async throws {
        let cache = try await cache(), current = try await cache.configuration()
        try await cache.replaceProvider(expectedToken: current.providerToken, provider: provider)
    }
    /// Sequential temporary full views; retain only a bounded lightweight index.
    /// Opening a view never activates or fetches, even after connectivity returns.
    func discovery() async throws -> [PublicationIndexEntry] {
        let cache = try await cache(), current = try await cache.configuration(), config = try typed(current)
        var result: [PublicationIndexEntry] = []
        for (sourceOrder, source) in config.feeds.subscriptions.enumerated() {
            try Task.checkCancellation()
            let status = try await cache.status(sourceID: source.id)
            guard status.cached else { continue }
            let view = try await cache.offline(sourceID: source.id)
            guard view.configurationToken == current.configurationToken else { throw PublicationFailure.message("Publication settings changed. Reload saved articles.") }
            result += view.entries.map { entry in
                .init(sourceID: source.id, variantKey: entry.variantKey, sourceTitle: source.title,
                      title: String((entry.title?.text ?? "Untitled article").prefix(512)), authors: String(entry.authors.joined(separator: ", ").prefix(512)), date: PublicationIndexDate(entry.published), sourceOrder: sourceOrder, firstSourceOrdinal: entry.firstSourceOrdinal)
            }
            result.sort(by: PublicationIndexEntry.newest); if result.count > 120 { result.removeLast(result.count - 120) }
        }
        guard try await cache.configuration().configurationToken == current.configurationToken else { throw PublicationFailure.message("Publication settings changed. Reload saved articles.") }
        return result
    }
    func close() async throws {
        closed = true; cancelReviews()
        let opening = self.opening, controller = self.controller
        self.controller = nil; self.opening = nil
        if let opening { opening.task.cancel(); if let value = try? await opening.task.value { try await value.close() } }
        if let controller { try await controller.close() }
    }
}
