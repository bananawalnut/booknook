// SPDX-License-Identifier: MPL-2.0
import SwiftUI
#if os(iOS)
import PencilKit
import UIKit
#endif

struct NotebookInkBlockView<Actions: View>: View {
    @ObservedObject var session: NotebookInkSession
    @Binding var text: String
    @Binding var styles: [InlineStyle]
    let formatRequest: FormatRequest?
    let whiteboard: Bool
    let noteLabel: String
    let contextLabel: String?
    let isFlipping: Bool
    let flip: () -> Void
    let addCharacter: () -> Void
    let changed: (String, [InlineStyle]) -> Void
    @ViewBuilder var actions: () -> Actions
    @State private var history = false
    @State private var transcription: NotebookInkVersion?
    @StateObject private var tools = NotebookWhiteboardTools()
    private var current: NotebookInkVersion? { session.currentVersion }
    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            VStack(spacing: 4) {
                Button(action: flip) {
                    Image(systemName: whiteboard ? "text.alignleft" : "rectangle.portrait.on.rectangle.portrait.angled")
                        .frame(width: 44, height: 44).contentShape(Rectangle())
                }.disabled(isFlipping)
                    .accessibilityLabel(whiteboard ? "Show note text" : "Flip note to whiteboard")
                    .accessibilityIdentifier("notebook-face-\(session.blockID)")
                    .help(whiteboard ? "Show note text" : "Flip this note to its whiteboard")
                Button(action: addCharacter) {
                    Image(systemName: "person.crop.rectangle.badge.plus").frame(width: 44, height: 44).contentShape(Rectangle())
                }.accessibilityLabel("Add character card to this note")
                    .accessibilityIdentifier("notebook-character-\(session.blockID)")
                    .help("Add a character card")
                Menu {
                    actions()
                    if whiteboard {
                        Divider()
                        #if os(iOS)
                        Picker("Pencil tool", selection: $tools.tool) {
                            Label("Pen", systemImage: "pencil.tip").tag("pen")
                            Label("Eraser", systemImage: "eraser").tag("eraser")
                        }
                        Button("Undo ink") { tools.undo?() }.disabled(!tools.canUndo)
                        Button("Redo ink") { tools.redo?() }.disabled(!tools.canRedo)
                        #endif
                        Button("Zoom in") { tools.zoomIn?() }
                        Button("Zoom out") { tools.zoomOut?() }
                        Button("Return to origin") { tools.home?() }
                    }
                    Divider()
                    Button("Ink history (\(session.versions.count))") { history = true }.disabled(session.versions.isEmpty)
                    Button("Transcribe handwriting…") { transcription = current }.disabled(current == nil || session.isSaving)
                } label: { Image(systemName: "ellipsis").frame(width: 44, height: 44).contentShape(Rectangle()) }
                    .accessibilityLabel("Note options")
                    .accessibilityIdentifier("notebook-section-options-\(session.blockID)")
            }.buttonStyle(.borderless).foregroundStyle(nookInk.opacity(0.55))
                .padding(.vertical, 4).frame(width: 44)
                .background(nookInk.opacity(0.035), in: RoundedRectangle(cornerRadius: 3))
            VStack(alignment: .leading, spacing: 10) {
                HStack(alignment: .firstTextBaseline) {
                    Text(noteLabel).font(.caption.weight(.medium))
                    if let contextLabel { Text(contextLabel).font(.caption2).lineLimit(2) }
                    Spacer(minLength: 0)
                    if whiteboard { Text("Whiteboard").font(.caption2) }
                }.foregroundStyle(nookInk.opacity(0.55))
                #if os(iOS)
                NotebookNoteFaces(text: $text, styles: $styles, formatRequest: formatRequest,
                                  session: session, tools: tools, whiteboard: whiteboard, changed: changed)
                    .frame(maxWidth: .infinity)
                #else
                ZStack(alignment: .topLeading) {
                    NativeNotebookEditor(text: $text, styles: $styles, formatRequest: formatRequest, changed: changed)
                        .environment(\.notebookPaneActive, !whiteboard)
                        .opacity(whiteboard ? 0 : 1).allowsHitTesting(!whiteboard).accessibilityHidden(whiteboard)
                    NotebookMacWhiteboard(session: session, tools: tools)
                        .opacity(whiteboard ? 1 : 0).allowsHitTesting(whiteboard).accessibilityHidden(!whiteboard)
                }.frame(height: whiteboard ? 480 : 210)
                #endif
                if whiteboard && tools.atLimit {
                    Text("You have reached this board’s coordinate limit. Add another note to keep drawing.")
                        .font(.caption).foregroundStyle(.secondary)
                }
                if session.needsVersionChoice {
                    Label("Choose an ink version to continue. All originals are retained.", systemImage: "arrow.triangle.branch")
                        .font(.callout).foregroundStyle(.orange)
                    Button("Review ink versions") { history = true }
                }
                if let error = session.saveError {
                    VStack(alignment: .leading, spacing: 8) {
                        Text(error).font(.callout).foregroundStyle(.red).textSelection(.enabled)
                        Text(session.allPendingJournaled ? "The original is retained in local recovery on this device. Retry to include it in portable backups." : "Keep Book Nook open until the original reaches local recovery.").font(.caption).foregroundStyle(.secondary)
                        Button("Retry ink save") { session.retry() }
                    }.accessibilityElement(children: .contain)
                }
                if !whiteboard {
                    ForEach(session.ink?.transcriptions ?? []) { transcript in
                        VStack(alignment: .leading, spacing: 5) {
                            Text(transcript.text).font(.body).textSelection(.enabled)
                            Text("Reviewed transcript · ink version \(inkVersionNumber(transcript.inkVersionId, versions: session.versions))")
                                .font(.caption2).foregroundStyle(.secondary)
                        }.padding(.vertical, 8)
                    }
                }
            }
        }.foregroundStyle(nookInk).background(readingPaper)
            .sheet(isPresented: $history) { NotebookInkHistory(session: session, allowWriting: true) }
            .sheet(item: $transcription) { version in NotebookTranscriptionReview(session: session, version: version) }
    }
}

