// SPDX-License-Identifier: MPL-2.0
import Foundation
import SwiftUI

struct Edition: Decodable, Identifiable { let id: String; let bookId: String; let assets: [Asset] }
struct ISBNHistoryScope: Decodable, Identifiable {
    let editionId: String
    let namespace: String
    let bookTitle: String
    var id: [String] { [editionId, namespace] }
    var label: String { "\(bookTitle) · \(namespace == "isbn13" ? "ISBN-13" : "ISBN")" }
}
struct ISBNReview: Decodable, Identifiable {
    let editionId: String
    let namespace: String
    let heads: [String]
    let choices: [[String]]
    let provisionalValues: [String]
    var id: [String] { [editionId, namespace] }
}
extension Array where Element == String {
    // Source identifier values retain their exact UTF-8 spelling in the portable history.
    var isbnValueBytes: [Data] { map { Data($0.utf8) } }
}
struct ISBNHistory: Decodable {
    struct Node: Decodable {
        let kind: String
        let values: [String]
    }
    let editionId: String
    let namespace: String
    let heads: [String]
    let nodes: [String: Node]
    let legacyAbsenceUnknown: Bool

    var activeChoices: [[String]] {
        var seen = Set<[Data]>()
        return heads.compactMap { nodes[$0]?.values }
            .filter { seen.insert($0.isbnValueBytes).inserted }
            .sorted { left, right in
                left.lexicographicallyPrecedes(right) { $0.utf8.lexicographicallyPrecedes($1.utf8) }
            }
    }

    // A compatibility anchor can be the only record of an older catalog value.
    var earlierChoices: [String] {
        let headIDs = Set(heads)
        let active = Set(activeChoices.map(\.isbnValueBytes))
        var offered = Set<[Data]>()
        let ranked = nodes.sorted { left, right in
            let leftAnchor = left.value.kind == "value_anchor"
            let rightAnchor = right.value.kind == "value_anchor"
            if leftAnchor != rightAnchor { return !leftAnchor }
            return left.key < right.key
        }
        return ranked.compactMap { id, node in
            let bytes = node.values.isbnValueBytes
            guard !headIDs.contains(id), !active.contains(bytes),
                  ["import", "review", "reassert", "value_anchor"].contains(node.kind),
                  offered.insert(bytes).inserted else { return nil }
            return id
        }
    }
}
enum ISBNDecision {
    case review(ISBNReview, [String])
    case reassert(ISBNHistory, String)

    var command: [String: Any] {
        switch self {
        case .review(let review, let values):
            return ["op": "review_isbn", "edition_id": review.editionId, "namespace": review.namespace,
                    "expected_heads": review.heads, "values": values]
        case .reassert(let history, let node):
            return ["op": "reassert_isbn", "edition_id": history.editionId, "namespace": history.namespace,
                    "expected_heads": history.heads, "historical_node": node]
        }
    }
}
struct Asset: Decodable, Identifiable {
    static let mobi6MediaType = "application/x-mobipocket-ebook"
    let id: String; let mediaType: String; let storageKey: String; let sha256: String; let spine: [String]
    var isMobi6: Bool { mediaType == Self.mobi6MediaType }
    var supportsTextSize: Bool { mediaType == "application/epub+zip" || isMobi6 }
}
struct ReaderAttachment {
    let editionID: String
    let asset: Asset
}
struct Notebook: Decodable, Identifiable { let id: String; let bookId: String; let passId: String; let blocks: [NoteBlock]; let revisions: [NoteRevision] }
struct NoteBlock: Decodable, Identifiable { let id: String; let kind: String; let text: String; let update: String; let styles: [InlineStyle]?; var ink: NotebookInk? = nil; var note: NoteMetadata? = nil }
struct InlineStyle: Codable, Equatable {
    var startUtf16: Int; var lengthUtf16: Int; var bold: Bool; var italic: Bool; var underline: Bool
    var dictionary: [String: Any] { ["start_utf16": startUtf16, "length_utf16": lengthUtf16, "bold": bold, "italic": italic, "underline": underline] }
}
struct NoteRevision: Decodable, Identifiable { let id: String; let blockId: String; let reader: String; let at: UInt64; let update: String }
struct LiteraryEntity: Decodable, Identifiable { let id: String; let name: String; let kind: String; let bookId: String }
struct Card: Decodable, Identifiable {
    let id: String; let characterId: String; let notebookId: String; let blockId: String; let versions: [CardVersion]
    var heads: [CardVersion] { let parents = Set(versions.flatMap(\.parents)); return versions.filter { !parents.contains($0.id) } }
}
struct CardVersion: Decodable, Identifiable { let id: String; let parents: [String]; let reader: String; let at: UInt64; let passId: String; let positions: [JSONValue]; let positionObservations: [PositionObservation]?; let interpretation: Interpretation }
struct Interpretation: Codable {
    var description: String = ""
    var traits: [String] = []
    var relationships: [CharacterRelationship] = []
    var observations: [String] = []
    var questions: [String] = []
    var passages: [JSONValue] = []
}
struct CharacterRelationship: Codable { var target: String; var description: String }
struct ReadingList: Decodable, Identifiable { let id: String; let owner: String; let title: String; let bookIds: [String]; let shared: Bool; let version: UInt64 }
struct ReadingGroup: Decodable, Identifiable { let id: String; let name: String; let `private`: Bool; let epoch: UInt64; let members: [String] }
struct ShareDraft: Decodable, Identifiable { let id: String; let kind: String; let body: String; let spoiler: Bool }
struct ReadingPass: Decodable, Identifiable { let id: String; let bookId: String; let startedAt: UInt64 }
struct ReadingSession: Decodable, Identifiable { let id: String; let passId: String; let startedAt: UInt64; let endedAt: UInt64? }
struct Annotation: Decodable, Identifiable { let id: String; let passId: String; let anchor: JSONValue; let originalContext: String; let layoutFingerprint: String?; let nativeDrawing: String?; let strokes: [JSONValue]; let inkGeometry: JSONValue? }
struct PositionObservation: Codable, Identifiable { let id: String; let passId: String; let location: JSONValue; let observedAt: UInt64; let receivedAt: UInt64; let alignment: JSONValue; let providerConnectionId: String?; var audioMetadata: JSONValue? = nil }
struct DeskSnapshot: Decodable {
    let ontologyVersion: String
    let revision: UInt64
    let localReader: String
    let books: [String: LibraryBook]
    let editions: [String: Edition]
    let notebooks: [String: Notebook]
    let entities: [String: LiteraryEntity]
    let cards: [String: Card]
    let lists: [String: ReadingList]
    let groups: [String: ReadingGroup]
    let drafts: [String: ShareDraft]
    let passes: [String: ReadingPass]
    let sessions: [String: ReadingSession]
    let annotations: [String: Annotation]
    let positions: [String: PositionObservation]
}
struct Response: Decodable { let state: DeskSnapshot; let result: JSONValue? }
struct ReleaseGate: Decodable, Identifiable { let id: String; let available: Bool; let reason: String? }

