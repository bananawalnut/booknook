// SPDX-License-Identifier: MPL-2.0
import SwiftUI
#if os(iOS)
import UIKit
import PencilKit
#else
import AppKit
#endif

@MainActor final class NotebookWhiteboardTools: ObservableObject {
    @Published var tool = "pen"
    @Published var canUndo = false
    @Published var canRedo = false
    @Published var atLimit = false
    var undo: (() -> Void)?
    var redo: (() -> Void)?
    var zoomIn: (() -> Void)?
    var zoomOut: (() -> Void)?
    var home: (() -> Void)?
}

func notebookWhiteboardExtent(_ version: NotebookInkVersion?) -> CGRect {
    guard let version else { return CGRect(x: 0, y: 0, width: 1024, height: 1024) }
    return CGRect(x: version.canvasSpace?.originX ?? 0, y: version.canvasSpace?.originY ?? 0,
                  width: version.canvasWidth, height: version.canvasHeight)
}

#if os(iOS)
/// PencilKit owns its tiled drawing and scrolling. Only the viewport-sized
/// canvas is a UIView; expanding the document changes its scrollable extent.
final class NotebookWhiteboardHost: UIView, PKCanvasViewDelegate {
    // PencilKit's tile renderer does not display ink at negative content
    // coordinates. This fixed bridge keeps every legal world point positive in
    // the renderer. It never changes while scrolling, so native undo remains in
    // one coordinate space. Persisted PKDrawing and portable ink stay in world
    // coordinates and do not contain this presentation-only translation.
    private static let rendererBias = CGPoint(x: NotebookWhiteboardGeometry.limit, y: NotebookWhiteboardGeometry.limit)
    let canvas = NotebookBoardCanvas()
    let session: NotebookInkSession
    let tools: NotebookWhiteboardTools
    private(set) var geometry = NotebookWhiteboardGeometry()
    private var appliedID: String?
    private var lastCaptured: Data?
    private var applying = false
    private var drawing = false
    private var strokeParents: [String]?
    private var capturedContext: NotebookInkCapture?
    private var captureScheduled = false
    private var applyingExtent = false
    private var initializedViewport = false
    private var frozenScrollViews: [(UIScrollView, Bool)] = []
    private var enabled = false
    init(session: NotebookInkSession, tools: NotebookWhiteboardTools) {
        self.session = session; self.tools = tools; super.init(frame: .zero)
        addSubview(canvas); clipsToBounds = true
        canvas.backgroundColor = UIColor(readingPaper); canvas.isOpaque = true
        canvas.drawingPolicy = .anyInput; canvas.delegate = self
        canvas.minimumZoomScale = 0.25; canvas.maximumZoomScale = 4
        canvas.contentInsetAdjustmentBehavior = .never
        canvas.bounces = false; canvas.bouncesZoom = false
        canvas.alwaysBounceHorizontal = true; canvas.alwaysBounceVertical = true
        // One finger and Pencil share PencilKit's drawing path. Reserve a
        // two-finger gesture for navigation so a finger stroke cannot pan.
        canvas.panGestureRecognizer.minimumNumberOfTouches = 2
        canvas.panGestureRecognizer.maximumNumberOfTouches = 2
        canvas.panGestureRecognizer.allowedTouchTypes = [NSNumber(value: UITouch.TouchType.direct.rawValue)]
        canvas.accessibilityLabel = "Note whiteboard"
        canvas.accessibilityHint = "Draw with a finger or Pencil. Pan with two fingers and pinch to zoom."
        canvas.accessibilityIdentifier = "notebook-whiteboard-\(session.blockID)"
        session.capturePendingStroke = { [weak self] in self?.capture() }
        tools.undo = { [weak self] in self?.undo() }; tools.redo = { [weak self] in self?.redo() }
        tools.zoomIn = { [weak self] in self?.changeZoom(by: 1.25) }
        tools.zoomOut = { [weak self] in self?.changeZoom(by: 0.8) }
        tools.home = { [weak self] in self?.showOriginal() }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) unavailable") }
    override func layoutSubviews() {
        super.layoutSubviews(); canvas.frame = bounds
        applyExtent(); expandVisibleExtent()
    }
    func update(active: Bool) {
        let nextEnabled = active && !session.needsVersionChoice
        if enabled && !nextEnabled {
            drawing = false; capture(); restoreScrolling()
            if canvas.isFirstResponder { canvas.resignFirstResponder() }
        }
        enabled = nextEnabled; canvas.isUserInteractionEnabled = nextEnabled
        canvas.tool = tools.tool == "eraser" ? PKEraserTool(.vector) : PKInkingTool(.pen, color: UIColor(nookInk), width: 3)
        guard !drawing else { return }
        let version = session.currentVersion
        guard version?.id != appliedID || lastCaptured == nil else { return }
        let next = rendererDrawing(notebookDrawing(version)), bytes = next.dataRepresentation()
        if bytes != canvas.drawing.dataRepresentation() {
            applying = true; canvas.drawing = next; canvas.undoManager?.removeAllActions(); applying = false
        }
        geometry = NotebookWhiteboardGeometry(extent: notebookWhiteboardExtent(version))
        // PencilKit may normalize serialization on assignment. Compare future
        // delegate callbacks with what the canvas actually accepted, rather
        // than treating normalization as a user edit.
        appliedID = version?.id; lastCaptured = canvas.drawing.dataRepresentation()
        applyExtent(); expandVisibleExtent(); refreshUndo()
    }
    func rendererPoint(forWorld point: CGPoint) -> CGPoint {
        CGPoint(x: point.x + Self.rendererBias.x, y: point.y + Self.rendererBias.y)
    }
    func rendererDrawing(_ drawing: PKDrawing) -> PKDrawing {
        drawing.transformed(using: CGAffineTransform(translationX: Self.rendererBias.x, y: Self.rendererBias.y))
    }
    var worldDrawing: PKDrawing {
        canvas.drawing.transformed(using: CGAffineTransform(translationX: -Self.rendererBias.x, y: -Self.rendererBias.y))
    }
    func viewportRect(forWorld rect: CGRect) -> CGRect {
        let point = rendererPoint(forWorld: rect.origin), zoom = canvas.zoomScale
        return CGRect(x: point.x * zoom - canvas.contentOffset.x, y: point.y * zoom - canvas.contentOffset.y,
                      width: rect.width * zoom, height: rect.height * zoom)
    }
    func setWorldViewportOrigin(_ origin: CGPoint) {
        let zoom = canvas.zoomScale
        let visible = CGRect(origin: origin, size: CGSize(width: canvas.bounds.width / zoom, height: canvas.bounds.height / zoom))
        if geometry.expand(around: visible) { applyExtent() }
        let point = rendererPoint(forWorld: origin)
        canvas.setContentOffset(CGPoint(x: point.x * zoom, y: point.y * zoom), animated: false)
        expandVisibleExtent()
    }
    private func applyExtent() {
        guard !applyingExtent else { return }; applyingExtent = true
        let zoom = canvas.zoomScale
        let offset = initializedViewport ? canvas.contentOffset : CGPoint(x: Self.rendererBias.x * zoom, y: Self.rendererBias.y * zoom)
        let rect = geometry.extent
        // The content size describes a tiled document, never a giant UIView.
        // World expansion only extends scroll bounds; drawing coordinates and
        // the fixed renderer bias are unaffected.
        canvas.contentSize = CGSize(width: max(1, rect.maxX + Self.rendererBias.x) * zoom,
                                    height: max(1, rect.maxY + Self.rendererBias.y) * zoom)
        canvas.contentInset = .zero
        canvas.setContentOffset(offset, animated: false)
        initializedViewport = true
        applyingExtent = false
    }
    func expandVisibleExtent() {
        guard !drawing, !applyingExtent,
              let rendererVisible = NotebookWhiteboardGeometry.visibleWorld(offset: canvas.contentOffset, zoom: canvas.zoomScale, viewport: canvas.bounds.size) else { return }
        let visible = rendererVisible.offsetBy(dx: -Self.rendererBias.x, dy: -Self.rendererBias.y)
        if geometry.expand(around: visible) { applyExtent() }
        let atLimit = visible.minX <= -NotebookWhiteboardGeometry.limit || visible.minY <= -NotebookWhiteboardGeometry.limit
            || visible.maxX >= NotebookWhiteboardGeometry.limit || visible.maxY >= NotebookWhiteboardGeometry.limit
        if tools.atLimit != atLimit { DispatchQueue.main.async { [weak self] in self?.tools.atLimit = atLimit } }
    }
    func scrollViewDidScroll(_ scrollView: UIScrollView) { expandVisibleExtent() }
    func scrollViewDidZoom(_ scrollView: UIScrollView) { applyExtent(); expandVisibleExtent() }
    private func changeZoom(by factor: CGFloat) {
        guard !drawing else { return }
        let center = CGPoint(x: (canvas.contentOffset.x + canvas.bounds.width / 2) / canvas.zoomScale,
                             y: (canvas.contentOffset.y + canvas.bounds.height / 2) / canvas.zoomScale)
        let zoom = min(canvas.maximumZoomScale, max(canvas.minimumZoomScale, canvas.zoomScale * factor))
        canvas.setZoomScale(zoom, animated: false)
        canvas.contentOffset = CGPoint(x: center.x * zoom - canvas.bounds.width / 2, y: center.y * zoom - canvas.bounds.height / 2)
        expandVisibleExtent()
    }
    private func showOriginal() {
        guard !drawing else { return }
        canvas.setZoomScale(1, animated: false); setWorldViewportOrigin(.zero)
    }
    func canvasViewDidBeginUsingTool(_ canvasView: PKCanvasView) {
        drawing = true; strokeParents = appliedID.map { [$0] } ?? []; capturedContext = session.beginCapture()
        restoreScrolling(); var ancestor = superview
        while let view = ancestor {
            if let scroll = view as? UIScrollView { frozenScrollViews.append((scroll, scroll.isScrollEnabled)); scroll.isScrollEnabled = false }
            ancestor = view.superview
        }
        canvas.panGestureRecognizer.isEnabled = false
        canvas.pinchGestureRecognizer?.isEnabled = false
        _ = canvas.becomeFirstResponder(); refreshUndo()
    }
    func canvasViewDidEndUsingTool(_ canvasView: PKCanvasView) {
        drawing = false; restoreScrolling(); capture(); expandVisibleExtent()
    }
    func canvasViewDrawingDidChange(_ canvasView: PKCanvasView) {
        guard !applying, !drawing, !captureScheduled else { return }; captureScheduled = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }; self.captureScheduled = false
            if !self.drawing { self.capture() }
        }
    }
    override func willMove(toWindow newWindow: UIWindow?) {
        if newWindow == nil { drawing = false; capture(); restoreScrolling() }
        super.willMove(toWindow: newWindow)
    }
    private func restoreScrolling() {
        for (view, wasEnabled) in frozenScrollViews { view.isScrollEnabled = wasEnabled }; frozenScrollViews.removeAll()
        canvas.panGestureRecognizer.isEnabled = true; canvas.pinchGestureRecognizer?.isEnabled = true
    }
    func capture() {
        guard !applying, lastCaptured != nil else { return }
        let displayedBytes = canvas.drawing.dataRepresentation()
        guard displayedBytes != lastCaptured else { refreshUndo(); return }
        let value = worldDrawing, bytes = value.dataRepresentation()
        let points = portableInk(value)
        // The provider contract bounds absolute coordinates. Never drop an
        // out-of-range original: retain the pending drawing and surface failure.
        let rect = geometry.extent
        if session.save(canvasWidth: rect.width, canvasHeight: rect.height, strokes: points,
                        nativeDrawing: bytes.base64EncodedString(), parents: strokeParents,
                        canvasSpace: NotebookCanvasSpace(kind: "whiteboard", originX: rect.minX, originY: rect.minY), capturedContext: capturedContext) {
            lastCaptured = displayedBytes; appliedID = session.currentVersion?.id
            strokeParents = appliedID.map { [$0] } ?? []
        }
        refreshUndo()
    }
    private func undo() { guard !drawing else { return }; capturedContext = session.beginCapture(); strokeParents = appliedID.map { [$0] } ?? []; canvas.undoManager?.undo(); capture() }
    private func redo() { guard !drawing else { return }; capturedContext = session.beginCapture(); strokeParents = appliedID.map { [$0] } ?? []; canvas.undoManager?.redo(); capture() }
    private func refreshUndo() {
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.tools.canUndo = !self.drawing && (self.canvas.undoManager?.canUndo ?? false)
            self.tools.canRedo = !self.drawing && (self.canvas.undoManager?.canRedo ?? false)
        }
    }
}

