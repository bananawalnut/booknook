// SPDX-License-Identifier: MPL-2.0
import Foundation
import Combine

struct NotebookInk: Decodable {
    let schema: String
    let versions: [NotebookInkVersion]
    let transcriptions: [NotebookInkTranscription]
    var heads: [NotebookInkVersion] {
        let parents = Set(versions.flatMap(\.parents))
        return versions.filter { !parents.contains($0.id) }
    }
}
struct NotebookInkPoint: Codable, Equatable {
    let x: Double; let y: Double; let pressure: Double; let time: Double
    var dictionary: [String: Any] { ["x": x, "y": y, "pressure": pressure, "time": time] }
}
struct NotebookInkStroke: Codable, Equatable {
    let points: [NotebookInkPoint]; let color: String; let width: Double
    var dictionary: [String: Any] { ["points": points.map(\.dictionary), "color": color, "width": width] }
}
struct NotebookInkVersion: Codable, Identifiable {
    let id: String; let parents: [String]; let reader: String; let at: UInt64
    let passId: String; let positions: [JSONValue]; let positionObservations: [PositionObservation]?
    let canvasWidth: Double; let canvasHeight: Double
    let strokes: [NotebookInkStroke]; let nativeDrawing: String?
    var canvasSpace: NotebookCanvasSpace? = nil
}
struct NotebookInkTranscription: Decodable, Identifiable {
    let id: String; let inkVersionId: String; let reader: String; let at: UInt64
    let text: String; let method: String
}

/// Retained by the desk, not a lazy notebook row: leaving a pane never drops a
/// failed save or queued stroke. Each request has a stable ID for idempotent retry.
@MainActor final class NotebookInkSession: ObservableObject {
    let notebookID: String; let blockID: String
    private let passID: String; private let reader: String
    @Published private(set) var ink: NotebookInk?
    @Published private(set) var pending: [NotebookInkVersion] = []
    @Published private(set) var selectedVersionID: String?
    @Published private(set) var saveError: String?
    private var writer: Task<Void, Never>?
    private let persist: ([String: Any]) async throws -> Void
    private let journal: InkJournal?
    private let captureContext: () throws -> NotebookCaptureContext
    private let noteCreation: (NotebookInkCapture) -> NotebookNoteCreation?
    private var noteCreations: [String: NotebookNoteCreation] = [:]
    private var records: [String: InkJournalRecord] = [:]
    private var staging: [String: Task<Void, Error>] = [:]
    @Published private(set) var journaledIDs: Set<String> = []
    var allPendingJournaled: Bool { pending.allSatisfy { journaledIDs.contains($0.id) } }
    private let changed: () -> Void
    private let failed: (String) -> Void
    // Weakly captured by the native canvas. Captures an in-progress stroke before
    // export, backgrounding, pane navigation or layout changes drain queued saves.
    var capturePendingStroke: (() -> Void)?

