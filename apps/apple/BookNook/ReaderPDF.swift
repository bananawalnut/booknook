// SPDX-License-Identifier: MPL-2.0
import SwiftUI
import PDFKit
#if os(iOS)
import PencilKit
#endif

typealias AnnotationSave = ([String: Any], @escaping (Bool) -> Void) -> Void

final class PDFReaderCoordinator: NSObject {
    var onMessage: ([String: Any]) -> Void
    var onInk: AnnotationSave
    var fingerprint = ""
    var initialPage: Int?
    var annotations: [Annotation] = []
    var markup = false
    weak var view: PDFView?
    private var tokens: [NSObjectProtocol] = []
    private var rendered = Set<String>()
    private var renderedMarks: [String: [(PDFPage, PDFAnnotation)]] = [:]
    #if os(iOS)
    var overlays: [Int: PDFInkOverlay] = [:]
    var inkSession: ReaderInkSession?
    #endif
    init(onMessage: @escaping ([String: Any]) -> Void, onInk: @escaping AnnotationSave) { self.onMessage = onMessage; self.onInk = onInk }
    deinit { tokens.forEach(NotificationCenter.default.removeObserver) }
    private func emit(_ message: [String: Any]) { var message = message; message["assetFingerprint"] = fingerprint; let receiver = onMessage; DispatchQueue.main.async { receiver(message) } }
    func connect(_ view: PDFView) {
        self.view = view
        tokens = [NotificationCenter.default.addObserver(forName: .PDFViewSelectionChanged, object: view, queue: .main) { [weak self] _ in self?.selection() }, NotificationCenter.default.addObserver(forName: .PDFViewPageChanged, object: view, queue: .main) { [weak self] _ in self?.position() }]
        #if os(iOS)
        view.pageOverlayViewProvider = self
        #endif
    }
    func load(_ url: URL) {
        guard let view, view.document?.documentURL != url else { return }
        rendered = []; renderedMarks = [:]; view.document = PDFDocument(url: url)
        if view.document == nil { emit(["type": "error", "message": "This PDF could not be opened."]) }
        else if view.document?.isLocked == true { emit(["type": "error", "message": "This PDF is password protected. Attach an accessible copy to read and annotate it."]) }
        if let initialPage, let page = view.document?.page(at: initialPage - 1) { view.go(to: page) }
        position()
    }
    func position() { if let view, let page = view.currentPage, let document = view.document { emit(["type": "pdf_position", "page": document.index(for: page) + 1]) } }
    func geometry(for page: PDFPage, rects: [CGRect], quote: String?) -> [String: Any] {
        let box = page.bounds(for: .cropBox)
        return ["space": "pdf_page", "resource": "book.pdf", "page": (view?.document?.index(for: page) ?? 0) + 1, "content_fingerprint": fingerprint, "width": box.width, "height": box.height, "origin_x": box.minX, "origin_y": box.minY, "text_quote": quote as Any? ?? NSNull(), "text_prefix": NSNull(), "text_suffix": NSNull(), "text_rects": rects.map { ["x": $0.minX, "y": $0.minY, "width": $0.width, "height": $0.height] }]
    }
    func selection() {
        guard let selection = view?.currentSelection, let document = view?.document else { emit(["type": "pdf_selection", "highlights": []]); return }
        let highlights: [[String: Any]] = selection.pages.compactMap { page in
            let lines = selection.selectionsByLine().filter { $0.pages.contains(page) }
            let rects = lines.map { $0.bounds(for: page) }.filter { !$0.isEmpty && !$0.isInfinite && !$0.isNull }
            guard !rects.isEmpty else { return nil }
            let quote = lines.compactMap(\.string).joined(separator: "\n")
            return ["page": document.index(for: page) + 1, "quote": quote, "geometry": geometry(for: page, rects: rects, quote: quote)]
        }
        emit(["type": "pdf_selection", "highlights": highlights])
    }
    func render() {
        guard let document = view?.document else { return }
        let present = Set(annotations.map(\.id))
        for id in rendered.subtracting(present) { for (page, mark) in renderedMarks.removeValue(forKey: id) ?? [] { page.removeAnnotation(mark) }; rendered.remove(id) }
        for annotation in annotations where !rendered.contains(annotation.id) {
            guard let geometry = annotation.inkGeometry?.object, geometry["space"] as? String == "pdf_page", geometry["content_fingerprint"] as? String == fingerprint, let pageNumber = geometry["page"] as? NSNumber, let page = document.page(at: pageNumber.intValue - 1) else { continue }
            if annotation.strokes.isEmpty && annotation.nativeDrawing == nil {
                for rect in geometry["text_rects"] as? [[String: Any]] ?? [] {
                    guard let x = rect["x"] as? NSNumber, let y = rect["y"] as? NSNumber, let w = rect["width"] as? NSNumber, let h = rect["height"] as? NSNumber else { continue }
                    let mark = PDFAnnotation(bounds: CGRect(x: x.doubleValue, y: y.doubleValue, width: w.doubleValue, height: h.doubleValue), forType: .highlight, withProperties: nil)
                    #if os(macOS)
                    mark.color = NSColor.systemYellow.withAlphaComponent(0.35)
                    #else
                    mark.color = UIColor.systemYellow.withAlphaComponent(0.35)
                    #endif
                    mark.contents = annotation.originalContext; mark.userName = "Book Nook"; page.addAnnotation(mark); renderedMarks[annotation.id, default: []].append((page, mark))
                }
            } else {
                // The portable points render on both platforms, independent of
                // Apple's PKDrawing encoding. They are in PDF page coordinates.
                for value in annotation.strokes {
                    guard let stroke = value.object, let points = stroke["points"] as? [[String: Any]], let first = points.first, let x = first["x"] as? NSNumber, let y = first["y"] as? NSNumber else { continue }
                    let box = page.bounds(for: .cropBox)
                    let mark = PDFAnnotation(bounds: box, forType: .ink, withProperties: nil)
                    let hex = stroke["color"] as? String ?? ""
                    let rgb = hex.count == 7 && hex.first == "#" ? UInt32(hex.dropFirst(), radix: 16) ?? 0x4d7050 : 0x4d7050
                    let red = CGFloat((rgb >> 16) & 255) / 255, green = CGFloat((rgb >> 8) & 255) / 255, blue = CGFloat(rgb & 255) / 255
                    #if os(macOS)
                    let path = NSBezierPath(); mark.color = NSColor(red: red, green: green, blue: blue, alpha: 1)
                    #else
                    let path = UIBezierPath(); mark.color = UIColor(red: red, green: green, blue: blue, alpha: 1)
                    #endif
                    path.move(to: CGPoint(x: x.doubleValue - box.minX, y: y.doubleValue - box.minY))
                    for point in points.dropFirst() { if let x = point["x"] as? NSNumber, let y = point["y"] as? NSNumber {
                        #if os(macOS)
                        path.line(to: CGPoint(x: x.doubleValue - box.minX, y: y.doubleValue - box.minY))
                        #else
                        path.addLine(to: CGPoint(x: x.doubleValue - box.minX, y: y.doubleValue - box.minY))
                        #endif
                    } }
                    let border = PDFBorder(); border.lineWidth = (stroke["width"] as? NSNumber)?.doubleValue ?? 3; mark.border = border; mark.add(path); mark.contents = annotation.originalContext; page.addAnnotation(mark); renderedMarks[annotation.id, default: []].append((page, mark))
                }
            }
            rendered.insert(annotation.id)
        }
        #if os(iOS)
        for overlay in overlays.values { overlay.setMarkup(markup) }
        #endif
    }
}