private func inkDate(_ at: UInt64) -> String { Date(timeIntervalSince1970: Double(at)).formatted(date: .abbreviated, time: .shortened) }
private func inkVersionNumber(_ id: String, versions: [NotebookInkVersion]) -> String { versions.firstIndex { $0.id == id }.map { String($0 + 1) } ?? "retained" }

struct NotebookPortableInk: View {
    let version: NotebookInkVersion
    var paper = true
    var body: some View {
        Canvas { context, size in
            let scale = NotebookCanvasGeometry(logicalSize: CGSize(width: version.canvasWidth, height: version.canvasHeight)).scale(in: size)
            guard scale > 0 else { return }
            context.scaleBy(x: scale, y: scale)
            context.translateBy(x: -(version.canvasSpace?.originX ?? 0), y: -(version.canvasSpace?.originY ?? 0))
            for stroke in version.strokes {
                guard let first = stroke.points.first else { continue }
                let color = notebookInkColor(stroke.color)
                if stroke.points.count == 1 {
                    context.fill(Path(ellipseIn: CGRect(x: first.x - stroke.width / 2, y: first.y - stroke.width / 2, width: stroke.width, height: stroke.width)), with: .color(color))
                } else {
                    var path = Path(); path.move(to: CGPoint(x: first.x, y: first.y))
                    for point in stroke.points.dropFirst() { path.addLine(to: CGPoint(x: point.x, y: point.y)) }
                    context.stroke(path, with: .color(color), style: StrokeStyle(lineWidth: stroke.width, lineCap: .round, lineJoin: .round))
                }
            }
        }.background(paper ? readingPaper : Color.clear).clipped()
            .accessibilityLabel("Original handwriting, \(version.strokes.count) strokes, saved \(inkDate(version.at))")
    }
}

func notebookInkColor(_ hex: String) -> Color {
    guard let number = UInt32(hex.dropFirst(), radix: 16) else { return .primary }
    return Color(red: Double((number >> 16) & 255) / 255, green: Double((number >> 8) & 255) / 255, blue: Double(number & 255) / 255)
}

