// SPDX-License-Identifier: MPL-2.0
import SwiftUI
import Combine
import CoreFoundation
import WebKit

struct ReaderInkContext: Codable, Equatable {
    let resource: String
    var page: Int? = nil
    var documentURL: String? = nil
    var contentFingerprint: String? = nil
    var layoutFingerprint: String? = nil
    var measured = true
}

struct ReaderInkResolution {
    let anchor: [String: Any]
    let geometry: [String: Any]
    let originalContext: String
}

struct ReaderInkDraft: Identifiable {
    let id: String
    let context: ReaderInkContext
    let bounds: CGRect
    let strokes: [[String: Any]]
    let nativeDrawing: String
    var error: String?
    fileprivate var resolution: ReaderInkResolution?
    fileprivate var initialResolver: ReaderInkResolver?
    fileprivate var saveUnplaced = false
    var canSaveUnplaced: Bool { error != nil && resolution == nil && !saveUnplaced }
    init(id: String, context: ReaderInkContext, bounds: CGRect, strokes: [[String: Any]], nativeDrawing: String,
         error: String? = nil, resolution: ReaderInkResolution? = nil, initialResolver: ReaderInkResolver? = nil) {
        self.id = id; self.context = context; self.bounds = bounds; self.strokes = strokes; self.nativeDrawing = nativeDrawing
        self.error = error; self.resolution = resolution; self.initialResolver = initialResolver
    }
}

typealias ReaderInkResolver = @MainActor (ReaderInkDraft) async throws -> ReaderInkResolution

/// The desk owns pending captures. A temporary EPUB/PDF view never owns the only
/// copy of a completed stroke, including while an anchor or save is asynchronous.
@MainActor final class ReaderInkSession: ObservableObject {
    let passID: String
    let editionID: String
    let assetFingerprint: String
    @Published private(set) var drafts: [ReaderInkDraft] = []
    @Published private(set) var isSaving = false
    var pendingCount: Int { drafts.count }
    var recoveryIdentity: String { "\(passID):\(editionID):\(assetFingerprint)" }
    var error: String? { drafts.compactMap(\.error).first }
    var capturePendingInk: [UUID: () -> Void] = [:]
    private var resolvers: [String: (UUID, ReaderInkResolver)] = [:]
    private var writer: Task<Void, Never>?
    private let persist: ([String: Any]) async throws -> Void
    private let journal: InkJournal?
    private var records: [String: InkJournalRecord] = [:]
    private var previousCaptureID: String?
    private var staging: [String: Task<Void, Error>] = [:]
    @Published private(set) var journaledIDs: Set<String> = []
    var allPendingJournaled: Bool { drafts.allSatisfy { journaledIDs.contains($0.id) } }
    private let changed: () -> Void
    private let failed: (String) -> Void

