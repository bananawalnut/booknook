// SPDX-License-Identifier: MPL-2.0
import Foundation
import SwiftUI

struct LibraryBook: Decodable, Identifiable {
    let id: String
    let title: String
    let authors: [String]
    let subjects: [String]
    let editionIds: [String]
    let shelf: String
    let rating: Int?
    let review: String
}
struct Edition: Decodable, Identifiable { let id: String; let bookId: String; let assets: [Asset] }
struct Asset: Decodable, Identifiable { let id: String; let mediaType: String; let storageKey: String; let sha256: String; let spine: [String] }
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
    private let directoryOverride: URL?
    private let additionalProviders: [any BookNookProvider]
    let sourceJobs = AppleProviderJobs()
    let recommendationProviderID: String
    let socialProviderID: String
    private let decoder: JSONDecoder = { let d = JSONDecoder(); d.keyDecodingStrategy = .convertFromSnakeCase; return d }()
    init(directory: URL? = nil, providers: [any BookNookProvider] = [],
         recommendationProviderID: String = BookNookServices.localRecommendations,
         socialProviderID: String = BookNookServices.nostrTransport) {
        self.directoryOverride = directory
        self.additionalProviders = providers
        self.recommendationProviderID = recommendationProviderID
        self.socialProviderID = socialProviderID
    }
    func open() throws -> (DeskSnapshot, [ReleaseGate]) {
        let root: URL
        if let directoryOverride { root = directoryOverride }
        else if let override = ProcessInfo.processInfo.environment["BOOKNOOK_DATA_DIR"] { root = URL(fileURLWithPath: override) }
        else { root = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true).appendingPathComponent("BookNook", isDirectory: true) }
        let services = try BookNookServices(directory: root, providers: AppleFileProviders.defaults() + additionalProviders)
        self.services = services; openedDirectory = root
        desk = services.desk
        return (try snapshot(), try decoder.decode([ReleaseGate].self, from: Data(try required().releaseGates().utf8)))
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
        let media: String
        switch url.pathExtension.lowercased() {
        case "epub": media = "application/epub+zip"
        case "pdf": media = "application/pdf"
        default: throw BookNookServiceError.message("Choose an accessible EPUB or PDF file.")
        }
        return try await importContent(providerID: AppleProviderIDs.contentFile, book: book, sourceID: url.path, mediaType: media)
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
    @Published var selectedBookId: String?
    @Published var selectedNotebookId: String?
    @Published var notebookScrollTarget: String?
    @Published var section = "desk"
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
    var readerRecoverySessions: [ReaderInkSession] { readerInkSessions.values.filter { $0.pendingCount > 0 && $0.error != nil }.sorted { ($0.passID, $0.assetFingerprint) < ($1.passID, $1.assetFingerprint) } }
    @Published var gates: [ReleaseGate] = []
    @Published var selection: [String: Any]?
    let repository: DeskRepository
    init(repository: DeskRepository = DeskRepository()) { self.repository = repository }
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
    func acceptsReaderContext(_ context: ReaderSourceContext) -> Bool {
        guard notebook?.passId == context.passID, let editionID = book?.editionIds.last,
              editionID == context.editionID, let asset = state?.editions[editionID]?.assets.last else { return false }
        return asset.id == context.assetID && asset.sha256 == context.fingerprint
    }
    func readerMessages(from context: ReaderSourceContext, currentResource: @escaping () -> String,
                        receive: @escaping ([String: Any]) -> Void) -> ([String: Any]) -> Void {
        { [weak self] message in
            guard let self, self.acceptsReaderContext(context), context.resource == currentResource() else { return }
            receive(message)
        }
    }
    func open() async {
        do {
            let (snapshot, gates) = try await repository.open(); state = snapshot; self.gates = gates
            inkJournal = try await repository.inkJournal()
            if let first = books.first(where: { $0.shelf == "currently-reading" }) ?? books.first { await select(first.id) }
            try await recoverInk()
        }
        catch { self.error = bookNookErrorMessage(error) }
    }
    func accept(_ snapshot: DeskSnapshot) {
        guard snapshot.revision >= (state?.revision ?? 0) else { return }
        state = snapshot
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
        var latest: [String: PositionObservation] = [:]
        for value in observations.values where value.passId == passID {
            guard let location = value.location.object else { continue }
            let key: String
            switch location["kind"] as? String {
            case "text", "page": key = "edition:\(location["edition_id"] as? String ?? "")"
            default: key = "\(location["provider"] as? String ?? ""):\(location["source_id"] as? String ?? "")"
            }
            if let old = latest[key], (old.observedAt, old.id) >= (value.observedAt, value.id) { continue }
            latest[key] = value
        }
        let values = latest.keys.sorted().compactMap { latest[$0] }
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
        guard let pass = state?.notebooks[notebook]?.passId else { throw CocoaError(.fileReadUnknown) }
        let at = UInt64(Date().timeIntervalSince1970)
        let context = try captureReadingContext(passID: pass)
        let command: [String: Any] = ["op": "add_block", "notebook_id": notebook, "face": face,
            "at": at, "capture_context": try context.dictionary]
        try await drainSaves(allowJournaledInk: true)
        let result = try await perform(command)
        guard let id = result?.object?["block_id"] as? String else { throw CocoaError(.coderReadCorrupt) }
        notebookScrollTarget = id
        return id
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
        let (records, loadFailures) = try await inkJournal.load()
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
        inkRecoveryFailures = failures
        if !failures.isEmpty { error = failures.joined(separator: "\n") }
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
        try await drainSaves(allowJournaledInk: true)
        let result = try await perform(["op": "open_notebook", "book_id": id, "at": Int(Date().timeIntervalSince1970)])
        guard let notebookID = result?.object?["id"] as? String else { throw CocoaError(.coderReadCorrupt) }
        selectedBookId = id; selectedNotebookId = notebookID; selection = nil
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