#if os(macOS)
struct NativePDFView: NSViewRepresentable {
    let url: URL; let initialPage: Int?; let fingerprint: String; let markup: Bool; let annotations: [Annotation]
    let onMessage: ([String: Any]) -> Void; let onInk: AnnotationSave
    func makeCoordinator() -> PDFReaderCoordinator { PDFReaderCoordinator(onMessage: onMessage, onInk: onInk) }
    func makeNSView(context: Context) -> PDFView { let view = PDFView(); view.autoScales = true; view.backgroundColor = NSColor(red: 247.0 / 255, green: 242.0 / 255, blue: 229.0 / 255, alpha: 1); context.coordinator.connect(view); return view }
    func updateNSView(_ view: PDFView, context: Context) { let c = context.coordinator; c.onMessage = onMessage; c.fingerprint = fingerprint; c.initialPage = initialPage; c.annotations = annotations; c.load(url); c.render() }
}
#else
struct NativePDFView: UIViewRepresentable {
    let url: URL; let initialPage: Int?; let fingerprint: String; let markup: Bool; let annotations: [Annotation]
    let onMessage: ([String: Any]) -> Void; let onInk: AnnotationSave
    var inkSession: ReaderInkSession? = nil
    func makeCoordinator() -> PDFReaderCoordinator { PDFReaderCoordinator(onMessage: onMessage, onInk: onInk) }
    func makeUIView(context: Context) -> PDFView { let view = PDFView(); view.autoScales = true; view.backgroundColor = UIColor(red: 247.0 / 255, green: 242.0 / 255, blue: 229.0 / 255, alpha: 1); context.coordinator.connect(view); return view }
    func updateUIView(_ view: PDFView, context: Context) {
        let c = context.coordinator
        if c.inkSession !== inkSession {
            for overlay in c.overlays.values { overlay.capture() }
            c.overlays = [:]
            // A pass change needs new overlays with the new immutable session.
            view.document = nil
        }
        c.onMessage = onMessage; c.onInk = onInk; c.inkSession = inkSession
        c.fingerprint = fingerprint; c.initialPage = initialPage; c.annotations = annotations; c.markup = markup; c.load(url); c.render()
    }
}
extension PDFReaderCoordinator: PDFPageOverlayViewProvider {
    func pdfView(_ view: PDFView, overlayViewFor page: PDFPage) -> UIView? {
        guard let document = view.document else { return nil }
        let index = document.index(for: page)
        let context = ReaderInkContext(resource: "book.pdf", page: index + 1, contentFingerprint: fingerprint)
        let resolution = ReaderInkResolution(anchor: ["kind": "page", "page": index + 1],
            geometry: geometry(for: page, rects: [], quote: nil), originalContext: "Pencil markup on PDF page \(index + 1)")
        let overlay = PDFInkOverlay(page: page, pdfView: view, session: inkSession, context: context, resolution: resolution)
        overlay.setMarkup(markup)
        overlays[index] = overlay; return overlay
    }
    func pdfView(_ view: PDFView, willEndDisplayingOverlayView overlayView: UIView, for page: PDFPage) { (overlayView as? PDFInkOverlay)?.capture(); if let index = view.document?.index(for: page) { overlays.removeValue(forKey: index) } }
}
final class PDFInkOverlay: UIView, PKCanvasViewDelegate {
    weak var page: PDFPage?
    weak var pdfView: PDFView?
    let canvas = PencilInputCanvas()
    let session: ReaderInkSession?
    let context: ReaderInkContext
    let resolution: ReaderInkResolution
    let captureOwner = UUID()
    var applying = false
    init(page: PDFPage, pdfView: PDFView, session: ReaderInkSession?, context: ReaderInkContext, resolution: ReaderInkResolution) {
        self.page = page; self.pdfView = pdfView; self.session = session; self.context = context; self.resolution = resolution
        super.init(frame: .zero)
        addSubview(canvas); canvas.backgroundColor = .clear; canvas.isOpaque = false; canvas.isScrollEnabled = false; canvas.drawingPolicy = .pencilOnly
        canvas.tool = PKInkingTool(.pen, color: UIColor(red: 0.30, green: 0.44, blue: 0.31, alpha: 0.8), width: 3); canvas.delegate = self; canvas.accessibilityLabel = "Pencil markup on PDF page"
        session?.capturePendingInk[captureOwner] = { [weak self] in self?.capture() }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) unavailable") }
    override func layoutSubviews() { super.layoutSubviews(); canvas.frame = bounds }
    override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? { let hit = super.hitTest(point, with: event); return hit === self ? nil : hit }
    func setMarkup(_ enabled: Bool) { if !enabled { capture() }; canvas.isUserInteractionEnabled = enabled; canvas.isHidden = !enabled }
    func canvasViewDidEndUsingTool(_ canvasView: PKCanvasView) { capture() }
    override func willMove(toWindow newWindow: UIWindow?) {
        if newWindow == nil { capture(); session?.unregister(owner: captureOwner) }
        super.willMove(toWindow: newWindow)
    }
    func capture() {
        guard !applying, !canvas.drawing.strokes.isEmpty, let session else { return }
        // Use PDFKit's conversion API, including crop box origin and rotation.
        var transform: CGAffineTransform?
        if let page, let pdfView {
            let p0 = convert(pdfView.convert(CGPoint.zero, from: page), from: pdfView)
            let px = convert(pdfView.convert(CGPoint(x: 1, y: 0), from: page), from: pdfView)
            let py = convert(pdfView.convert(CGPoint(x: 0, y: 1), from: page), from: pdfView)
            transform = ReaderCoordinates.measuredTransform(origin: p0, xUnit: px, yUnit: py)
        }
        let drawing = transform.map { canvas.drawing.transformed(using: $0.inverted()) } ?? canvas.drawing
        var capturedContext = context; capturedContext.measured = transform != nil
        session.capture(context: capturedContext, bounds: drawing.bounds, strokes: portableInk(drawing),
                        nativeDrawing: drawing.dataRepresentation().base64EncodedString(), resolution: transform == nil ? nil : resolution)
        applying = true; canvas.drawing = PKDrawing(); applying = false
    }
}
#endif