    init(passID: String, editionID: String, assetFingerprint: String,
         persist: @escaping ([String: Any]) async throws -> Void, journal: InkJournal? = nil,
         changed: @escaping () -> Void = {}, failed: @escaping (String) -> Void = { _ in }) {
        self.passID = passID; self.editionID = editionID; self.assetFingerprint = assetFingerprint
        self.persist = persist; self.changed = changed; self.failed = failed; self.journal = journal
    }
    func registerResolver(owner: UUID, resource: String, resolver: @escaping ReaderInkResolver) { resolvers[resource] = (owner, resolver) }
    func unregister(owner: UUID) {
        capturePendingInk.removeValue(forKey: owner)
        resolvers = resolvers.filter { $0.value.0 != owner }
    }
    @discardableResult func capture(context: ReaderInkContext, bounds: CGRect, strokes: [[String: Any]], nativeDrawing: String,
                                   resolution: ReaderInkResolution? = nil, resolver: ReaderInkResolver? = nil) -> String {
        let id = UUID().uuidString
        drafts.append(ReaderInkDraft(id: id, context: context, bounds: bounds, strokes: strokes,
                                    nativeDrawing: nativeDrawing, resolution: resolution, initialResolver: resolver))
        if journal != nil {
            do {
                let contextValue = try JSONSerialization.jsonObject(with: JSONEncoder().encode(context))
                var payload: [String: Any] = ["pass_id": passID, "edition_id": editionID, "asset_fingerprint": assetFingerprint,
                    "context": contextValue, "bounds": [bounds.minX, bounds.minY, bounds.width, bounds.height],
                    "strokes": strokes, "native_drawing": nativeDrawing,
                    "previous_capture_id": previousCaptureID as Any? ?? NSNull()]
                if let resolution { payload["resolution"] = ["anchor": resolution.anchor, "geometry": resolution.geometry, "original_context": resolution.originalContext] }
                records[id] = try InkJournalRecord(id: id, kind: "reader", payload: payload); stage(id)
            } catch { drafts[drafts.count - 1].error = "Ink recovery is not saved: \(bookNookErrorMessage(error))"; failed(drafts.last!.error!) }
        }
        previousCaptureID = id; changed(); startWriter(); return id
    }
    func captureAll() { for capture in Array(capturePendingInk.values) { capture() } }
    private func stage(_ id: String) {
        guard let journal, let record = records[id], !journaledIDs.contains(id), staging[id] == nil else { return }
        staging[id] = Task {
            do { try await journal.store(record); journaledIDs.insert(id); changed() }
            catch {
                if let index = drafts.firstIndex(where: { $0.id == id }) {
                    drafts[index].error = "Ink recovery has not reached this device: \(bookNookErrorMessage(error))"; failed(drafts[index].error!)
                }
                changed(); throw error
            }
        }
    }
    func checkpoint() async throws {
        captureAll()
        for draft in drafts {
            stage(draft.id)
            if let task = staging[draft.id] {
                do { try await task.value; staging.removeValue(forKey: draft.id) }
                catch { staging.removeValue(forKey: draft.id); throw error }
            }
        }
        guard allPendingJournaled else { throw InkJournalFailure("Some reader markup has not reached local recovery. Keep Book Nook open and retry.") }
    }
    func recover(_ record: InkJournalRecord) throws {
        guard record.kind == "reader", let payload = record.payload.object,
              payload["pass_id"] as? String == passID, payload["edition_id"] as? String == editionID,
              payload["asset_fingerprint"] as? String == assetFingerprint,
              let contextValue = payload["context"], let coordinates = payload["bounds"] as? [NSNumber], coordinates.count == 4,
              coordinates.allSatisfy({ CFGetTypeID($0) != CFBooleanGetTypeID() && $0.doubleValue.isFinite }),
              coordinates[2].doubleValue > 0, coordinates[3].doubleValue > 0,
              let strokes = payload["strokes"] as? [[String: Any]],
              let bytes = payload["native_drawing"] as? String, Data(base64Encoded: bytes) != nil else { throw CocoaError(.coderReadCorrupt) }
        let bounds = coordinates.map(\.doubleValue)
        let context = try JSONDecoder().decode(ReaderInkContext.self, from: JSONSerialization.data(withJSONObject: contextValue))
        guard !context.resource.isEmpty, context.resource.utf8.count <= 8192 else { throw CocoaError(.coderReadCorrupt) }
        var resolution: ReaderInkResolution?
        if let value = payload["resolution"] as? [String: Any] {
            guard let anchor = value["anchor"] as? [String: Any], let geometry = value["geometry"] as? [String: Any],
                  let original = value["original_context"] as? String else { throw CocoaError(.coderReadCorrupt) }
            resolution = ReaderInkResolution(anchor: anchor, geometry: geometry, originalContext: original)
        }
        let draft = ReaderInkDraft(id: record.id, context: context,
            bounds: CGRect(x: bounds[0], y: bounds[1], width: bounds[2], height: bounds[3]), strokes: strokes,
            nativeDrawing: bytes, error: resolution == nil ? "Recovered original markup. Open its original chapter and retry, or save it as unplaced markup." : nil, resolution: resolution)
        if let resolution { try validate(resolution, for: draft) }
        guard !drafts.contains(where: { $0.id == record.id }) else { return }
        records[record.id] = record; journaledIDs.insert(record.id); previousCaptureID = record.id; drafts.append(draft); changed(); startWriter()
    }
    private func acknowledge(_ draft: ReaderInkDraft) async throws {
        if let journal, let record = records[draft.id] { try await journal.remove(record) }
        drafts.removeAll { $0.id == draft.id }; records.removeValue(forKey: draft.id)
        journaledIDs.remove(draft.id); changed()
    }