private struct NotebookInkHistory: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject var session: NotebookInkSession
    let allowWriting: Bool
    @State private var error: String?
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack { Text("Original ink history").font(.title2); Spacer(); Button("Done") { dismiss() } }
            Text("Every saved stroke and erasure remains recoverable. Continuing an earlier version creates a separate branch on the next edit.").font(.callout).foregroundStyle(.secondary)
            if let error { Text(error).foregroundStyle(.red) }
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 24) {
                    ForEach(Array(session.versions.enumerated()).reversed(), id: \.element.id) { index, version in
                        VStack(alignment: .leading, spacing: 8) {
                            Text("Version \(index + 1) · \(inkDate(version.at))").font(.headline)
                            Text("\(version.strokes.count) strokes · Saved by \(version.reader)").font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                            if session.heads.contains(where: { $0.id == version.id }) { Text("Current branch").font(.caption).foregroundStyle(nookAccent) }
                            NotebookPortableInk(version: version).aspectRatio(version.canvasWidth / version.canvasHeight, contentMode: .fit).frame(maxHeight: 350)
                            if allowWriting {
                                Button("Continue from this version") {
                                    if session.chooseVersion(id: version.id) { dismiss() }
                                    else { error = "Finish saving the current ink before choosing a version." }
                                }.disabled(session.isSaving)
                            }
                            Divider()
                        }
                    }
                }
            }
        }.padding(24).background(readingPaper).foregroundStyle(nookInk).frame(minWidth: 360, idealWidth: 560, minHeight: 400)
    }
}

private struct NotebookTranscriptionReview: View {
    @EnvironmentObject private var model: DeskModel
    @Environment(\.dismiss) private var dismiss
    @ObservedObject var session: NotebookInkSession
    let version: NotebookInkVersion
    @StateObject private var draft = NotebookTranscriptionDraft()
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Review a transcript").font(.title2)
            Text("Ink version \(inkVersionNumber(version.id, versions: session.versions)) · \(inkDate(version.at)). Text is saved alongside this original; neither handwriting nor typed prose is replaced.").font(.callout).foregroundStyle(.secondary)
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    NotebookPortableInk(version: version).aspectRatio(version.canvasWidth / version.canvasHeight, contentMode: .fit).frame(maxHeight: 280)
                    if draft.isRecognizing {
                        HStack { ProgressView(); Text("Recognizing on this device…"); Button("Stop recognition") { draft.cancelRecognition() } }
                    } else {
                        Button("Recognize on this device") { draft.recognize(version) }.disabled(draft.isSaving || !draft.text.isEmpty)
                    }
                    Text("Review recognition carefully, or enter a transcript yourself. Nothing is sent to a service.").font(.caption).foregroundStyle(.secondary)
                    if !draft.text.isEmpty { Text("Clear the draft first if you want to recognize it again.").font(.caption).foregroundStyle(.secondary) }
                    TextEditor(text: $draft.text).scrollContentBackground(.hidden).background(readingPaper).frame(minHeight: 180).accessibilityLabel("Reviewed handwriting transcript").disabled(draft.isRecognizing || draft.isSaving)
                    if let error = draft.error { Text(error).foregroundStyle(.red).textSelection(.enabled) }
                }
            }
            HStack {
                Button("Cancel") { draft.cancelRecognition(); dismiss() }.disabled(draft.isSaving)
                Spacer()
                Button("Save transcript") {
                    Task {
                        if await draft.save({ text, method in
                            try await model.saveInkTranscription(notebook: session.notebookID, block: session.blockID, version: version.id, text: text, method: method)
                        }) { dismiss() }
                    }
                }.disabled(draft.isRecognizing || draft.isSaving || draft.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }.padding(24).background(readingPaper).foregroundStyle(nookInk).frame(minWidth: 360, idealWidth: 560, minHeight: 500)
            .interactiveDismissDisabled(draft.isSaving).onDisappear { draft.cancelRecognition() }
    }
}

#if os(iOS)
@MainActor final class NotebookInkTools: ObservableObject {
    @Published var tool = "pen"
    @Published var canUndo = false
    @Published var canRedo = false
    var undo: (() -> Void)?
    var redo: (() -> Void)?
}

struct NotebookPencilCanvas: UIViewRepresentable {
    @Environment(\.notebookPaneActive) private var paneActive
    @ObservedObject var session: NotebookInkSession
    @ObservedObject var tools: NotebookInkTools
    let editable: Bool
    func makeUIView(context: Context) -> NotebookInkHost { NotebookInkHost(session: session, tools: tools) }
    func updateUIView(_ view: NotebookInkHost, context: Context) { view.update(editable: editable, active: paneActive) }
}

