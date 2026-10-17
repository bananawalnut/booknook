// SPDX-License-Identifier: MIT OR Apache-2.0
import Foundation

/// Encoding conveniences, not a second validator. All original output JSON still
/// crosses the authoritative Rust validator before any result is applied.
public enum BookNookProviderWire {
    public static func json(_ object: Any) throws -> String {
        String(decoding: try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys, .fragmentsAllowed]), as: UTF8.self)
    }

    public static func descriptor(id: String, sourceNamespace: String, capabilities: [String],
                                  available: Bool = true, reason: String? = nil,
                                  connectionJSON: String? = nil) throws -> String {
        // Insert the original connection JSON, so duplicate/unknown keys cannot vanish
        // through Swift decoding before the registry's shared validation.
        let fields = try json(["id": id, "source_namespace": sourceNamespace,
            "capabilities": capabilities, "available": available, "reason": reason as Any? ?? NSNull()] as [String: Any])
        return String(fields.dropLast()) + ",\"connection\":" + (connectionJSON ?? "null") + "}"
    }

    /// Echo the native-authored request identity; do not make up a generation or source.
    public static func response(to requestJSON: String, resultJSON: String,
                                contentBytes: Data = Data()) throws -> BookNookProviderResponse {
        struct Identity: Decodable {
            let request_id: String; let generation: UInt64; let provider_id: String; let connection_id: String?
        }
        let request = try JSONDecoder().decode(Identity.self, from: Data(requestJSON.utf8))
        let header = try json(["schema": "booknook.provider-response.v1", "request_id": request.request_id,
            "generation": request.generation, "provider_id": request.provider_id,
            "connection_id": request.connection_id as Any? ?? NSNull()] as [String: Any])
        return BookNookProviderResponse(responseJSON: String(header.dropLast()) + ",\"result\":" + resultJSON + "}", contentBytes: contentBytes)
    }
}