final class NotebookBoardCanvas: PKCanvasView {
    private let boardUndo = UndoManager()
    override var undoManager: UndoManager? { boardUndo }
}

/// One native owner retains both faces and their independent undo managers.
/// Hidden prose cannot receive input; flipping never recreates its editor.
struct NotebookNoteFaces: UIViewRepresentable {
    @Environment(\.notebookPaneActive) private var active
    @Binding var text: String
    @Binding var styles: [InlineStyle]
    let formatRequest: FormatRequest?
    @ObservedObject var session: NotebookInkSession
    @ObservedObject var tools: NotebookWhiteboardTools
    let whiteboard: Bool
    let changed: (String, [InlineStyle]) -> Void
    private var editor: NativeNotebookEditor { NativeNotebookEditor(text: $text, styles: $styles, formatRequest: formatRequest, changed: changed) }
    func makeCoordinator() -> NativeNotebookEditor.Coordinator { NativeNotebookEditor.Coordinator(editor) }
    func makeUIView(context: Context) -> NotebookNoteFacesHost {
        let textView = editor.makeEditor(coordinator: context.coordinator, active: active && !whiteboard)
        textView.isScrollEnabled = false; textView.accessibilityIdentifier = "notebook-text-\(session.blockID)"
        let host = NotebookNoteFacesHost(editor: textView, board: NotebookWhiteboardHost(session: session, tools: tools))
        host.showBoard(whiteboard, active: active); return host
    }
    func updateUIView(_ view: NotebookNoteFacesHost, context: Context) {
        editor.updateEditor(view.editor, coordinator: context.coordinator, active: active && !whiteboard)
        view.showBoard(whiteboard, active: active)
    }
    func sizeThatFits(_ proposal: ProposedViewSize, uiView: NotebookNoteFacesHost, context: Context) -> CGSize? {
        guard let width = proposal.width, width.isFinite, width > 0 else { return nil }
        return CGSize(width: width, height: whiteboard ? 480 : max(110, uiView.editor.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude)).height))
    }
    static func dismantleUIView(_ view: NotebookNoteFacesHost, coordinator: NativeNotebookEditor.Coordinator) {
        coordinator.textViewDidChange(view.editor); view.board.capture()
    }
}
final class NotebookNoteFacesHost: UIView {
    let editor: UITextView
    let board: NotebookWhiteboardHost
    private let placeholder = UILabel()
    init(editor: UITextView, board: NotebookWhiteboardHost) {
        self.editor = editor; self.board = board; super.init(frame: .zero)
        addSubview(editor); addSubview(placeholder); addSubview(board); clipsToBounds = true
        placeholder.text = "Write a note…"; placeholder.font = notebookBodyFont()
        placeholder.textColor = UIColor(nookInk).withAlphaComponent(0.4); placeholder.isUserInteractionEnabled = false
        placeholder.isAccessibilityElement = false
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) unavailable") }
    func showBoard(_ visible: Bool, active: Bool) {
        editor.isHidden = visible; editor.accessibilityElementsHidden = visible
        board.isHidden = !visible; board.accessibilityElementsHidden = !visible; board.update(active: active && visible)
        placeholder.isHidden = visible || !editor.text.isEmpty
        invalidateIntrinsicContentSize(); setNeedsLayout()
    }
    override func layoutSubviews() {
        super.layoutSubviews(); editor.frame = bounds; board.frame = bounds
        let inset = editor.textContainerInset
        placeholder.frame = CGRect(x: inset.left + editor.textContainer.lineFragmentPadding, y: inset.top,
                                   width: max(0, bounds.width - inset.left - inset.right - 10), height: notebookBodyFont().lineHeight)
    }
}
#else
struct NotebookMacWhiteboard: NSViewRepresentable {
    @ObservedObject var session: NotebookInkSession
    @ObservedObject var tools: NotebookWhiteboardTools
    func makeNSView(context: Context) -> NotebookMacBoardView { NotebookMacBoardView(tools: tools) }
    func updateNSView(_ view: NotebookMacBoardView, context: Context) { view.version = session.currentVersion; view.needsDisplay = true }
}
final class NotebookMacBoardView: NSView {
    var version: NotebookInkVersion?
    private var origin = CGPoint.zero
    private var zoom: CGFloat = 1
    private var dragPoint: CGPoint?
    private var geometry = NotebookWhiteboardGeometry()
    init(tools: NotebookWhiteboardTools) {
        super.init(frame: .zero)
        setAccessibilityElement(true); setAccessibilityRole(.image)
        setAccessibilityLabel("Note whiteboard. Drag or scroll to pan; pinch or use the menu to zoom. Pencil originals are read-only on Mac.")
        tools.zoomIn = { [weak self] in self?.changeZoom(1.25) }; tools.zoomOut = { [weak self] in self?.changeZoom(0.8) }
        tools.home = { [weak self] in self?.origin = .zero; self?.zoom = 1; self?.needsDisplay = true }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) unavailable") }
    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { true }
    override func draw(_ dirtyRect: NSRect) {
        NSColor(readingPaper).setFill(); bounds.fill()
        guard let context = NSGraphicsContext.current?.cgContext, let version else { return }
        context.saveGState(); defer { context.restoreGState() }
        context.scaleBy(x: zoom, y: zoom); context.translateBy(x: -origin.x, y: -origin.y)
        for stroke in version.strokes {
            guard let first = stroke.points.first else { continue }
            context.setStrokeColor(NSColor(notebookInkColor(stroke.color)).cgColor)
            context.setFillColor(NSColor(notebookInkColor(stroke.color)).cgColor)
            context.setLineWidth(stroke.width); context.setLineCap(.round); context.setLineJoin(.round)
            if stroke.points.count == 1 { context.fillEllipse(in: CGRect(x: first.x - stroke.width / 2, y: first.y - stroke.width / 2, width: stroke.width, height: stroke.width)) }
            else {
                context.beginPath(); context.move(to: CGPoint(x: first.x, y: first.y))
                for point in stroke.points.dropFirst() { context.addLine(to: CGPoint(x: point.x, y: point.y)) }
                context.strokePath()
            }
        }
    }
    override func mouseDown(with event: NSEvent) { dragPoint = convert(event.locationInWindow, from: nil) }
    override func mouseDragged(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        if let old = dragPoint { pan(CGSize(width: old.x - point.x, height: old.y - point.y)) }; dragPoint = point
    }
    override func mouseUp(with event: NSEvent) { dragPoint = nil }
    override func scrollWheel(with event: NSEvent) { pan(CGSize(width: -event.scrollingDeltaX, height: -event.scrollingDeltaY)) }
    override func magnify(with event: NSEvent) { changeZoom(1 + event.magnification) }
    private func pan(_ delta: CGSize) {
        origin.x = min(NotebookWhiteboardGeometry.limit, max(-NotebookWhiteboardGeometry.limit, origin.x + delta.width / zoom))
        origin.y = min(NotebookWhiteboardGeometry.limit, max(-NotebookWhiteboardGeometry.limit, origin.y + delta.height / zoom))
        _ = geometry.expand(around: CGRect(origin: origin, size: CGSize(width: bounds.width / zoom, height: bounds.height / zoom)))
        needsDisplay = true
    }
    private func changeZoom(_ factor: CGFloat) {
        guard factor.isFinite, factor > 0 else { return }
        let center = CGPoint(x: origin.x + bounds.width / (2 * zoom), y: origin.y + bounds.height / (2 * zoom))
        zoom = min(4, max(0.25, zoom * factor))
        origin = CGPoint(x: center.x - bounds.width / (2 * zoom), y: center.y - bounds.height / (2 * zoom)); needsDisplay = true
    }
}
#endif