final class NotebookInkHost: UIView, PKCanvasViewDelegate {
    let canvas = NotebookPencilInputCanvas()
    let session: NotebookInkSession
    let tools: NotebookInkTools
    private var logicalSize = NotebookCanvasGeometry.defaultSize
    private var appliedID: String?
    private var lastCaptured: Data?
    private var applying = false
    private var drawing = false
    private var strokeParents: [String]?
    private var captureScheduled = false
    private var appliedTool = ""
    private var frozenScrollViews: [(UIScrollView, Bool)] = []
    init(session: NotebookInkSession, tools: NotebookInkTools) {
        self.session = session; self.tools = tools; super.init(frame: .zero)
        addSubview(canvas); clipsToBounds = true
        canvas.backgroundColor = .clear; canvas.isOpaque = false; canvas.isScrollEnabled = false
        canvas.drawingPolicy = .pencilOnly; canvas.delegate = self
        canvas.accessibilityLabel = "Handwriting page. Write with Pencil; scroll with a finger."
        canvas.accessibilityIdentifier = "notebook-ink-\(session.blockID)"
        session.capturePendingStroke = { [weak self] in self?.capture() }
        tools.undo = { [weak self] in self?.undo() }; tools.redo = { [weak self] in self?.redo() }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) unavailable") }
    /// The host itself must not swallow a finger that the Pencil canvas declines:
    /// a transparent ink layer sits above TextKit in the same native page.
    override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? {
        guard isUserInteractionEnabled, !isHidden, alpha > 0, bounds.contains(point) else { return nil }
        return canvas.hitTest(convert(point, to: canvas), with: event)
    }
    var logicalCanvasSize: CGSize { logicalSize }
    static let maximumCanvasHeight: CGFloat = 16384
    func extendPaper(to logicalHeight: CGFloat) {
        guard !drawing, logicalHeight.isFinite else { return }
        let saved = session.currentVersion.map { CGFloat($0.canvasHeight) } ?? NotebookCanvasGeometry.defaultSize.height
        let height = max(saved, min(Self.maximumCanvasHeight, logicalHeight))
        if logicalSize.height != height { logicalSize.height = height; setNeedsLayout() }
    }
    override func layoutSubviews() {
        super.layoutSubviews()
        let scale = NotebookCanvasGeometry(logicalSize: logicalSize).scale(in: bounds.size)
        canvas.bounds = CGRect(origin: .zero, size: logicalSize)
        canvas.transform = CGAffineTransform(scaleX: scale, y: scale)
        canvas.center = CGPoint(x: logicalSize.width * scale / 2, y: logicalSize.height * scale / 2)
    }
    func update(editable: Bool, active: Bool = true) {
        let enabled = editable && active && !session.needsVersionChoice
        if !enabled && canvas.isUserInteractionEnabled {
            // A compact resize can hide this retained canvas mid-stroke. Keep
            // its original drawing, release only its responder, and let finger
            // scrolling resume without disturbing an unrelated sheet editor.
            drawing = false; capture(); restoreScrolling()
            if canvas.isFirstResponder { canvas.resignFirstResponder() }
        }
        canvas.isUserInteractionEnabled = enabled
        canvas.accessibilityLabel = editable ? "Handwriting page. Write with Pencil; scroll with a finger." : "Original handwriting page."
        if appliedTool != tools.tool {
            canvas.tool = tools.tool == "eraser" ? PKEraserTool(.vector) : PKInkingTool(.pen, color: UIColor(red: 0.20, green: 0.28, blue: 0.23, alpha: 1), width: 3)
            appliedTool = tools.tool
        }
        guard !drawing else { return }
        let version = session.currentVersion
        guard version?.id != appliedID || lastCaptured == nil else { return }
        let next = notebookDrawing(version)
        let data = next.dataRepresentation()
        if data != canvas.drawing.dataRepresentation() {
            applying = true; canvas.drawing = next; canvas.undoManager?.removeAllActions(); applying = false
        }
        logicalSize = version.map { CGSize(width: $0.canvasWidth, height: $0.canvasHeight) } ?? NotebookCanvasGeometry.defaultSize
        appliedID = version?.id; lastCaptured = data
        setNeedsLayout(); scheduleUndoState()
    }
    func canvasViewDidBeginUsingTool(_ canvasView: PKCanvasView) {
        // A published remote head may still be waiting for updateUIView. Parent
        // the canvas the reader actually saw, never an unapplied model snapshot.
        drawing = true; strokeParents = appliedID.map { [$0] } ?? []
        restoreScrolling()
        var ancestor = superview
        while let view = ancestor {
            if let scroll = view as? UIScrollView { frozenScrollViews.append((scroll, scroll.isScrollEnabled)); scroll.isScrollEnabled = false }
            ancestor = view.superview
        }
        _ = canvas.becomeFirstResponder()
        scheduleUndoState()
    }
    func canvasViewDidEndUsingTool(_ canvasView: PKCanvasView) { drawing = false; restoreScrolling(); capture() }
    func canvasViewDrawingDidChange(_ canvasView: PKCanvasView) {
        // Final estimated Pencil samples can arrive after endUsingTool. Capture
        // those as another immutable refinement, without saving every sample.
        if !applying && !drawing { scheduleCapture() }
    }
    override func willMove(toWindow newWindow: UIWindow?) { if newWindow == nil { capture(); restoreScrolling() }; super.willMove(toWindow: newWindow) }
    private func restoreScrolling() {
        for (view, enabled) in frozenScrollViews { view.isScrollEnabled = enabled }
        frozenScrollViews.removeAll()
    }
    private func scheduleCapture() {
        guard !captureScheduled else { return }; captureScheduled = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }; self.captureScheduled = false
            if !self.drawing { self.capture() }
        }
    }
    private func capture() {
        // Before the first update the native canvas is empty even when its
        // session has saved ink. A visibility/teardown callback must not turn
        // that uninitialized canvas into a new erasure version.
        guard !applying, lastCaptured != nil else { return }
        let value = canvas.drawing, data = value.dataRepresentation()
        guard data != lastCaptured else { scheduleUndoState(); return }
        if session.save(canvasWidth: logicalSize.width, canvasHeight: logicalSize.height, strokes: portableInk(value), nativeDrawing: data.base64EncodedString(), parents: strokeParents) {
            lastCaptured = data; appliedID = session.currentVersion?.id
            strokeParents = session.currentVersion.map { [$0.id] } ?? []
        }
        scheduleUndoState()
    }
    private func undo() { guard !drawing else { return }; strokeParents = appliedID.map { [$0] } ?? []; canvas.undoManager?.undo(); capture() }
    private func redo() { guard !drawing else { return }; strokeParents = appliedID.map { [$0] } ?? []; canvas.undoManager?.redo(); capture() }
    private func scheduleUndoState() {
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.tools.canUndo = !self.drawing && (self.canvas.undoManager?.canUndo ?? false)
            self.tools.canRedo = !self.drawing && (self.canvas.undoManager?.canRedo ?? false)
        }
    }
}

