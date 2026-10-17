// SPDX-License-Identifier: MPL-2.0
#if os(iOS)
import UIKit
import WebKit
import PencilKit

func inkNumber(_ value: Any?) -> CGFloat? { (value as? NSNumber).map { CGFloat(truncating: $0) } }
func portableInk(_ drawing: PKDrawing) -> [[String: Any]] {
    drawing.strokes.map { stroke in
        let points: [[String: Any]] = stroke.path.map { p in let point = p.location.applying(stroke.transform); return ["x": point.x, "y": point.y, "pressure": min(1, max(0, p.force)), "time": p.timeOffset] }
        var red: CGFloat = 0, green: CGFloat = 0, blue: CGFloat = 0, alpha: CGFloat = 0
        stroke.ink.color.getRed(&red, green: &green, blue: &blue, alpha: &alpha)
        return ["points": points, "color": String(format: "#%02x%02x%02x", Int(red * 255), Int(green * 255), Int(blue * 255)), "width": (stroke.path.first?.size.width ?? 3) * hypot(stroke.transform.a, stroke.transform.b)]
    }
}

/// Saved ink and an in-flight Pencil stroke are separate layers. Finger gestures
/// continue to reach WKWebView, while PKCanvasView captures Pencil input.
final class PencilInputCanvas: PKCanvasView {
    override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? {
        if let touches = event?.allTouches, !touches.isEmpty, touches.allSatisfy({ $0.type != .pencil }) { return nil }
        return super.hitTest(point, with: event)
    }
}
final class InkReaderHost: UIView, PKCanvasViewDelegate {
    let web: WKWebView
    let savedCanvas = PKCanvasView()
    let canvas = PencilInputCanvas()
    var resource = ""
    var annotations: [Annotation] = []
    var onInk: AnnotationSave = { _, acknowledged in acknowledged(false) }
    var onError: (String) -> Void = { _ in }
    private var viewport: CGRect = .zero
    private var placements: [String: [String: Any]] = [:]
    private var applying = false
    private var markup = false
    private var lastTransform = CGAffineTransform.identity
    private var inkSession: ReaderInkSession?
    private let captureOwner = UUID()
    private var contentFingerprint: String?
    private var layoutFingerprint: String?
    private var strokeContext: ReaderInkContext?
    init(web: WKWebView) {
        self.web = web; super.init(frame: .zero)
        addSubview(web); addSubview(savedCanvas); addSubview(canvas)
        for layer in [savedCanvas, canvas] { layer.backgroundColor = .clear; layer.isOpaque = false; layer.isScrollEnabled = false }
        savedCanvas.isUserInteractionEnabled = false
        canvas.drawingPolicy = .pencilOnly; canvas.tool = PKInkingTool(.pen, color: UIColor(red: 0.30, green: 0.44, blue: 0.31, alpha: 0.8), width: 3); canvas.delegate = self
        canvas.accessibilityLabel = "Pencil markup. Use one finger to scroll the book."
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) unavailable") }
    func configure(session: ReaderInkSession?, resource: String) {
        if inkSession !== session || self.resource != resource {
            capture(); inkSession?.unregister(owner: captureOwner)
            inkSession = session; self.resource = resource
            viewport = .zero; contentFingerprint = nil; layoutFingerprint = nil; strokeContext = nil
        }
        session?.capturePendingInk[captureOwner] = { [weak self] in self?.capture() }
        session?.registerResolver(owner: captureOwner, resource: resource) { [weak web] draft in
            guard let web else { throw AuthorizationError.message("Open the original chapter before retrying retained markup.") }
            return try await ReaderInkAnchorResolver.resolve(draft, in: web)
        }
    }
    override func layoutSubviews() { super.layoutSubviews(); web.frame = bounds; savedCanvas.frame = bounds; canvas.frame = bounds; render() }
    func setMarkup(_ value: Bool) { if markup && !value { capture() }; markup = value; canvas.isHidden = !value; canvas.isUserInteractionEnabled = value }
    func receive(_ message: [String: Any]) {
        switch message["type"] as? String {
        case "geometry":
            guard let v = message["viewport"] as? [String: Any], let x = inkNumber(v["x"]), let y = inkNumber(v["y"]), let w = inkNumber(v["width"]), let h = inkNumber(v["height"]), w > 0, h > 0 else { return }
            contentFingerprint = message["contentFingerprint"] as? String
            layoutFingerprint = message["fingerprint"] as? String
            viewport = CGRect(x: x, y: y, width: w, height: h); render()
        case "ink_placements":
            placements = Dictionary(uniqueKeysWithValues: (message["placements"] as? [[String: Any]] ?? []).compactMap { item in (item["id"] as? String).map { ($0, item) } }); render()
        default: break
        }
    }
    private func transform() -> CGAffineTransform? {
        ReaderCoordinates.viewportTransform(viewport: viewport, nativeSize: bounds.size)
    }
    private func render() {
        guard let transform = transform() else { return }
        let strokes = annotations.flatMap { annotation -> [PKStroke] in
            guard let placement = placements[annotation.id], placement["status"] as? String == "resolved", let encoded = annotation.nativeDrawing, let data = Data(base64Encoded: encoded), let drawing = try? PKDrawing(data: data) else { return [] }
            let delta = CGAffineTransform(translationX: inkNumber(placement["dx"]) ?? 0, y: inkNumber(placement["dy"]) ?? 0)
            return drawing.transformed(using: delta.concatenating(transform)).strokes
        }
        savedCanvas.drawing = PKDrawing(strokes: strokes)
        if !canvas.drawing.strokes.isEmpty && lastTransform != transform {
            // Moving the viewport does not change the canonical stroke geometry.
            applying = true; canvas.drawing = canvas.drawing.transformed(using: lastTransform.inverted().concatenating(transform)); applying = false
        }
        lastTransform = transform
    }
    func canvasViewDidBeginUsingTool(_ canvasView: PKCanvasView) { strokeContext = context(); web.scrollView.isScrollEnabled = false; web.scrollView.pinchGestureRecognizer?.isEnabled = false }
    func canvasViewDidEndUsingTool(_ canvasView: PKCanvasView) { capture(); web.scrollView.isScrollEnabled = true; web.scrollView.pinchGestureRecognizer?.isEnabled = true }
    func canvasViewDrawingDidChange(_ canvasView: PKCanvasView) { /* Persist the completed stroke, not partial Pencil samples. */ }
    override func willMove(toWindow newWindow: UIWindow?) { if newWindow == nil { capture(); inkSession?.unregister(owner: captureOwner) }; super.willMove(toWindow: newWindow) }
    private func context() -> ReaderInkContext {
        ReaderInkContext(resource: resource, documentURL: web.url?.absoluteString,
                         contentFingerprint: contentFingerprint, layoutFingerprint: layoutFingerprint,
                         measured: transform() != nil && contentFingerprint != nil && layoutFingerprint != nil)
    }
    private func capture() {
        guard !applying, !canvas.drawing.strokes.isEmpty else { return }
        guard let inkSession else { onError("Markup cannot be saved until its reading session is ready. Keep this page open."); return }
        let capturedContext = strokeContext ?? context()
        let drawing = capturedContext.measured ? canvas.drawing.transformed(using: lastTransform.inverted()) : canvas.drawing
        // Retain the exact bytes and context before clearing the temporary layer.
        // The resolver keeps only WKWebView alive, never its removable host.
        let sourceWeb = web
        inkSession.capture(context: capturedContext, bounds: drawing.bounds, strokes: portableInk(drawing),
                           nativeDrawing: drawing.dataRepresentation().base64EncodedString(), resolver: { draft in
            try await ReaderInkAnchorResolver.resolve(draft, in: sourceWeb)
        })
        applying = true; canvas.drawing = PKDrawing(); applying = false
        strokeContext = nil
    }

}
#endif