    func retry() {
        for index in drafts.indices { drafts[index].error = nil; stage(drafts[index].id) }
        changed(); startWriter()
    }
    func saveAsUnplaced(id: String) {
        guard !isSaving, let index = drafts.firstIndex(where: { $0.id == id }), drafts[index].canSaveUnplaced else { return }
        drafts[index].saveUnplaced = true; drafts[index].error = nil; drafts[index].initialResolver = nil
        changed(); startWriter()
    }
    func flush() async throws {
        captureAll(); startWriter()
        while let writer { await writer.value }
        if !drafts.isEmpty { throw AuthorizationError.message(error ?? "Reader markup is still pending. Return to its original page and retry before continuing.") }
    }
    private func startWriter() {
        guard writer == nil, drafts.contains(where: { $0.error == nil }) else { return }
        isSaving = true; changed()
        writer = Task {
            while let original = drafts.first(where: { $0.error == nil }) {
                do {
                    if let task = staging[original.id] { try await task.value; staging.removeValue(forKey: original.id) }
                    if journal != nil && !journaledIDs.contains(original.id) { throw InkJournalFailure("The original capture is not yet in local recovery.") }
                    if original.saveUnplaced {
                        try await persist(["op": "save_annotation", "annotation": try unplacedAnnotation(original)])
                        try await acknowledge(original); continue
                    }
                    var resolved = original.resolution
                    if resolved == nil {
                        guard original.context.measured else { throw AuthorizationError.message("The reader had not measured this page. Its original ink is retained, but it cannot be placed automatically.") }
                        guard let resolver = original.initialResolver ?? resolvers[original.context.resource]?.1 else {
                            throw AuthorizationError.message("Open the original chapter and retry. The captured ink has not been placed on another page.")
                        }
                        resolved = try await resolver(original)
                    }
                    guard let resolved else { throw CocoaError(.coderReadCorrupt) }
                    try validate(resolved, for: original)
                    guard let index = drafts.firstIndex(where: { $0.id == original.id }) else { continue }
                    drafts[index].resolution = resolved; drafts[index].initialResolver = nil
                    var anchor = resolved.anchor; anchor["edition_id"] = editionID
                    let annotation: [String: Any] = ["id": original.id, "pass_id": passID, "anchor": anchor,
                        "original_context": resolved.originalContext, "layout_fingerprint": original.context.layoutFingerprint as Any? ?? NSNull(),
                        "strokes": original.strokes, "native_drawing": original.nativeDrawing, "ink_geometry": resolved.geometry]
                    try await persist(["op": "save_annotation", "annotation": annotation])
                    try await acknowledge(original)
                } catch {
                    staging.removeValue(forKey: original.id)
                    if let index = drafts.firstIndex(where: { $0.id == original.id }) {
                        let message = "Reader markup is not fully saved: \(bookNookErrorMessage(error))"
                        drafts[index].error = message; drafts[index].initialResolver = nil
                        changed(); failed(message)
                    }
                }
            }
            writer = nil; isSaving = false; changed()
        }
    }
    private func unplacedAnnotation(_ draft: ReaderInkDraft) throws -> [String: Any] {
        let anchor: [String: Any]
        if let page = draft.context.page { anchor = ["kind": "page", "edition_id": editionID, "page": page] }
        else {
            let value: [String: Any] = ["schema": "booknook.reader-unplaced.v1", "edition_id": editionID,
                "asset_fingerprint": assetFingerprint, "resource": draft.context.resource,
                "document_url": draft.context.documentURL as Any? ?? NSNull(),
                "content_fingerprint": draft.context.contentFingerprint as Any? ?? NSNull(),
                "layout_fingerprint": draft.context.layoutFingerprint as Any? ?? NSNull(),
                "coordinate_space": draft.context.measured ? "epub_document" : "reader_view_unmeasured"]
            let encoded = try JSONSerialization.data(withJSONObject: value, options: .sortedKeys)
            anchor = ["kind": "provider", "provider": "booknook.reader.unplaced", "source_id": assetFingerprint,
                      "value": String(decoding: encoded, as: UTF8.self)]
        }
        return ["id": draft.id, "pass_id": passID, "anchor": anchor,
                "original_context": "Unplaced markup — \(draft.context.page.map { "PDF page \($0)" } ?? draft.context.resource). Original ink retained; no passage or placement was inferred.",
                "layout_fingerprint": draft.context.layoutFingerprint ?? "unplaced:\(assetFingerprint)",
                "strokes": draft.strokes, "native_drawing": draft.nativeDrawing, "ink_geometry": NSNull()]
    }
    private func validate(_ value: ReaderInkResolution, for draft: ReaderInkDraft) throws {
        let context = draft.context
        guard value.geometry["resource"] as? String == context.resource,
              value.geometry["content_fingerprint"] as? String == context.contentFingerprint,
              (value.anchor["edition_id"] as? String).map({ $0 == editionID }) ?? true else {
            throw AuthorizationError.message("The source changed before the ink could be anchored. The original capture remains retained.")
        }
        if let page = context.page {
            guard value.geometry["space"] as? String == "pdf_page", value.anchor["kind"] as? String == "page",
                  (value.anchor["page"] as? NSNumber)?.intValue == page,
                  (value.geometry["page"] as? NSNumber)?.intValue == page,
                  context.contentFingerprint == assetFingerprint else { throw CocoaError(.coderReadCorrupt) }
        } else {
            guard value.geometry["space"] as? String == "epub_document", value.anchor["kind"] as? String == "text",
                  value.anchor["resource"] as? String == context.resource else { throw CocoaError(.coderReadCorrupt) }
        }
    }
}

