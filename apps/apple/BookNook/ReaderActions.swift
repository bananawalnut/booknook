// SPDX-License-Identifier: MPL-2.0
import Foundation

/// Captured before a renderer callback is queued. Asset identity alone cannot
/// distinguish two reading passes over the same book.
struct ReaderSourceContext: Equatable {
    let passID: String
    let editionID: String
    let assetID: String
    let fingerprint: String
    let resource: String
}

struct ReaderDestination {
    let id = UUID()
    let resource: String
    var fragment: String?
    var anchor: [String: Any]?
}

struct ReaderHighlightRequest {
    let anchor: [String: Any]
    let geometry: [String: Any]?
    let context: String
    var key: String {
        let data = try? JSONSerialization.data(withJSONObject: ["anchor": anchor, "geometry": geometry as Any? ?? NSNull()], options: .sortedKeys)
        return data?.base64EncodedString() ?? "invalid-highlight"
    }
}