/// Lossless transport for extensible locator values; domain records above remain typed.
enum JSONValue: Codable {
    case object([String: JSONValue]), array([JSONValue]), string(String), unsigned(UInt64), integer(Int64), number(Double), bool(Bool), null
    init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() { self = .null }
        else if let v = try? c.decode(Bool.self) { self = .bool(v) }
        else if let v = try? c.decode(String.self) { self = .string(v) }
        else if let v = try? c.decode(UInt64.self) { self = .unsigned(v) }
        else if let v = try? c.decode(Int64.self) { self = .integer(v) }
        else if let v = try? c.decode(Double.self) { self = .number(v) }
        else if let v = try? c.decode([String: JSONValue].self) { self = .object(v) }
        else { self = .array(try c.decode([JSONValue].self)) }
    }
    func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self { case .object(let v): try c.encode(v); case .array(let v): try c.encode(v); case .string(let v): try c.encode(v); case .unsigned(let v): try c.encode(v); case .integer(let v): try c.encode(v); case .number(let v): try c.encode(v); case .bool(let v): try c.encode(v); case .null: try c.encodeNil() }
    }
    var object: [String: Any]? { if case .object(let v) = self { return v.mapValues(\.any) }; return nil }
    var any: Any { switch self { case .object(let v): return v.mapValues(\.any); case .array(let v): return v.map(\.any); case .string(let v): return v; case .unsigned(let v): return v; case .integer(let v): return v; case .number(let v): return v; case .bool(let v): return v; case .null: return NSNull() } }
}