struct ReaderInkPendingView: View {
    @ObservedObject var session: ReaderInkSession
    var body: some View {
        if !session.drafts.isEmpty {
            DisclosureGroup("\(session.pendingCount) reader markup capture(s) pending") {
                VStack(alignment: .leading, spacing: 12) {
                    Text(session.allPendingJournaled ? "Original ink is saved in local recovery on this device. Resolve or save it as unplaced markup to include it in portable backups." : "Saving originals to local recovery… Keep Book Nook open until this completes. Pending captures are not yet in portable backups.").font(.caption)
                    ForEach(session.drafts) { draft in
                        VStack(alignment: .leading, spacing: 6) {
                            Text(draft.context.page.map { "PDF page \($0)" } ?? draft.context.resource).font(.caption.bold())
                            ReaderInkDraftPreview(draft: draft).frame(height: 120)
                            if let error = draft.error { Text(error).font(.caption).foregroundStyle(.red).textSelection(.enabled) }
                            if draft.canSaveUnplaced {
                                Button("Save as unplaced markup") { session.saveAsUnplaced(id: draft.id) }.disabled(session.isSaving)
                                Text("Keep the original ink in your notebook backup without placing it over book text.").font(.caption2).foregroundStyle(.secondary)
                            }
                        }
                    }
                    Button("Retry pending markup") { session.retry() }.disabled(session.isSaving)
                }
            }.font(.caption).padding(.horizontal, 18).padding(.bottom, 12)
        }
    }
}

struct ReaderInkDraftPreview: View {
    let draft: ReaderInkDraft
    var saved = false
    var body: some View {
        Canvas { context, size in
            let box = draft.bounds.insetBy(dx: -4, dy: -4)
            guard box.width > 0, box.height > 0, box.width.isFinite, box.height.isFinite else { return }
            let scale = min(size.width / box.width, size.height / box.height)
            context.scaleBy(x: scale, y: scale); context.translateBy(x: -box.minX, y: -box.minY)
            for stroke in draft.strokes {
                guard let points = stroke["points"] as? [[String: Any]], let first = points.first,
                      let x = (first["x"] as? NSNumber)?.doubleValue, let y = (first["y"] as? NSNumber)?.doubleValue else { continue }
                let width = (stroke["width"] as? NSNumber)?.doubleValue ?? 3
                let hex = stroke["color"] as? String ?? ""
                let rgb = hex.count == 7 && hex.first == "#" ? UInt32(hex.dropFirst(), radix: 16) ?? 0x283c34 : 0x283c34
                let color = Color(red: Double((rgb >> 16) & 255)/255, green: Double((rgb >> 8) & 255)/255, blue: Double(rgb & 255)/255)
                if points.count == 1 { context.fill(Path(ellipseIn: CGRect(x: x-width/2, y: y-width/2, width: width, height: width)), with: .color(color)); continue }
                var path = Path(); path.move(to: CGPoint(x: x, y: y))
                for point in points.dropFirst() { if let x = (point["x"] as? NSNumber)?.doubleValue, let y = (point["y"] as? NSNumber)?.doubleValue { path.addLine(to: CGPoint(x: x, y: y)) } }
                context.stroke(path, with: .color(color), style: StrokeStyle(lineWidth: width, lineCap: .round))
            }
        }.background(readingPaper).clipped().accessibilityLabel("\(saved ? "Saved unplaced markup" : "Retained unsaved markup"), \(draft.strokes.count) strokes")
    }
}

