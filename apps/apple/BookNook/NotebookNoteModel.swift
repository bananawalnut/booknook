// SPDX-License-Identifier: MPL-2.0
import Foundation

/// The edition and audio observations are separate source locations, never a
/// conversion between audiobook time and ebook percentage.
struct NotebookCaptureContext: Codable {
    let passId: String
    let positions: [JSONValue]
    let positionObservations: [PositionObservation]
    static func empty(passID: String) -> Self {
        Self(passId: passID, positions: [], positionObservations: [])
    }
    var dictionary: [String: Any] { get throws {
        let encoder = JSONEncoder(); encoder.keyEncodingStrategy = .convertToSnakeCase
        guard let value = try JSONSerialization.jsonObject(with: encoder.encode(self)) as? [String: Any] else {
            throw CocoaError(.coderInvalidValue)
        }
        return value
    } }
}
struct NotebookInkCapture {
    let at: UInt64
    let context: NotebookCaptureContext
}
struct NotebookNoteCreation: Codable {
    let at: UInt64
    let captureContext: NotebookCaptureContext
    var dictionary: [String: Any] { get throws {
        ["at": at, "capture_context": try captureContext.dictionary]
    } }
}
struct NoteCreationContext: Decodable {
    let source: String
    let at: UInt64?
    let reader: String
    let passId: String
    let positions: [JSONValue]
    let positionObservations: [PositionObservation]
}
struct NoteMetadata: Decodable {
    let schema: String
    let face: String
    let created: NoteCreationContext
}
struct NotebookCanvasSpace: Codable, Equatable {
    let kind: String
    let originX: Double
    let originY: Double
    var dictionary: [String: Any] { ["kind": kind, "origin_x": originX, "origin_y": originY] }
}
extension NoteBlock {
    var face: String { note?.face ?? "text" }
    var contextLabel: String {
        guard let created = note?.created else {
            return text.isEmpty && ink == nil ? "" : "Earlier note"
        }
        if created.source == "pending" { return "" }
        var parts: [String] = []
        if created.source == "history_fallback" || created.source == "unknown" { parts.append("Earlier note") }
        if let at = created.at {
            parts.append(Date(timeIntervalSince1970: TimeInterval(at)).formatted(date: .abbreviated, time: .shortened))
        }
        for position in created.positions {
            guard let location = position.object else { continue }
            switch location["kind"] as? String {
            case "page": parts.append("Page \((location["page"] as? NSNumber)?.stringValue ?? "—")")
            case "text":
                parts.append("Book passage")
            case "audio":
                let seconds = ((location["milliseconds"] as? NSNumber)?.uint64Value ?? 0) / 1000
                parts.append("Audio \(seconds / 3600):\(String(format: "%02d", (seconds / 60) % 60)):\(String(format: "%02d", seconds % 60))")
            default: parts.append("\(location["provider"] as? String ?? "Source") location")
            }
        }
        return parts.joined(separator: " · ")
    }
}
