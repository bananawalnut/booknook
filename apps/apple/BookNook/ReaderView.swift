// SPDX-License-Identifier: MPL-2.0
import SwiftUI
import WebKit
import PDFKit
import UniformTypeIdentifiers
#if os(iOS)
import PencilKit
#endif

struct ReaderPane: View {
    @EnvironmentObject var model: DeskModel
    @State var importing = false
    @State var root: URL?
    @State var chapter = 0
    @State var markup = false
    @AppStorage("booknook.readerFontSize") var fontSize = 19
    @State var fingerprint = ""
    @State var geometry: [String: Any] = [:]
    @State var placements: [String: String] = [:]
    @State var pdfHighlights: [[String: Any]] = []
    @State var lastRecordedPosition = ""
    @State private var destination: ReaderDestination?
    @State private var navigationStatus: String?
    @State private var savingHighlight = false
    @State private var highlightStatus: String?
    @State private var showingAnnotations = false
    var edition: Edition? { model.book?.editionIds.last.flatMap { model.state?.editions[$0] } }
    var asset: Asset? { edition?.assets.last }
    var annotations: [Annotation] { model.state?.annotations.values.filter { $0.passId == model.notebook?.passId && (($0.anchor.object?["edition_id"] as? String) == edition?.id || ReaderUnplacedInkLocation.matches($0, editionID: edition?.id ?? "")) } ?? [] }
    var savedPosition: [String: Any]? { model.state?.positions.values.filter { $0.passId == model.notebook?.passId && ($0.location.object?["edition_id"] as? String) == edition?.id }.max { $0.observedAt < $1.observedAt }?.location.object }
    var currentResource: String { guard let asset, asset.spine.indices.contains(chapter) else { return "" }; return asset.spine[chapter] }
    var inkSession: ReaderInkSession? {
        guard let passID = model.notebook?.passId, let edition, let asset else { return nil }
        return model.readerInkSession(passID: passID, editionID: edition.id, assetFingerprint: asset.sha256)
    }
    var messageReceiver: ([String: Any]) -> Void {
        guard let passID = model.notebook?.passId, let edition, let asset else { return { _ in } }
        let source = ReaderSourceContext(passID: passID, editionID: edition.id, assetID: asset.id,
            fingerprint: asset.sha256, resource: asset.mediaType == "application/pdf" ? "book.pdf" : currentResource)
        return model.readerMessages(from: source,
            currentResource: { self.asset?.mediaType == "application/pdf" ? "book.pdf" : currentResource },
            receive: handle)
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                ViewThatFits(in: .horizontal) {
                    Text("Book").font(.system(.subheadline, design: .serif)).foregroundStyle(.secondary).fixedSize()
                    Color.clear.frame(width: 0, height: 0)
                }
                Spacer(minLength: 0)
                if asset != nil && !highlightRequests.isEmpty {
                    Button { saveHighlight() } label: { Image(systemName: selectionAlreadyHighlighted ? "checkmark" : "highlighter").accessibilityLabel(selectionAlreadyHighlighted ? "Selection already highlighted" : "Save selected highlight") }.disabled(savingHighlight || highlightRequests.isEmpty || selectionAlreadyHighlighted).help(selectionAlreadyHighlighted ? "This selection is already highlighted" : "Save selected highlight")
                }
                Menu {
                    if asset?.mediaType == "application/epub+zip" {
                        Button("Larger text") { fontSize = min(34, fontSize + 2) }
                        Button("Smaller text") { fontSize = max(13, fontSize - 2) }
                        Button("Default text size") { fontSize = 19 }
                        Divider()
                    }
                    #if os(iOS)
                    if asset != nil { Toggle("Pencil markup", isOn: $markup) }
                    #endif
                    Button("Saved annotations (\(annotations.count))") { showingAnnotations = true }.disabled(annotations.isEmpty)
                    Button("Attach EPUB or PDF…") { importing = true }
                } label: { Image(systemName: "ellipsis").frame(width: 32, height: 32).accessibilityLabel("Book options") }
            }.buttonStyle(.borderless).padding(.horizontal, 12).padding(.vertical, 10)
            if let status = highlightStatus ?? navigationStatus {
                Text(status).font(.caption).foregroundStyle(.secondary).padding(.horizontal, 24).padding(.bottom, 8).accessibilityLabel(status)
            }
            if let asset, let root {
                if asset.mediaType == "application/pdf" {
                    Group {
                    #if os(iOS)
                    NativePDFView(url: root.appendingPathComponent("book.pdf"), initialPage: (savedPosition?["page"] as? NSNumber)?.intValue, fingerprint: asset.sha256, markup: markup, annotations: annotations, onMessage: messageReceiver, onInk: inkReceiver, inkSession: inkSession)
                    #else
                    NativePDFView(url: root.appendingPathComponent("book.pdf"), initialPage: (savedPosition?["page"] as? NSNumber)?.intValue, fingerprint: asset.sha256, markup: markup, annotations: annotations, onMessage: messageReceiver, onInk: inkReceiver)
                    #endif
                    }
                    .overlay { readingPaper.blendMode(.darken).allowsHitTesting(false).accessibilityHidden(true) }
                    .compositingGroup()
                }
                else {
                    EPUBView(root: root, resource: currentResource, markup: markup, annotations: annotations, geometry: geometry, fontSize: fontSize, restorePosition: savedPosition, spine: asset.spine, destination: destination, onMessage: messageReceiver, onInk: inkReceiver, inkSession: inkSession)
                    ViewThatFits(in: .horizontal) {
                      HStack {
                        Button { openChapter(chapter - 1) } label: { Image(systemName: "chevron.left").frame(width: 32, height: 32).accessibilityLabel("Previous chapter") }.disabled(chapter == 0)
                        Spacer()
                        Menu {
                            ForEach(Array(asset.spine.enumerated()), id: \.offset) { index, resource in
                                Button { openChapter(index) } label: {
                                    if index == chapter { Label(chapterLabel(resource), systemImage: "checkmark") }
                                    else { Text(chapterLabel(resource)) }
                                }
                            }
                        } label: { Text("Chapter \(chapter + 1) of \(asset.spine.count)").font(.caption).fixedSize() }
                        .accessibilityLabel("Choose chapter, currently \(chapter + 1) of \(asset.spine.count)")
                        Spacer()
                        Button { openChapter(chapter + 1) } label: { Image(systemName: "chevron.right").frame(width: 32, height: 32).accessibilityLabel("Next chapter") }.disabled(chapter + 1 >= asset.spine.count)
                      }
                      Menu {
                          Button("Previous chapter") { openChapter(chapter - 1) }.disabled(chapter == 0)
                          ForEach(Array(asset.spine.enumerated()), id: \.offset) { index, resource in
                              Button { openChapter(index) } label: {
                                  if index == chapter { Label(chapterLabel(resource), systemImage: "checkmark") }
                                  else { Text(chapterLabel(resource)) }
                              }
                          }
                          Button("Next chapter") { openChapter(chapter + 1) }.disabled(chapter + 1 >= asset.spine.count)
                      } label: { Text("\(chapter + 1) / \(asset.spine.count)").font(.caption).frame(maxWidth: .infinity) }
                      .accessibilityLabel("Choose chapter, currently \(chapter + 1) of \(asset.spine.count)")
                    }.buttonStyle(.borderless).padding(.horizontal, 14).padding(.vertical, 6)
                }
            } else {
                VStack(alignment: .leading, spacing: 18) {
                    Text("Your book goes here.").font(.system(.title2, design: .serif))
                    Text("Open an EPUB or PDF when you’re ready.").font(.callout).foregroundStyle(.secondary)
                    Button("Choose a book file") { importing = true }.buttonStyle(.bordered)
                }.padding(24).frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            if let inkSession { ReaderInkPendingView(session: inkSession) }
        }
        .background(readingPaper).accessibilityIdentifier("book-pane")
        .sheet(isPresented: $showingAnnotations) {
            VStack(alignment: .leading, spacing: 20) {
                HStack { Text("Saved annotations").font(.title2); Spacer(); Button("Done") { showingAnnotations = false } }
                ScrollView {
                    VStack(alignment: .leading, spacing: 18) {
                        ForEach(annotations.sorted { $0.id < $1.id }) { annotation in annotationRow(annotation) }
                    }.frame(maxWidth: .infinity, alignment: .leading)
                }
            }.padding(24).frame(minWidth: 300, idealWidth: 460, minHeight: 300)
        }
        .task(id: "\(asset?.id ?? ""):\(model.notebook?.passId ?? "")") {
            chapter = 0; root = nil; destination = nil; resetGeometry()
            if let asset {
                if let resource = savedPosition?["resource"] as? String, let restored = asset.spine.firstIndex(of: resource) { chapter = restored }
                do {
                    let directory = try await model.repository.assetDirectory(asset.id)
                    try Task.checkCancellation()
                    root = directory
                } catch { if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } }
            }
        }
        .fileImporter(isPresented: $importing, allowedContentTypes: [.pdf, UTType(filenameExtension: "epub") ?? .data]) { result in
            Task { do { let url = try result.get(); let access = url.startAccessingSecurityScopedResource(); defer { if access { url.stopAccessingSecurityScopedResource() } }; if let book = model.book { try await model.drainSaves(); model.accept(try await model.repository.importContent(book: book.id, url: url)) } } catch { if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } } }
        }
    }
    func resetGeometry() { fingerprint = ""; geometry = [:]; placements = [:]; pdfHighlights = []; lastRecordedPosition = ""; highlightStatus = nil; navigationStatus = nil; model.selection = nil }
    func chapterLabel(_ resource: String) -> String {
        let number = asset?.spine.firstIndex(of: resource).map { $0 + 1 }
        let filename = (resource as NSString).lastPathComponent.removingPercentEncoding ?? resource
        return number.map { "Chapter \($0) · \(filename)" } ?? filename
    }
    func openChapter(_ index: Int, fragment: String? = nil, anchor: [String: Any]? = nil) {
        guard let asset, asset.spine.indices.contains(index) else { return }
        resetGeometry(); chapter = index
        destination = ReaderDestination(resource: asset.spine[index], fragment: fragment, anchor: anchor)
    }
    @ViewBuilder func annotationRow(_ annotation: Annotation) -> some View {
        let unplacedResource = ReaderUnplacedInkLocation.resource(annotation)
        let resource = annotation.anchor.object?["resource"] as? String ?? unplacedResource ?? ""
        let index = asset?.spine.firstIndex(of: resource)
        let page = (annotation.anchor.object?["page"] as? NSNumber)?.intValue
        let ink = annotation.nativeDrawing != nil || !annotation.strokes.isEmpty
        let unplaced = ink && annotation.inkGeometry == nil
        VStack(alignment: .leading, spacing: 3) {
            if let page {
                Label(annotation.originalContext, systemImage: ink ? "pencil.tip" : "highlighter")
                    .lineLimit(3).frame(maxWidth: .infinity, alignment: .leading)
                Text("Saved on PDF page \(page)").font(.caption2).foregroundStyle(.secondary)
            } else {
                Button {
                    if let index { openChapter(index, anchor: unplacedResource == nil ? annotation.anchor.object : nil); showingAnnotations = false }
                } label: {
                    Label(annotation.originalContext, systemImage: ink ? "pencil.tip" : "highlighter").lineLimit(3).frame(maxWidth: .infinity, alignment: .leading)
                }.buttonStyle(.borderless).disabled(index == nil).help(unplaced ? "Open the original chapter" : "Return to this saved passage")
                Text(index == nil ? "Original source is not in this attached book" : "Saved in \(chapterLabel(resource))").font(.caption2).foregroundStyle(.secondary)
            }
            if unplaced {
                Text("Unplaced markup. Original ink is saved; its position over the page could not be confirmed.").font(.caption2).foregroundStyle(.secondary)
                ReaderSavedInkPreview(annotation: annotation).frame(height: 120)
            } else if ink && resource == currentResource && placements[annotation.id] != "resolved" {
                Text("Markup is retained with its original context. Its text layout has changed or its passage cannot be uniquely located.").font(.caption2).foregroundStyle(.secondary)
            }
        }
    }
    func handle(_ message: [String: Any]) {
        if let source = message["assetFingerprint"] as? String, source != asset?.sha256 { return }
        if message["type"] as? String == "error" { model.error = message["message"] as? String ?? "Reader unavailable"; return }
        guard let edition else { return }
        switch message["type"] as? String {
        case "selection":
            highlightStatus = nil
            guard let quote = message["quote"] as? String, !quote.isEmpty else { return }
            model.selection = ["kind": "text", "edition_id": edition.id, "resource": currentResource, "quote": quote, "prefix": message["prefix"] as? String ?? "", "suffix": message["suffix"] as? String ?? ""]
        case "selection_cleared": model.selection = nil; highlightStatus = nil
        case "geometry":
            fingerprint = message["fingerprint"] as? String ?? ""; geometry = message
            if var position = message["position"] as? [String: Any] { position["kind"] = "text"; position["edition_id"] = edition.id; position["resource"] = currentResource; recordPosition(position) }
        case "ink_placements":
            for placement in message["placements"] as? [[String: Any]] ?? [] { if let id = placement["id"] as? String { placements[id] = placement["status"] as? String ?? "missing" } }
        case "pdf_selection":
            highlightStatus = nil
            pdfHighlights = message["highlights"] as? [[String: Any]] ?? []
            if let page = pdfHighlights.first?["page"] { model.selection = ["kind": "page", "edition_id": edition.id, "page": page] } else { model.selection = nil }
        case "pdf_position":
            if let page = message["page"] { recordPosition(["kind": "page", "edition_id": edition.id, "page": page]) }
        case "navigate":
            if let resource = message["resource"] as? String, let index = asset?.spine.firstIndex(of: resource) { openChapter(index, fragment: message["fragment"] as? String) }
        case "navigation_status": navigationStatus = message["message"] as? String
        default: break
        }
    }
    func recordPosition(_ location: [String: Any]) {
        guard let edition, let data = try? JSONSerialization.data(withJSONObject: location, options: .sortedKeys), let key = String(data: data, encoding: .utf8), key != lastRecordedPosition else { return }
        lastRecordedPosition = key
        model.recordPosition(location: location, alignment: ["status": "confirmed", "edition_id": edition.id, "evidence": "Position observed in the attached content asset by Book Nook's native reader."])
    }
    var highlightRequests: [ReaderHighlightRequest] {
        guard let edition else { return [] }
        if !pdfHighlights.isEmpty {
            return pdfHighlights.compactMap { item in
                guard let page = item["page"], let geometry = item["geometry"] as? [String: Any] else { return nil }
                return ReaderHighlightRequest(anchor: ["kind": "page", "edition_id": edition.id, "page": page], geometry: geometry, context: item["quote"] as? String ?? "PDF highlight")
            }
        }
        guard let anchor = model.selection, anchor["kind"] as? String == "text" else { return [] }
        return [ReaderHighlightRequest(anchor: anchor, geometry: nil, context: anchor["quote"] as? String ?? "")]
    }
    var savedHighlightKeys: Set<String> {
        Set(annotations.filter { $0.strokes.isEmpty && $0.nativeDrawing == nil }.compactMap { annotation in
            annotation.anchor.object.map { ReaderHighlightRequest(anchor: $0, geometry: annotation.inkGeometry?.object, context: annotation.originalContext).key }
        })
    }
    var selectionAlreadyHighlighted: Bool { !highlightRequests.isEmpty && highlightRequests.allSatisfy { savedHighlightKeys.contains($0.key) } }
    func saveHighlight() {
        guard !savingHighlight, let pass = model.notebook?.passId else { return }
        let pending = highlightRequests.filter { !savedHighlightKeys.contains($0.key) }
        guard !pending.isEmpty else { return }
        savingHighlight = true; highlightStatus = "Saving highlight…"
        Task {
            var saved = 0
            defer { savingHighlight = false }
            do {
                for request in pending {
                    let annotation: [String: Any] = ["id": UUID().uuidString, "pass_id": pass, "anchor": request.anchor, "original_context": request.context, "layout_fingerprint": NSNull(), "strokes": [], "native_drawing": NSNull(), "ink_geometry": request.geometry as Any? ?? NSNull()]
                    try await model.perform(["op": "save_annotation", "annotation": annotation]); saved += 1
                }
                highlightStatus = saved == 1 ? "Highlight saved" : "\(saved) highlights saved"
            } catch {
                highlightStatus = "Could not save all highlights (\(saved) of \(pending.count) saved). \(bookNookErrorMessage(error)) Select the passage and retry."
            }
        }
    }
    var inkReceiver: AnnotationSave {
        let editionId = edition?.id, passId = model.notebook?.passId, layout = fingerprint, desk = model
        return { ink, acknowledged in
            guard let editionId, let passId, let id = ink["id"] as? String, var anchor = ink["anchor"] as? [String: Any] else { acknowledged(false); return }
            anchor["edition_id"] = editionId
            var annotation = ink; annotation["pass_id"] = passId; annotation["anchor"] = anchor
            annotation["original_context"] = anchor["quote"] ?? ink["original_context"] ?? "PDF page markup"
            annotation["layout_fingerprint"] = layout.isEmpty ? NSNull() : layout as Any
            Task { await desk.send(["op": "save_annotation", "annotation": annotation]); acknowledged(desk.state?.annotations[id] != nil) }
        }
    }


}

