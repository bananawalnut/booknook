// SPDX-License-Identifier: MPL-2.0
import Foundation
import CoreGraphics
import Vision

/// Raster recognition is an optional, on-device draft. Its source is an immutable
/// ink version; saving the reviewed text must never modify that version or prose.
enum NotebookInkRecognizer {
    static func recognize(_ version: NotebookInkVersion) async throws -> String {
        let cancellation = NotebookRecognitionCancellation()
        let task = Task.detached(priority: .userInitiated) { () throws -> String in
            try Task.checkCancellation()
            let image = try raster(version)
            let request = VNRecognizeTextRequest()
            cancellation.register(request)
            try Task.checkCancellation()
            request.revision = VNRecognizeTextRequestRevision3
            request.recognitionLevel = .accurate
            request.usesLanguageCorrection = true
            request.automaticallyDetectsLanguage = true
            let handler = VNImageRequestHandler(cgImage: image, options: [:])
            try handler.perform([request])
            try Task.checkCancellation()
            let text = (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }
                .joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
            guard !text.isEmpty else {
                throw AuthorizationError.message("No readable text was found. You can write a transcription yourself; your handwriting is still saved.")
            }
            return text
        }
        return try await withTaskCancellationHandler(operation: { try await task.value }, onCancel: { cancellation.cancel(); task.cancel() })
    }

    /// Bounds raster work to 2048 pixels per side. Canonical page coordinates and
    /// original PKDrawing bytes are never rescaled or replaced by this OCR image.
    static func raster(_ version: NotebookInkVersion) throws -> CGImage {
        guard version.canvasWidth.isFinite, version.canvasHeight.isFinite,
              version.canvasWidth > 0, version.canvasHeight > 0 else { throw CocoaError(.coderReadCorrupt) }
        var extent = CGRect.null
        for stroke in version.strokes {
            guard stroke.width.isFinite, stroke.width > 0 else { throw CocoaError(.coderReadCorrupt) }
            for point in stroke.points {
                guard point.x.isFinite, point.y.isFinite else { throw CocoaError(.coderReadCorrupt) }
                let radius = max(1, stroke.width)
                extent = extent.union(CGRect(x: point.x - radius, y: point.y - radius, width: radius * 2, height: radius * 2))
            }
        }
        guard !extent.isNull else { throw AuthorizationError.message("Write something on the page before transcribing it.") }
        let origin = version.canvasSpace.map { CGPoint(x: $0.originX, y: $0.originY) } ?? .zero
        guard origin.x.isFinite, origin.y.isFinite else { throw CocoaError(.coderReadCorrupt) }
        let page = CGRect(origin: origin, size: CGSize(width: version.canvasWidth, height: version.canvasHeight))
        extent = extent.insetBy(dx: -16, dy: -16).intersection(page)
        guard !extent.isNull, extent.width > 0, extent.height > 0 else { throw CocoaError(.coderReadCorrupt) }
        let scale = min(2, 2048 / max(extent.width, extent.height))
        let width = max(1, Int(ceil(extent.width * scale))), height = max(1, Int(ceil(extent.height * scale)))
        guard let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { throw CocoaError(.fileReadTooLarge) }
        context.setFillColor(CGColor(gray: 1, alpha: 1)); context.fill(CGRect(x: 0, y: 0, width: width, height: height))
        context.translateBy(x: 0, y: CGFloat(height)); context.scaleBy(x: scale, y: -scale)
        context.translateBy(x: -extent.minX, y: -extent.minY)
        // Normalize color only in this temporary recognition image so light ink
        // and dark-mode display preferences cannot erase a recognition candidate.
        context.setStrokeColor(CGColor(gray: 0, alpha: 1)); context.setFillColor(CGColor(gray: 0, alpha: 1))
        context.setLineCap(.round); context.setLineJoin(.round)
        for stroke in version.strokes {
            guard let first = stroke.points.first else { continue }
            context.setLineWidth(max(0.5, stroke.width))
            if stroke.points.count == 1 {
                context.fillEllipse(in: CGRect(x: first.x - stroke.width / 2, y: first.y - stroke.width / 2, width: stroke.width, height: stroke.width))
            } else {
                context.beginPath(); context.move(to: CGPoint(x: first.x, y: first.y))
                for point in stroke.points.dropFirst() { context.addLine(to: CGPoint(x: point.x, y: point.y)) }
                context.strokePath()
            }
        }
        guard let image = context.makeImage() else { throw CocoaError(.coderReadCorrupt) }
        return image
    }
}

/// VNImageRequestHandler.perform is synchronous: forward cancellation to Vision,
/// as well as rejecting its late result in the presentation controller.
private final class NotebookRecognitionCancellation: @unchecked Sendable {
    private let lock = NSLock()
    private var request: VNRequest?
    private var cancelled = false
    func register(_ request: VNRequest) {
        lock.lock(); self.request = request; let cancelled = cancelled; lock.unlock()
        if cancelled { request.cancel() }
    }
    func cancel() {
        lock.lock(); cancelled = true; let request = request; lock.unlock()
        request?.cancel()
    }
}