/// A canvas owns its undo stack, so selecting an ink-history branch cannot
/// clear a text editor's undo actions higher in the responder chain.
final class NotebookPencilInputCanvas: PKCanvasView {
    private let inkUndoManager = UndoManager()
    override var undoManager: UndoManager? { inkUndoManager }
    override var canBecomeFirstResponder: Bool { isUserInteractionEnabled && super.canBecomeFirstResponder }
    override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? {
        if let touches = event?.allTouches, !touches.isEmpty, touches.allSatisfy({ $0.type != .pencil }) { return nil }
        return super.hitTest(point, with: event)
    }
}

func notebookDrawing(_ version: NotebookInkVersion?) -> PKDrawing {
    guard let version else { return PKDrawing() }
    if let native = version.nativeDrawing, let data = Data(base64Encoded: native), let drawing = try? PKDrawing(data: data) { return drawing }
    return PKDrawing(strokes: version.strokes.map { stroke in
        let color = UIColor(notebookInkColor(stroke.color))
        let points = stroke.points.map { point in PKStrokePoint(location: CGPoint(x: point.x, y: point.y), timeOffset: point.time, size: CGSize(width: stroke.width, height: stroke.width), opacity: 1, force: point.pressure, azimuth: 0, altitude: .pi / 2) }
        return PKStroke(ink: PKInk(.pen, color: color), path: PKStrokePath(controlPoints: points, creationDate: Date(timeIntervalSince1970: 0)))
    })
}
#endif