actor DeskRepository {
    private var desk: Desk?
    private var services: BookNookServices?
    private var openedDirectory: URL?
    private struct Opening {
        let id: UUID
        let directory: URL
        let task: Task<BookNookServices, Error>
    }
    private var opening: Opening?
    private let openingQueue = DispatchQueue(label: "org.booknook.library-open", qos: .userInitiated)
    private let openServices: @Sendable (URL, [any BookNookProvider]) throws -> BookNookServices
    private let directoryOverride: URL?
    private let additionalProviders: [any BookNookProvider]
    let sourceJobs = AppleProviderJobs()
    let recommendationProviderID: String
    let socialProviderID: String
    private let decoder: JSONDecoder = { let d = JSONDecoder(); d.keyDecodingStrategy = .convertFromSnakeCase; return d }()
    init(directory: URL? = nil, providers: [any BookNookProvider] = [],
         recommendationProviderID: String = BookNookServices.localRecommendations,
         socialProviderID: String = BookNookServices.nostrTransport,
         openServices: @escaping @Sendable (URL, [any BookNookProvider]) throws -> BookNookServices = {
             try BookNookServices(directory: $0, providers: $1)
         }) {
        self.openServices = openServices
        self.directoryOverride = directory
        self.additionalProviders = providers
        self.recommendationProviderID = recommendationProviderID
        self.socialProviderID = socialProviderID
    }
    func resolvedDirectory() throws -> URL {
        let root: URL
        if let directoryOverride { root = directoryOverride }
        else if let override = ProcessInfo.processInfo.environment["BOOKNOOK_DATA_DIR"] { root = URL(fileURLWithPath: override) }
        else { root = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true).appendingPathComponent("BookNook", isDirectory: true) }
        return root
    }
    func open() async throws -> (DeskSnapshot, [ReleaseGate]) {
        if services != nil {
            return (try snapshot(), try decoder.decode([ReleaseGate].self, from: Data(try required().releaseGates().utf8)))
        }
        let pending: Opening
        if let opening { pending = opening }
        else {
            let root = try resolvedDirectory()
            let providers = try AppleFileProviders.defaults() + additionalProviders
            let openServices = self.openServices, queue = openingQueue
            // Native construction includes blocking filesystem calls. A dedicated
            // queue leaves the actor and Swift's cooperative executor available.
            // The one shared task stays live until that operation actually returns;
            // cancelling a window must not start a competing open or fake a timeout.
            let task = Task<BookNookServices, Error> {
                try await withCheckedThrowingContinuation { continuation in
                    queue.async {
                        do { continuation.resume(returning: try openServices(root, providers)) }
                        catch { continuation.resume(throwing: error) }
                    }
                }
            }
            pending = Opening(id: UUID(), directory: root, task: task)
            opening = pending
        }
        do {
            let opened = try await pending.task.value
            if services == nil {
                let snapshot = try decoder.decode(DeskSnapshot.self, from: Data(try opened.desk.snapshot().utf8))
                let gates = try decoder.decode([ReleaseGate].self, from: Data(try opened.desk.releaseGates().utf8))
                services = opened; desk = opened.desk; openedDirectory = pending.directory
                if opening?.id == pending.id { opening = nil }
                return (snapshot, gates)
            }
            if opening?.id == pending.id { opening = nil }
            return (try snapshot(), try decoder.decode([ReleaseGate].self, from: Data(try required().releaseGates().utf8)))
        } catch {
            if opening?.id == pending.id { opening = nil }
            throw error
        }
    }
    func inkJournal() throws -> InkJournal { guard let openedDirectory else { throw CocoaError(.fileReadUnknown) }; return InkJournal(deskDirectory: openedDirectory) }
    func required() throws -> Desk { guard let desk else { throw CocoaError(.fileReadUnknown) }; return desk }
    func providerServices() throws -> BookNookServices { guard let services else { throw CocoaError(.fileReadUnknown) }; return services }
    func snapshot() throws -> DeskSnapshot { try decoder.decode(DeskSnapshot.self, from: Data(try required().snapshot().utf8)) }
    func execute(_ raw: Data) throws -> DeskSnapshot {
        try executeResponse(raw).state
    }
    func executeResponse(_ raw: Data) throws -> Response {
        let response = try required().execute(commandJson: String(decoding: raw, as: UTF8.self))
        return try decoder.decode(Response.self, from: Data(response.utf8))
    }
    func stageText(_ data: Data) throws { try required().stageTextEdit(editJson: String(decoding: data, as: UTF8.self)) }
    func flushText() throws -> DeskSnapshot { try decoder.decode(DeskSnapshot.self, from: Data(try required().flushTextEdits().utf8)) }
    func importContent(book: String, url: URL) async throws -> DeskSnapshot {
        let read = try await readReaderContent(url)
        return try await commitReaderContent(read, book: book)
    }
    func readReaderContent(_ url: URL) async throws -> BookNookProviderRead {
        let media: String
        switch url.pathExtension.lowercased() {
        case "epub": media = "application/epub+zip"
        case "pdf": media = "application/pdf"
        case "mobi", "azw": media = Asset.mobi6MediaType
        default: throw BookNookServiceError.message("Choose an EPUB, PDF, or DRM-free legacy MOBI (.mobi or .azw) file.")
        }
        return try await providerServices().fetch(providerID: AppleProviderIDs.contentFile,
            operationJSON: BookNookProviderWire.json(["kind": "content", "source_id": url.path, "media_type": media]))
    }
    func commitReaderContent(_ read: BookNookProviderRead, book: String) async throws -> DeskSnapshot {
        let raw = try await providerServices().commitContent(read, bookID: book)
        return try decoder.decode(DeskSnapshot.self, from: Data(raw.utf8))
    }
    func assetDirectory(_ id: String) throws -> URL { URL(fileURLWithPath: try required().assetDirectory(assetId: id), isDirectory: true) }
    func export(to url: URL) throws { try required().exportArchive(path: url.path) }
    func restore(from url: URL) throws -> DeskSnapshot { try decoder.decode(DeskSnapshot.self, from: Data(try required().restoreArchive(path: url.path).utf8)) }
    func connections() throws -> ConnectionConfiguration { try decoder.decode(ConnectionConfiguration.self, from: Data(try required().connectionSettings().utf8)) }
    func saveConnections(_ value: ConnectionConfiguration) throws { let encoder = JSONEncoder(); encoder.keyEncodingStrategy = .convertToSnakeCase; try required().saveConnectionSettings(settingsJson: String(decoding: encoder.encode(value), as: UTF8.self)) }
    func canonicalPayload(_ payload: Data) throws -> String { try required().canonicalPayload(payloadJson: String(decoding: payload, as: UTF8.self)) }
    func authorizedAction(payload: Data, action: String, resourceVersion: UInt64, idempotencyKey: String) throws -> String { try required().authorizedAction(payloadJson: String(decoding: payload, as: UTF8.self), action: action, resourceVersion: resourceVersion, idempotencyKey: idempotencyKey) }
    func custodyUnlock(recovery: Data, passphrase: String) throws -> String { try required().custodyUnlock(recoveryJson: String(decoding: recovery, as: UTF8.self), passphrase: passphrase) }
    func custodyLock() throws { try required().custodyLock() }
    func custodyPrepare(request: String, payload: String, action: String, resourceVersion: UInt64) throws -> String { try required().custodyPrepare(requestJson: request, payloadJson: payload, action: action, resourceVersion: resourceVersion) }
    func custodyApprove(token: String) throws -> String { try required().custodyApprove(token: token) }
    func custodyCancel(token: String) throws { try required().custodyCancel(token: token) }
}