/// Versioned private source metadata, using the existing provider-location
/// contract. A locator is not a quote and must never enter text placement.
enum ReaderUnplacedInkLocation {
    static func matches(_ annotation: Annotation, editionID: String) -> Bool { payload(annotation)?["edition_id"] as? String == editionID }
    static func resource(_ annotation: Annotation) -> String? { payload(annotation)?["resource"] as? String }
    private static func payload(_ annotation: Annotation) -> [String: Any]? {
        guard let anchor = annotation.anchor.object, anchor["kind"] as? String == "provider",
              anchor["provider"] as? String == "booknook.reader.unplaced", let value = anchor["value"] as? String,
              value.utf8.count <= 16_384, annotation.inkGeometry == nil,
              let object = try? JSONSerialization.jsonObject(with: Data(value.utf8)) as? [String: Any],
              object["schema"] as? String == "booknook.reader-unplaced.v1",
              Set(object.keys) == Set(["schema", "edition_id", "asset_fingerprint", "resource", "document_url", "content_fingerprint", "layout_fingerprint", "coordinate_space"]),
              let edition = object["edition_id"] as? String, valid(edition, limit: 512),
              let resource = object["resource"] as? String, valid(resource, limit: 4096),
              let fingerprint = object["asset_fingerprint"] as? String, valid(fingerprint, limit: 4096),
              fingerprint == anchor["source_id"] as? String,
              ["epub_document", "reader_view_unmeasured"].contains(object["coordinate_space"] as? String ?? "") else { return nil }
        for key in ["document_url", "content_fingerprint", "layout_fingerprint"] {
            guard object[key] is NSNull || (object[key] as? String).map({ valid($0, limit: 4096) }) == true else { return nil }
        }
        return object
    }
    private static func valid(_ value: String, limit: Int) -> Bool {
        !value.isEmpty && value.utf8.count <= limit && !value.unicodeScalars.contains { $0.value < 32 || $0.value == 127 }
    }
}

struct ReaderSavedInkPreview: View {
    let annotation: Annotation
    var body: some View {
        let strokes = annotation.strokes.compactMap(\.object)
        let points = strokes.flatMap { $0["points"] as? [[String: Any]] ?? [] }
        let xs = points.compactMap { ($0["x"] as? NSNumber)?.doubleValue }
        let ys = points.compactMap { ($0["y"] as? NSNumber)?.doubleValue }
        let bounds = CGRect(x: xs.min() ?? 0, y: ys.min() ?? 0, width: (xs.max() ?? 1) - (xs.min() ?? 0), height: (ys.max() ?? 1) - (ys.min() ?? 0))
        ReaderInkDraftPreview(draft: ReaderInkDraft(id: annotation.id,
            context: ReaderInkContext(resource: ReaderUnplacedInkLocation.resource(annotation) ?? "book.pdf"),
            bounds: bounds, strokes: strokes, nativeDrawing: annotation.nativeDrawing ?? ""), saved: true)
    }
}

/// Shared native/WebKit resolver: source and layout checks execute in the same
/// DOM evaluation as anchor discovery, so a chapter navigation cannot retarget ink.
@MainActor enum ReaderInkAnchorResolver {
    static func resolve(_ draft: ReaderInkDraft, in web: WKWebView) async throws -> ReaderInkResolution {
        let r = draft.bounds
        let input: [String: Any] = ["bounds": ["x": r.minX, "y": r.minY, "width": r.width, "height": r.height],
            "resource": draft.context.resource, "url": draft.context.documentURL ?? "", "layout": draft.context.layoutFingerprint ?? ""]
        let encoded = try JSONSerialization.data(withJSONObject: input)
        let json = String(decoding: encoded, as: UTF8.self)
        let script = """
        (()=>{const c=\(json);if(!c.url||location.href!==new URL(c.url).href)return {status:'source_changed'};
        const r=window.booknookInkAnchor?.(c.bounds,c.resource);if(r?.status!=='resolved')return r;
        const layout=[location.pathname,innerWidth,innerHeight,getComputedStyle(document.body).fontSize,document.documentElement.scrollHeight,r.geometry.content_fingerprint].join('|');
        return layout===c.layout?r:{status:'layout_changed'};})()
        """
        let result = try await web.evaluateJavaScript(script)
        guard let result = result as? [String: Any], result["status"] as? String == "resolved",
              let geometry = result["geometry"] as? [String: Any], var anchor = result["anchor"] as? [String: Any] else {
            throw AuthorizationError.message("The original passage and layout could not be confirmed. Return to that chapter with its original text size and retry. Image-only EPUB markup remains unresolved; the original ink is retained.")
        }
        anchor["kind"] = "text"; anchor["resource"] = draft.context.resource
        return ReaderInkResolution(anchor: anchor, geometry: geometry, originalContext: anchor["quote"] as? String ?? "EPUB markup")
    }
}