    init(notebookID: String, blockID: String, passID: String, reader: String, ink: NotebookInk?,
         persist: @escaping ([String: Any]) async throws -> Void, journal: InkJournal? = nil,
         captureContext: (() throws -> NotebookCaptureContext)? = nil,
         noteCreation: @escaping (NotebookInkCapture) -> NotebookNoteCreation? = { _ in nil },
         changed: @escaping () -> Void = {}, failed: @escaping (String) -> Void = { _ in }) {
        self.notebookID = notebookID; self.blockID = blockID; self.passID = passID
        self.reader = reader; self.ink = ink; self.persist = persist
        self.changed = changed; self.failed = failed; self.journal = journal
        self.captureContext = captureContext ?? { .empty(passID: passID) }
        self.noteCreation = noteCreation
    }
    var heads: [NotebookInkVersion] { ink?.heads ?? [] }
    var versions: [NotebookInkVersion] { (ink?.versions ?? []) + pending.filter { draft in !(ink?.versions.contains { $0.id == draft.id } ?? false) } }
    var currentVersion: NotebookInkVersion? {
        if let last = pending.last { return last }
        if let id = selectedVersionID { return ink?.versions.first { $0.id == id } }
        return heads.count == 1 ? heads.first : nil
    }
    var needsVersionChoice: Bool { pending.isEmpty && selectedVersionID == nil && heads.count > 1 }
    var isSaving: Bool { !pending.isEmpty || writer != nil }
    var pendingCount: Int { pending.count }
    func accept(_ incoming: NotebookInk?) {
        let before = (ink?.versions.map(\.id) ?? []) + (ink?.transcriptions.map(\.id) ?? [])
        let after = (incoming?.versions.map(\.id) ?? []) + (incoming?.transcriptions.map(\.id) ?? [])
        guard before != after else { return }
        ink = incoming
        if let selectedVersionID, !versions.contains(where: { $0.id == selectedVersionID }) { self.selectedVersionID = nil }
    }
    @discardableResult func chooseVersion(id: String) -> Bool {
        guard !isSaving, ink?.versions.contains(where: { $0.id == id }) == true else { return false }
        selectedVersionID = id; changed(); return true
    }
    func beginCapture() -> NotebookInkCapture {
        let at = UInt64(Date().timeIntervalSince1970)
        do {
            let context = try captureContext()
            guard context.passId == passID else { throw InkJournalFailure("The drawing belongs to a different reading pass.") }
            return NotebookInkCapture(at: at, context: context)
        } catch {
            report("The drawing's reading position is unavailable. Its original drawing will be retained; retry saves without a source position. \(bookNookErrorMessage(error))")
            return NotebookInkCapture(at: at, context: .empty(passID: passID))
        }
    }
    @discardableResult func save(canvasWidth: Double, canvasHeight: Double, strokes: [[String: Any]], nativeDrawing: String?, parents: [String]? = nil, canvasSpace: NotebookCanvasSpace? = nil, capturedContext: NotebookInkCapture? = nil) -> Bool {
        guard parents != nil || !needsVersionChoice else { report("Choose an ink version before continuing to write."); return false }
        do {
            let data = try JSONSerialization.data(withJSONObject: strokes)
            let points = try JSONDecoder().decode([NotebookInkStroke].self, from: data)
            let current = currentVersion
            // UIKit can report the same drawing on end-of-tool and teardown.
            if let current, (parents == nil || parents == [current.id]), current.strokes == points, current.nativeDrawing == nativeDrawing,
               current.canvasWidth == canvasWidth, current.canvasHeight == canvasHeight, current.canvasSpace == canvasSpace { return true }
            if current == nil && points.isEmpty && (parents ?? []).isEmpty && versions.isEmpty { return true }
            let capture = capturedContext ?? beginCapture(), context = capture.context
            guard context.passId == passID else { throw InkJournalFailure("The drawing belongs to a different reading pass. Its save was not moved.") }
            let version = NotebookInkVersion(id: UUID().uuidString, parents: parents ?? current.map { [$0.id] } ?? [],
                reader: reader, at: capture.at, passId: passID,
                positions: context.positions, positionObservations: context.positionObservations, canvasWidth: canvasWidth,
                canvasHeight: canvasHeight, strokes: points, nativeDrawing: nativeDrawing, canvasSpace: canvasSpace)
            let creation = noteCreation(capture)
            if let creation { noteCreations[version.id] = creation }
            if journal != nil {
                let encoder = JSONEncoder(); encoder.keyEncodingStrategy = .convertToSnakeCase
                let value = try JSONSerialization.jsonObject(with: encoder.encode(version))
                var payload: [String: Any] = ["notebook_id": notebookID, "block_id": blockID, "version": value]
                if let creation { payload["note_creation"] = try creation.dictionary }
                records[version.id] = try InkJournalRecord(id: version.id, kind: "notebook", payload: payload)
            }
            pending.append(version); selectedVersionID = version.id; stage(version.id); changed()
            if saveError == nil { startWriter() }
            return true
        } catch { report("Ink is not fully saved: \(bookNookErrorMessage(error))"); return false }
    }
    func retry() { saveError = nil; for version in pending { stage(version.id) }; startWriter() }
    private func stage(_ id: String) {
        guard let journal, let record = records[id], !journaledIDs.contains(id), staging[id] == nil else { return }
        staging[id] = Task {
            do { try await journal.store(record); journaledIDs.insert(id); changed() }
            catch { report("Ink recovery has not been saved to this device: \(bookNookErrorMessage(error))"); throw error }
        }
    }
    func checkpoint() async throws {
        capturePendingStroke?()
        for version in pending {
            stage(version.id)
            if let task = staging[version.id] {
                do { try await task.value; staging.removeValue(forKey: version.id) }
                catch { staging.removeValue(forKey: version.id); throw error }
            }
        }
        guard allPendingJournaled else { throw InkJournalFailure("Some handwriting has not reached the local recovery journal. Keep Book Nook open and retry.") }
    }
    func recover(_ record: InkJournalRecord) throws {
        guard record.kind == "notebook", let payload = record.payload.object,
              payload["notebook_id"] as? String == notebookID, payload["block_id"] as? String == blockID,
              let value = payload["version"] else { throw CocoaError(.coderReadCorrupt) }
        let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase
        let version = try decoder.decode(NotebookInkVersion.self, from: JSONSerialization.data(withJSONObject: value))
        guard version.id == record.id, version.passId == passID, version.reader == reader,
              version.canvasWidth.isFinite, version.canvasHeight.isFinite,
              version.canvasWidth > 0, version.canvasHeight > 0 else { throw CocoaError(.coderReadCorrupt) }
        if pending.contains(where: { $0.id == record.id }) {
            guard let original = records[record.id], try original.encoded() == record.encoded() else {
                throw InkJournalFailure("A recovered drawing reuses an existing capture ID with different content. Both originals were retained.")
            }
            return
        }
        if let origin = payload["note_creation"] {
            let creation = try decoder.decode(NotebookNoteCreation.self, from: JSONSerialization.data(withJSONObject: origin))
            guard creation.captureContext.passId == passID else { throw CocoaError(.coderReadCorrupt) }
            noteCreations[record.id] = creation
        }
        records[record.id] = record; journaledIDs.insert(record.id)
        pending.append(version); selectedVersionID = version.id; changed(); startWriter()
    }
    func flush() async throws {
        capturePendingStroke?()
        if writer == nil && saveError == nil { startWriter() }
        while let task = writer { await task.value }
        if let saveError { throw AuthorizationError.message(saveError) }
        if !pending.isEmpty { throw AuthorizationError.message("Some handwriting is still pending. Retry save before continuing.") }
    }
    private func report(_ message: String) { saveError = message; changed(); failed(message) }
    private func startWriter() {
        guard writer == nil, !pending.isEmpty else { return }
        writer = Task {
            while let version = pending.first {
                do {
                    if let task = staging[version.id] { try await task.value; staging.removeValue(forKey: version.id) }
                    var command: [String: Any] = ["op": "set_notebook_ink", "notebook_id": notebookID, "block_id": blockID,
                        "version_id": version.id, "parents": version.parents, "at": version.at,
                        "canvas_width": version.canvasWidth, "canvas_height": version.canvasHeight,
                        "strokes": version.strokes.map(\.dictionary), "native_drawing": version.nativeDrawing as Any? ?? NSNull(),
                        "capture_context": try NotebookCaptureContext(passId: version.passId, positions: version.positions,
                            positionObservations: version.positionObservations ?? []).dictionary]
                    if let space = version.canvasSpace { command["canvas_space"] = space.dictionary }
                    if let creation = noteCreations[version.id] { command["note_creation"] = try creation.dictionary }
                    try await persist(command)
                    // The owning model normally accepts the durable response before
                    // this acknowledgement. Keep the local version if a consumer
                    // supplies a persistence closure without a state refresh.
                    if !(ink?.versions.contains { $0.id == version.id } ?? false) {
                        ink = NotebookInk(schema: "booknook.notebook-ink.v1", versions: (ink?.versions ?? []) + [version], transcriptions: ink?.transcriptions ?? [])
                    }
                    if let journal, let record = records[version.id] { try await journal.remove(record) }
                    // Remove only the acknowledged ID, never a newer queued drawing.
                    pending.removeAll { $0.id == version.id }; records.removeValue(forKey: version.id)
                    noteCreations.removeValue(forKey: version.id)
                    journaledIDs.remove(version.id); saveError = nil; changed()
                } catch { staging.removeValue(forKey: version.id); report("Ink is not fully saved: \(bookNookErrorMessage(error))"); break }
            }
            objectWillChange.send(); writer = nil; changed()
        }
    }
}