#if os(macOS)
struct EPUBView: NSViewRepresentable {
    let root: URL; let resource: String; let markup: Bool; let annotations: [Annotation]; let geometry: [String: Any]; let fontSize: Int; let restorePosition: [String: Any]?; let spine: [String]; let destination: ReaderDestination?
    let onMessage: ([String: Any]) -> Void; let onInk: AnnotationSave
    var inkSession: ReaderInkSession? = nil
    func makeCoordinator() -> ReaderCoordinator { ReaderCoordinator(onMessage) }
    func makeNSView(context: Context) -> WKWebView { makeWebView(context.coordinator) }
    func updateNSView(_ view: WKWebView, context: Context) { context.coordinator.onMessage = onMessage; updateWeb(view, coordinator: context.coordinator, root: root, resource: resource, annotations: annotations, fontSize: fontSize, restorePosition: restorePosition, spine: spine, destination: destination) }
}
#else
struct EPUBView: UIViewRepresentable {
    let root: URL; let resource: String; let markup: Bool; let annotations: [Annotation]; let geometry: [String: Any]; let fontSize: Int; let restorePosition: [String: Any]?; let spine: [String]; let destination: ReaderDestination?
    let onMessage: ([String: Any]) -> Void; let onInk: AnnotationSave
    var inkSession: ReaderInkSession? = nil
    func makeCoordinator() -> ReaderCoordinator { ReaderCoordinator(onMessage) }
    func makeUIView(context: Context) -> InkReaderHost {
        let host = InkReaderHost(web: makeWebView(context.coordinator))
        context.coordinator.onMessage = { [weak host] message in host?.receive(message); onMessage(message) }
        return host
    }
    func updateUIView(_ view: InkReaderHost, context: Context) {
        context.coordinator.onMessage = { [weak view] message in view?.receive(message); onMessage(message) }
        view.configure(session: inkSession, resource: resource)
        view.onInk = onInk; view.onError = { onMessage(["type": "error", "message": $0]) }; view.annotations = annotations
        updateWeb(view.web, coordinator: context.coordinator, root: root, resource: resource, annotations: annotations, fontSize: fontSize, restorePosition: restorePosition, spine: spine, destination: destination)
        view.setMarkup(markup)
    }
}
#endif