@MainActor final class DeskModel: ObservableObject {
    @Published var state: DeskSnapshot?
    @Published private(set) var openingLibrary = false
    @Published private(set) var libraryOpenError: String?
    private var libraryOpeningTask: Task<Void, Never>?
    var libraryReady: Bool { state != nil && !openingLibrary }
    @Published var selectedBookId: String? {
        didSet { if oldValue != selectedBookId { readerScopeDidChange() } }
    }
    @Published var selectedNotebookId: String? {
        didSet { if oldValue != selectedNotebookId { readerScopeDidChange() } }
    }
    @Published private var readerChoiceID = UUID()
    private var readerEditionChoices: [String: String] = [:]
    private var readerIntentID = UUID()
    private var readingPassIntentID = UUID()
    private var deferredReaderEditions: Set<String> = []
    @Published var notebookScrollTarget: String?
    @Published var section = "desk" {
        didSet { if oldValue != section { readingPassIntentID = UUID() } }
    }
    @Published var sharingBookId: String?
    @Published var sharingDraftId: String?
    @Published var error: String?
    @Published private var textSaving = false
    var saving: Bool { !inkRecoveryFailures.isEmpty || textSaving || inkSessions.values.contains { $0.isSaving } || readerInkSessions.values.contains { $0.isSaving || $0.pendingCount > 0 } }
    private var inkSessions: [String: NotebookInkSession] = [:]
    private var readerInkSessions: [String: ReaderInkSession] = [:]
    private var inkJournal: InkJournal?
    private var inkRecoveryFailures: [String] = []
    private var observedReadingPositions: [String: PositionObservation] = [:]
    private var positionRequests: [String: UUID] = [:]
    private var pendingNoteCreations: [String: NotebookNoteCreation] = [:]
    private struct OrderedNoteAppend {
        let blockID: String
        let command: [String: Any]
    }
    private var pendingNoteAppends: [String: OrderedNoteAppend] = [:]
    private var noteAppendTasks: [String: Task<String, Error>] = [:]
    var readerRecoverySessions: [ReaderInkSession] { readerInkSessions.values.filter { $0.pendingCount > 0 && $0.error != nil }.sorted { ($0.passID, $0.assetFingerprint) < ($1.passID, $1.assetFingerprint) } }
    @Published var gates: [ReleaseGate] = []
    @Published var selection: [String: Any]?
    let repository: DeskRepository
    let publications: PublicationsModel
    init(repository: DeskRepository = DeskRepository(), publicationProvider: (any BookNookFeedSourceProviderV1)? = nil) {
        self.repository = repository
        publications = PublicationsModel(repository: PublicationRepository(desk: repository, provider: publicationProvider))
    }
    lazy var authorization = AuthorizationCoordinator(repository: repository)
    @Published var editorGeneration = UUID()
    struct PendingEditorDraft { let session: String; let baseUpdate: String; let text: String; let styles: [InlineStyle] }
    private var editorDrafts: [String: PendingEditorDraft] = [:]
    private func editorKey(_ notebook: String, _ block: String) -> String { (try? JSONSerialization.data(withJSONObject: [notebook, block]).base64EncodedString()) ?? notebook + block }
    func pendingEditorDraft(notebook: String, block: String) -> PendingEditorDraft? { editorDrafts[editorKey(notebook, block)] }
    private struct EditIntent {
        let notebook: String; let block: String; let session: String; let baseUpdate: String
        let text: String; let styles: [InlineStyle]; let sequence: UInt64
        let at: UInt64; let context: NotebookCaptureContext; let creation: NotebookNoteCreation?
        var data: Data { get throws {
            var value: [String: Any] = ["notebook_id": notebook, "block_id": block, "session_id": session,
                "sequence": sequence, "base_update": baseUpdate, "text": text, "styles": styles.map(\.dictionary),
                "at": at, "capture_context": try context.dictionary]
            if let creation { value["note_creation"] = try creation.dictionary }
            return try JSONSerialization.data(withJSONObject: value)
        } }
    }
    private var pendingText: [String: EditIntent] = [:]
    private var editSequence: UInt64 = 0
    private var writer: Task<Void, Never>?
    private var saveFailure: Error?
    private var flushRequested = false
    private var flushTask: Task<Void, Never>?
    private var flushToken = UUID()
    var books: [LibraryBook] { (state?.books.values.map { $0 } ?? []).sorted { $0.title.localizedCaseInsensitiveCompare($1.title) == .orderedAscending } }
    var book: LibraryBook? { selectedBookId.flatMap { state?.books[$0] } }
    var notebook: Notebook? { selectedNotebookId.flatMap { state?.notebooks[$0] } }
    var readerAttachments: [ReaderAttachment] {
        guard let book, let state else { return [] }
        return book.editionIds.compactMap { id in
            guard let edition = state.editions[id], edition.bookId == book.id,
                  let asset = edition.assets.last(where: {
                      $0.mediaType == "application/pdf" || ($0.supportsTextSize && !$0.spine.isEmpty)
                  }) else { return nil }
            return ReaderAttachment(editionID: id, asset: asset)
        }
    }
    var readerAttachment: ReaderAttachment? {
        guard let book, notebook?.bookId == book.id else { return nil }
        let attachments = readerAttachments
        if let chosen = readerEditionChoices[book.id],
           let attachment = attachments.first(where: { $0.editionID == chosen }) {
            return attachment
        }
        return attachments.last { !deferredReaderEditions.contains($0.editionID) } ?? attachments.last
    }
    var readerAttachmentIdentity: ReaderAttachmentIdentity? {
        guard let attachment = readerAttachment, let passID = notebook?.passId else { return nil }
        return ReaderAttachmentIdentity(passID: passID, editionID: attachment.editionID,
            assetID: attachment.asset.id, fingerprint: attachment.asset.sha256, choiceID: readerChoiceID)
    }
    func readerSourceContext(resource: String) -> ReaderSourceContext? {
        guard let identity = readerAttachmentIdentity, let asset = readerAttachment?.asset,
              (asset.mediaType == "application/pdf" ? resource == "book.pdf" : asset.spine.contains(resource)) else {
            return nil
        }
        return ReaderSourceContext(passID: identity.passID, editionID: identity.editionID,
            assetID: identity.assetID, fingerprint: identity.fingerprint, resource: resource, choiceID: identity.choiceID)
    }
    var readerAnnotations: [Annotation] {
        guard let identity = readerAttachmentIdentity else { return [] }
        return state?.annotations.values.filter {
            $0.passId == identity.passID &&
                (($0.anchor.object?["edition_id"] as? String) == identity.editionID ||
                 ReaderUnplacedInkLocation.matches($0, editionID: identity.editionID))
        } ?? []
    }
    var savedReaderPosition: [String: Any]? {
        guard let identity = readerAttachmentIdentity else { return nil }
        return state?.positions.values.filter {
            $0.passId == identity.passID && ($0.location.object?["edition_id"] as? String) == identity.editionID
        }.max { $0.observedAt < $1.observedAt }?.location.object
    }
    private func resetReaderContext() {
        readerChoiceID = UUID()
        selection = nil
    }
    private func readerScopeDidChange() {
        readerIntentID = UUID()
        readingPassIntentID = UUID()
        deferredReaderEditions.removeAll()
        resetReaderContext()
    }
    func selectReaderEdition(_ id: String) async throws {
        guard let bookID = book?.id, let notebook, notebook.bookId == bookID,
              readerAttachments.contains(where: { $0.editionID == id }) else {
            throw BookNookServiceError.message("This edition is no longer available in the current book.")
        }
        let passID = notebook.passId
        let intent = UUID()
        readerIntentID = intent
        guard readerEditionChoices[bookID] != id else { return }
        do { try await drainSaves(allowJournaledInk: true) }
        catch {
            guard readerIntentID == intent else { throw CancellationError() }
            throw error
        }
        try Task.checkCancellation()
        guard readerIntentID == intent, book?.id == bookID, self.notebook?.passId == passID,
              readerAttachments.contains(where: { $0.editionID == id }) else { throw CancellationError() }
        let previous = readerAttachmentIdentity
        readerEditionChoices[bookID] = id
        if readerAttachmentIdentity != previous { resetReaderContext() }
    }
    func attachReaderContent(_ url: URL) async throws {
        guard let bookID = book?.id, let notebook, notebook.bookId == bookID else {
            throw BookNookServiceError.message("Open a reading pass before attaching a book file.")
        }
        let passID = notebook.passId
        let intent = UUID()
        readerIntentID = intent
        func isCurrent() -> Bool {
            readerIntentID == intent && book?.id == bookID && self.notebook?.passId == passID && !Task.isCancelled
        }
        do {
            try await drainSaves(allowJournaledInk: true)
            guard isCurrent() else { throw CancellationError() }
            let read = try await repository.readReaderContent(url)
            guard isCurrent() else { throw CancellationError() }
            // Use the validated result's digest, so an unrelated attachment in
            // the returned snapshot cannot turn a duplicate into a new selection.
            let execution = try JSONSerialization.jsonObject(with: Data(read.executionJSON.utf8)) as? [String: Any]
            guard let response = execution?["response"] as? [String: Any],
                  let result = response["result"] as? [String: Any], let digest = result["sha256"] as? String else {
                throw CocoaError(.coderReadCorrupt)
            }
            func containsContent(_ editionID: String, in snapshot: DeskSnapshot?) -> Bool {
                snapshot?.editions[editionID]?.assets.contains(where: { $0.sha256 == digest }) == true
            }
            let alreadyAttached = book?.editionIds.contains { containsContent($0, in: state) } == true
            let snapshot = try await repository.commitReaderContent(read, book: bookID)
            let previous = readerAttachmentIdentity
            let previousChoiceID = readerChoiceID
            let latest = snapshot.revision >= (state?.revision ?? 0) ? snapshot : state
            if !alreadyAttached,
               let editionID = latest?.books[bookID]?.editionIds.last(where: { containsContent($0, in: latest) }) {
                if isCurrent() {
                    readerEditionChoices[bookID] = editionID
                } else if let previous, book?.id == bookID, previous.editionID != editionID {
                    // A commit that already started may finish after navigation.
                    // Keep it in the library without retargeting the open reader.
                    deferredReaderEditions.insert(editionID)
                }
            }
            accept(snapshot)
            if readerChoiceID == previousChoiceID, readerAttachmentIdentity != previous {
                resetReaderContext()
            }
        } catch {
            guard isCurrent() else { throw CancellationError() }
            throw error
        }
    }
    func acceptsReaderContext(_ context: ReaderSourceContext) -> Bool {
        readerSourceContext(resource: context.resource) == context
    }
    func readerMessages(from context: ReaderSourceContext, currentResource: @escaping () -> String,
                        receive: @escaping ([String: Any]) -> Void) -> ([String: Any]) -> Void {
        { [weak self] message in
            guard let self, self.acceptsReaderContext(context), context.resource == currentResource() else { return }
            receive(message)
        }
    }
    func open() async {
        if let libraryOpeningTask { await libraryOpeningTask.value; return }
        guard state == nil else { return }
        if error == libraryOpenError { error = nil }
        libraryOpenError = nil; openingLibrary = true
        let task = Task { @MainActor in
            do {
                let (snapshot, gates) = try await repository.open(); state = snapshot; self.gates = gates
                inkJournal = try await repository.inkJournal()
                if let first = books.first(where: { $0.shelf == "currently-reading" }) ?? books.first { await select(first.id) }
                try await recoverInk()
            } catch {
                let message = bookNookErrorMessage(error)
                self.error = message
                if state == nil { libraryOpenError = message }
            }
        }
        libraryOpeningTask = task
        await task.value
        libraryOpeningTask = nil; openingLibrary = false
    }
    func accept(_ snapshot: DeskSnapshot) {
        guard snapshot.revision >= (state?.revision ?? 0) else { return }
        let previous = readerAttachmentIdentity
        state = snapshot
        if readerAttachmentIdentity != previous { resetReaderContext() }
        for session in inkSessions.values {
            if let block = snapshot.notebooks[session.notebookID]?.blocks.first(where: { $0.id == session.blockID }) {
                session.accept(block.ink)
            }
        }
    }
    func inkSession(notebook: Notebook, block: NoteBlock) -> NotebookInkSession {
        let key = editorKey(notebook.id, block.id)
        if let existing = inkSessions[key] { return existing }
        let session = NotebookInkSession(notebookID: notebook.id, blockID: block.id,
            passID: notebook.passId, reader: state?.localReader ?? "", ink: block.ink,
            persist: { [weak self] command in
                guard let self else { throw CocoaError(.fileWriteUnknown) }
                _ = try await self.perform(command)
            }, journal: inkJournal, captureContext: { [weak self] in
                guard let self else { throw CocoaError(.fileReadUnknown) }
                return try self.captureReadingContext(passID: notebook.passId)
            }, noteCreation: { [weak self] capture in
                self?.captureNoteCreation(notebook: notebook.id, block: block.id, capture: capture)
            }, changed: { [weak self] in self?.objectWillChange.send() },
            failed: { [weak self] message in self?.error = message })
        inkSessions[key] = session
        return session
    }
    func captureReadingContext(passID: String) throws -> NotebookCaptureContext {
        guard let state, state.passes[passID] != nil else {
            throw InkJournalFailure("The reading pass is unavailable. Your writing has not been moved to another pass.")
        }
        var observations = state.positions
        // A reader callback can precede its asynchronous database acknowledgement.
        // Preserve that available observation now, before drawing/text save awaits.
        for (id, value) in observedReadingPositions where value.passId == passID {
            if let saved = observations[id], saved.observedAt > value.observedAt { continue }
            observations[id] = value
        }
        var latest: [[Data]: PositionObservation] = [:]
        for value in observations.values where value.passId == passID {
            guard let location = value.location.object, let kind = location["kind"] as? String else { continue }
            let fields: [String]
            switch kind {
            case "text", "page":
                guard let edition = location["edition_id"] as? String else { continue }
                fields = ["edition:\(edition)", "edition", "", edition, ""]
            case "audio", "provider":
                guard let provider = location["provider"] as? String, let source = location["source_id"] as? String else { continue }
                fields = ["\(provider):\(source)", kind, provider, source, value.providerConnectionId ?? ""]
            default: continue
            }
            // Preserve source identifiers byte for byte, including distinct Unicode spellings.
            let key = fields.map { Data($0.utf8) }
            if let old = latest[key], (old.observedAt, old.id) >= (value.observedAt, value.id) { continue }
            latest[key] = value
        }
        let values = latest.keys.sorted { left, right in
            left.lexicographicallyPrecedes(right) { $0.lexicographicallyPrecedes($1) }
        }.compactMap { latest[$0] }
        return NotebookCaptureContext(passId: passID, positions: values.map(\.location), positionObservations: values)
    }
    private func captureNoteCreation(notebook: String, block: String, capture: NotebookInkCapture) -> NotebookNoteCreation? {
        let key = editorKey(notebook, block)
        if let existing = pendingNoteCreations[key] { return existing }
        guard let value = state?.notebooks[notebook]?.blocks.first(where: { $0.id == block }) else { return nil }
        // Existing populated legacy sections get their real historic fallback in
        // the core. Never relabel one as a newly written note.
        let pending = value.note?.created.source == "pending"
        let blank = value.note == nil && value.text.isEmpty && value.ink == nil &&
            !(state?.cards.values.contains { $0.notebookId == notebook && $0.blockId == block } ?? false)
        guard pending || blank else { return nil }
        let creation = NotebookNoteCreation(at: capture.at, captureContext: capture.context)
        pendingNoteCreations[key] = creation
        return creation
    }
    @discardableResult func createNote(notebook: String, face: String = "text") async throws -> String {
        let key = editorKey(notebook, face)
        if let active = noteAppendTasks[key] { return try await active.value }
        let intent: OrderedNoteAppend
        if let pending = pendingNoteAppends[key] {
            intent = pending
        } else {
            guard let visible = state?.notebooks[notebook] else { throw CocoaError(.fileReadUnknown) }
            let context = try captureReadingContext(passID: visible.passId)
            let blockID = UUID().uuidString.lowercased().replacingOccurrences(of: "-", with: "")
            let command: [String: Any] = [
                "op": "append_ordered_note", "notebook_id": notebook, "block_id": blockID,
                "visible_block_ids": visible.blocks.map(\.id), "at": Int(Date().timeIntervalSince1970),
                "face": face, "capture_context": try context.dictionary,
            ]
            intent = OrderedNoteAppend(blockID: blockID, command: command)
            pendingNoteAppends[key] = intent
        }
        let append = Task { @MainActor in
            defer { self.noteAppendTasks.removeValue(forKey: key) }
            try await self.drainSaves(allowJournaledInk: true)
            let result = try await self.perform(intent.command)
            guard result?.object?["block_id"] as? String == intent.blockID else {
                throw CocoaError(.coderReadCorrupt)
            }
            self.pendingNoteAppends.removeValue(forKey: key)
            if self.selectedNotebookId == notebook { self.notebookScrollTarget = intent.blockID }
            return intent.blockID
        }
        noteAppendTasks[key] = append
        return try await append.value
    }
    func setNoteFace(notebook: String, block: String, face: String) async throws {
        let at = UInt64(Date().timeIntervalSince1970)
        try await drainSaves(allowJournaledInk: true)
        try await perform(["op": "set_note_face", "notebook_id": notebook, "block_id": block, "face": face, "at": at])
    }
    func characterNoteCommand(notebook: String, block: String, name: String, existingCharacterID: String?) throws -> [String: Any] {
        guard let pass = state?.notebooks[notebook]?.passId else { throw CocoaError(.fileReadUnknown) }
        let capture = NotebookInkCapture(at: UInt64(Date().timeIntervalSince1970), context: try captureReadingContext(passID: pass))
        var command: [String: Any] = ["notebook_id": notebook, "block_id": block, "at": capture.at,
            "capture_context": try capture.context.dictionary]
        if let id = existingCharacterID, !id.isEmpty { command["op"] = "attach_character"; command["character_id"] = id }
        else { command["op"] = "add_character"; command["name"] = name }
        if let creation = captureNoteCreation(notebook: notebook, block: block, capture: capture) {
            command["note_creation"] = try creation.dictionary
        }
        return command
    }
    func characterSupportingSelection(notebookID: String) -> [String: Any]? {
        guard selectedNotebookId == notebookID, let attachment = readerAttachment, let selection,
              selection["edition_id"] as? String == attachment.editionID else { return nil }
        return selection
    }
    func characterVersionCommand(cardID: String, parents: [String], interpretation: Interpretation,
                                 at: UInt64 = UInt64(Date().timeIntervalSince1970)) throws -> [String: Any] {
        guard let state, let card = state.cards[cardID], let notebook = state.notebooks[card.notebookId] else {
            throw AuthorizationError.message("This character card is no longer available.")
        }
        let captured = try captureReadingContext(passID: notebook.passId)
        var positions = captured.positions
        var interpretation = interpretation
        if let selection = characterSupportingSelection(notebookID: notebook.id) {
            let options: JSONSerialization.WritingOptions = [.sortedKeys, .fragmentsAllowed]
            let data = try JSONSerialization.data(withJSONObject: selection, options: options)
            let location = try JSONDecoder().decode(JSONValue.self, from: data)
            func includingSelection(in locations: [JSONValue]) throws -> [JSONValue] {
                let included = try locations.contains { try JSONSerialization.data(withJSONObject: $0.any, options: options) == data }
                return included ? locations : locations + [location]
            }
            positions = try includingSelection(in: positions)
            interpretation.passages = try includingSelection(in: interpretation.passages)
        }
        let context = NotebookCaptureContext(passId: captured.passId, positions: positions,
                                             positionObservations: captured.positionObservations)
        let encoder = JSONEncoder()
        encoder.keyEncodingStrategy = .convertToSnakeCase
        return ["op": "save_card_with_context", "card_id": cardID, "parents": parents, "at": at,
                "interpretation": try JSONSerialization.jsonObject(with: encoder.encode(interpretation)),
                "capture_context": try context.dictionary]
    }
    func continueHandwriting(notebookID: String) async {
        do { try await createNote(notebook: notebookID, face: "whiteboard") }
        catch { self.error = bookNookErrorMessage(error) }
    }
    func saveInkTranscription(notebook: String, block: String, version: String, text: String, method: String) async throws {
        try await perform(["op": "save_notebook_transcription", "notebook_id": notebook,
            "block_id": block, "transcription_id": UUID().uuidString, "ink_version_id": version,
            "text": text, "method": method, "at": Int(Date().timeIntervalSince1970)])
    }
    func readerInkSession(passID: String, editionID: String, assetFingerprint: String) -> ReaderInkSession {
        let key = (try? JSONSerialization.data(withJSONObject: [passID, editionID, assetFingerprint]).base64EncodedString()) ?? passID + editionID + assetFingerprint
        if let existing = readerInkSessions[key] { return existing }
        let session = ReaderInkSession(passID: passID, editionID: editionID, assetFingerprint: assetFingerprint,
            persist: { [weak self] command in
                guard let self else { throw CocoaError(.fileWriteUnknown) }
                _ = try await self.perform(command)
            }, journal: inkJournal, changed: { [weak self] in self?.objectWillChange.send() },
            failed: { [weak self] message in self?.error = message })
        readerInkSessions[key] = session
        return session
    }
    private func recoverInk() async throws {
        guard let inkJournal else { return }
        let records: [InkJournalRecord], loadFailures: [String]
        do { (records, loadFailures) = try await inkJournal.load() }
        catch {
            let message = "Ink recovery could not be read: \(bookNookErrorMessage(error))"
            inkRecoveryFailures = [message]; self.error = message
            objectWillChange.send()
            throw InkJournalFailure(message)
        }
        var failures = loadFailures
        for record in records {
            do {
                guard let payload = record.payload.object else { throw CocoaError(.coderReadCorrupt) }
                if record.kind == "notebook" {
                    guard let notebookID = payload["notebook_id"] as? String, let blockID = payload["block_id"] as? String,
                          let notebook = state?.notebooks[notebookID], let block = notebook.blocks.first(where: { $0.id == blockID }) else {
                        throw InkJournalFailure("The notebook for a pending drawing is unavailable. Its recovery file was retained.")
                    }
                    try inkSession(notebook: notebook, block: block).recover(record)
                    if let raw = payload["note_creation"] {
                        let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase
                        let creation = try decoder.decode(NotebookNoteCreation.self, from: JSONSerialization.data(withJSONObject: raw))
                        let key = editorKey(notebookID, blockID)
                        if pendingNoteCreations[key] == nil { pendingNoteCreations[key] = creation }
                    }
                } else {
                    guard let pass = payload["pass_id"] as? String, let edition = payload["edition_id"] as? String,
                          let fingerprint = payload["asset_fingerprint"] as? String,
                          let readingPass = state?.passes[pass], state?.editions[edition]?.bookId == readingPass.bookId else {
                        throw InkJournalFailure("The reading pass or edition for pending markup is unavailable. Its recovery file was retained.")
                    }
                    if let saved = state?.annotations[record.id] {
                        // Final SQLite commit may precede journal removal. Never
                        // overwrite a different record or relocate an old capture.
                        guard saved.passId == pass, saved.nativeDrawing == payload["native_drawing"] as? String,
                              NSDictionary(dictionary: ["strokes": saved.strokes.map(\.any)]).isEqual(to: ["strokes": payload["strokes"] as Any]),
                              saved.anchor.object?["edition_id"] as? String == edition || ReaderUnplacedInkLocation.matches(saved, editionID: edition) else {
                            throw InkJournalFailure("A saved annotation differs from its pending original. Both copies were retained for review.")
                        }
                        try await inkJournal.remove(record)
                    } else {
                        try readerInkSession(passID: pass, editionID: edition, assetFingerprint: fingerprint).recover(record)
                    }
                }
            } catch { failures.append("Pending ink recovery needs attention: \(bookNookErrorMessage(error))") }
        }
        let previousRecoveryError = inkRecoveryFailures.joined(separator: "\n")
        inkRecoveryFailures = failures
        if !failures.isEmpty { error = failures.joined(separator: "\n") }
        else if error == previousRecoveryError { error = nil }
        objectWillChange.send()
    }
    func send(_ command: [String: Any]) async {
        do { try await perform(command) }
        catch { self.error = bookNookErrorMessage(error) }
    }
    @discardableResult func perform(_ command: [String: Any]) async throws -> JSONValue? {
        let data = try JSONSerialization.data(withJSONObject: command)
        let response = try await repository.executeResponse(data)
        accept(response.state)
        return response.result
    }
    func select(_ id: String) async {
        do { try await selectBook(id) } catch { self.error = bookNookErrorMessage(error) }
    }
    func selectBook(_ id: String) async throws {
        // A new book request supersedes a pending pass change before either save completes.
        readingPassIntentID = UUID()
        try await drainSaves(allowJournaledInk: true)
        let result = try await perform(["op": "open_notebook", "book_id": id, "at": Int(Date().timeIntervalSince1970)])
        guard let notebookID = result?.object?["id"] as? String else { throw CocoaError(.coderReadCorrupt) }
        selectedBookId = id; selectedNotebookId = notebookID; selection = nil
    }
    struct ReadingPassIntent {
        fileprivate let id: UUID
        fileprivate let bookID: String
        fileprivate let notebookID: String
    }
    enum ReadingPassAction {
        case select(String)
        case begin(at: Int)
    }
    func captureReadingPassIntent() -> ReadingPassIntent? {
        guard section == "desk", let bookID = book?.id, let notebook, notebook.bookId == bookID else { return nil }
        let id = UUID()
        readingPassIntentID = id
        return ReadingPassIntent(id: id, bookID: bookID, notebookID: notebook.id)
    }
    func changeReadingPass(_ action: ReadingPassAction, intent: ReadingPassIntent) async throws {
        func isCurrent() -> Bool {
            !Task.isCancelled && readingPassIntentID == intent.id && section == "desk" &&
                book?.id == intent.bookID && notebook?.id == intent.notebookID && notebook?.bookId == intent.bookID
        }
        guard isCurrent() else { throw CancellationError() }
        if case .select(let id) = action, id == intent.notebookID { return }
        do {
            try await drainSaves(allowJournaledInk: true)
            guard isCurrent() else { throw CancellationError() }

            let notebookID: String
            switch action {
            case .select(let id):
                notebookID = id
            case .begin(let at):
                let result = try await perform(["op": "start_pass", "book_id": intent.bookID, "at": at])
                // The pass may already be saved. Navigation only cancels taking focus.
                guard isCurrent() else { throw CancellationError() }
                guard let id = result?.object?["id"] as? String else { throw CocoaError(.coderReadCorrupt) }
                notebookID = id
            }
            guard state?.notebooks[notebookID]?.bookId == intent.bookID else {
                throw BookNookServiceError.message("This reading pass is no longer available in the current book.")
            }
            selectedNotebookId = notebookID
            reloadEditorSessions()
        } catch {
            guard isCurrent() else { throw CancellationError() }
            throw error
        }
    }
    func saveText(notebook: String, block: String, session: String, baseUpdate: String, text: String, styles: [InlineStyle]) {
        guard editSequence < UInt64.max else { error = "Edit sequence exhausted. Reopen the desk."; return }
        editSequence += 1
        editorDrafts[editorKey(notebook, block)] = PendingEditorDraft(session: session, baseUpdate: baseUpdate, text: text, styles: styles)
        let queueKey = (try? JSONSerialization.data(withJSONObject: [notebook, block, session]).base64EncodedString()) ?? session
        do {
            guard let pass = state?.notebooks[notebook]?.passId else { throw CocoaError(.fileReadUnknown) }
            let capture = NotebookInkCapture(at: UInt64(Date().timeIntervalSince1970), context: try captureReadingContext(passID: pass))
            let creation = text.isEmpty ? pendingNoteCreations[editorKey(notebook, block)] : captureNoteCreation(notebook: notebook, block: block, capture: capture)
            pendingText[queueKey] = EditIntent(notebook: notebook, block: block, session: session, baseUpdate: baseUpdate,
                text: text, styles: styles, sequence: editSequence, at: capture.at, context: capture.context, creation: creation)
            textSaving = true; flushTask?.cancel(); startWriter()
        } catch { self.error = bookNookErrorMessage(error) }
    }
    private func startWriter() {
        guard writer == nil else { return }
        saveFailure = nil
        writer = Task {
            do {
                while let (key, edit) = pendingText.first {
                    try await repository.stageText(edit.data)
                    if pendingText[key]?.sequence == edit.sequence { pendingText.removeValue(forKey: key) }
                }
            } catch { saveFailure = error; self.error = "Not fully saved: \(error.localizedDescription)" }
            writer = nil
            if saveFailure == nil && !flushRequested { scheduleFlush() }
        }
    }
    private func scheduleFlush() {
        flushTask?.cancel(); let token = UUID(); flushToken = token
        flushTask = Task {
            do {
                try await Task.sleep(for: .milliseconds(300))
                let snapshot = try await repository.flushText()
                accept(snapshot)
                if !Task.isCancelled && flushToken == token {
                    textSaving = !pendingText.isEmpty || writer != nil
                    if !textSaving { editorDrafts.removeAll() }
                    flushTask = nil
                }
            } catch is CancellationError { }
            catch { if flushToken == token { saveFailure = error; self.error = "Not fully saved: \(error.localizedDescription)"; textSaving = true; flushTask = nil } }
        }
    }
    func retrySave() {
        if !inkRecoveryFailures.isEmpty { Task { do { try await recoverInk() } catch { self.error = bookNookErrorMessage(error) } } }
        textSaving = true; startWriter()
        for session in inkSessions.values { session.retry() }
        for session in readerInkSessions.values { session.retry() }
    }
    /// Commit native composition, durably stage every intent, then merge the journal.
    func drainSaves(allowJournaledInk: Bool = false) async throws {
        for session in inkSessions.values { session.capturePendingStroke?() }
        for session in readerInkSessions.values { session.captureAll() }
        #if os(macOS)
        NSApp.keyWindow?.makeFirstResponder(nil)
        #else
        UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
        #endif
        flushRequested = true; flushTask?.cancel(); flushTask = nil; flushToken = UUID()
        defer { flushRequested = false }
        if writer == nil && !pendingText.isEmpty { startWriter() }
        while let writer { await writer.value }
        if let saveFailure { throw saveFailure }
        do { accept(try await repository.flushText()); textSaving = !pendingText.isEmpty }
        catch { saveFailure = error; textSaving = true; throw error }
        if allowJournaledInk {
            // Navigation can leave a durable unresolved capture behind; portable
            // export and explicit final save still require its reconciliation.
            for session in inkSessions.values { try await session.checkpoint() }
            for session in readerInkSessions.values { try await session.checkpoint() }
        } else {
            if !inkRecoveryFailures.isEmpty { throw InkJournalFailure(inkRecoveryFailures.joined(separator: "\n")) }
            for session in inkSessions.values { try await session.flush() }
            for session in readerInkSessions.values { try await session.flush() }
        }
        if saving && !allowJournaledInk { throw AuthorizationError.message("Some writing is still pending. Retry save before continuing.") }
        editorDrafts.removeAll()
    }
    func reloadEditorSessions() { editorDrafts.removeAll(); editorGeneration = UUID() }
    func recordPosition(location: [String: Any], alignment: [String: Any] = ["status": "unmatched"]) {
        guard let pass = notebook?.passId else { return }
        let source = location["edition_id"] as? String ?? "\(location["provider"] as? String ?? "reader"):\(location["source_id"] as? String ?? "")"
        let now = Int(Date().timeIntervalSince1970)
        let observation: [String: Any] = ["id": "\(pass):position:\(source)", "pass_id": pass, "location": location, "observed_at": now, "received_at": now, "alignment": alignment]
        do {
            let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase
            let captured = try decoder.decode(PositionObservation.self, from: JSONSerialization.data(withJSONObject: observation))
            observedReadingPositions[captured.id] = captured
            let request = UUID(); positionRequests[captured.id] = request
            Task {
                do { try await perform(["op": "observe_position", "observation": observation]) }
                catch { self.error = bookNookErrorMessage(error) }
                if positionRequests[captured.id] == request {
                    positionRequests.removeValue(forKey: captured.id)
                    observedReadingPositions.removeValue(forKey: captured.id)
                }
            }
        } catch { self.error = bookNookErrorMessage(error) }
    }
}
