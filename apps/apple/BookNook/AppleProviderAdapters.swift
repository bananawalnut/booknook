// SPDX-License-Identifier: MPL-2.0
import Foundation
import CryptoKit

enum AppleProviderIDs {
    static let goodreadsFile = "booknook.apple.goodreads-file"
    static let kindleFile = "booknook.apple.kindle-file"
    static let contentFile = "booknook.apple.content-file"
    static let companionCatalog = "booknook.apple.companion-catalog"
    static let kindleWeb = "booknook.apple.kindle-web"
    static let audible = "booknook.apple.companion-audible"
    static let suppliedCatalog = "booknook.apple.supplied-catalog"
}

/// Only native-authored requests reach these adapters; original response JSON is
/// validated by Rust. This decoder routes I/O and never reconciles private records.
private struct AppleProviderRequest: Decodable {
    struct Operation: Decodable {
        let kind: String
        let source_id: String?
        let source_ids: [String]?
        let media_type: String?
    }
    let provider_id: String
    let operation: Operation
    init(_ json: String) throws { self = try JSONDecoder().decode(Self.self, from: Data(json.utf8)) }
}

enum AppleFileProviders {
    static func defaults() throws -> [any BookNookProvider] {
        try [catalog(id: AppleProviderIDs.goodreadsFile, format: "goodreads"),
             catalog(id: AppleProviderIDs.kindleFile, format: "kindle"), content()]
    }
    private static func catalog(id: String, format: String) throws -> any BookNookProvider {
        BookNookClosureProvider(descriptorJSON: try BookNookProviderWire.descriptor(
            id: id, sourceNamespace: format, capabilities: format == "kindle" ? ["catalog", "positions"] : ["catalog"])) { requestJSON in
            let request = try AppleProviderRequest(requestJSON)
            guard request.operation.kind == "catalog", let path = request.operation.source_id else {
                throw BookNookServiceError.message("The file source only provides catalogs.")
            }
            let bytes = try await read(path: path, maximum: 32 * 1024 * 1024)
            return try catalogReply(requestJSON, bytes: bytes, format: format)
        }
    }
    private static func content() throws -> any BookNookProvider {
        BookNookClosureProvider(descriptorJSON: try BookNookProviderWire.descriptor(
            id: AppleProviderIDs.contentFile, sourceNamespace: "booknook.local-files", capabilities: ["content"])) { requestJSON in
            let request = try AppleProviderRequest(requestJSON)
            guard request.operation.kind == "content", let path = request.operation.source_id,
                  let media = request.operation.media_type else {
                throw BookNookServiceError.message("The selected file source only provides book content.")
            }
            let bytes = try await read(path: path, maximum: 128 * 1024 * 1024)
            let hash = SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
            return try BookNookProviderWire.response(to: requestJSON, resultJSON: BookNookProviderWire.json([
                "kind": "content", "source_id": path, "media_type": media, "sha256": hash
            ]), contentBytes: bytes)
        }
    }
    static func catalogReply(_ requestJSON: String, bytes: Data, format: String = "kindle") throws -> BookNookProviderResponse {
        guard bytes.count <= 32 * 1024 * 1024, let text = String(data: bytes, encoding: .utf8) else {
            throw BookNookServiceError.message("Choose a UTF-8 catalog no larger than 32 MiB.")
        }
        return try BookNookProviderWire.response(to: requestJSON, resultJSON: BookNookProviderWire.json([
            "kind": "catalog", "catalog": ["format": format, format == "kindle" ? "json" : "csv": text]
        ]))
    }
    private static func read(path: String, maximum: Int) async throws -> Data {
        let reader = Task.detached(priority: .userInitiated) {
            guard path.hasPrefix("/") else { throw BookNookServiceError.message("Choose a local file.") }
            let url = URL(fileURLWithPath: path)
            let access = url.startAccessingSecurityScopedResource()
            defer { if access { url.stopAccessingSecurityScopedResource() } }
            let attributes = try url.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey])
            guard attributes.isRegularFile == true, (attributes.fileSize ?? 0) <= maximum else {
                throw BookNookServiceError.message("The selected file is not a regular file within the supported size limit.")
            }
            let handle = try FileHandle(forReadingFrom: url)
            defer { try? handle.close() }
            var bytes = Data()
            while true {
                try Task.checkCancellation()
                let chunk = try handle.read(upToCount: min(1_048_576, maximum + 1 - bytes.count)) ?? Data()
                if chunk.isEmpty { return bytes }
                bytes.append(chunk)
                guard bytes.count <= maximum else { throw BookNookServiceError.message("The selected file exceeds the supported size limit.") }
            }
        }
        return try await withTaskCancellationHandler {
            let bytes = try await reader.value
            try Task.checkCancellation()
            return bytes
        } onCancel: { reader.cancel() }
    }
}

/// Short-lived source closures live here only until their one fetch completes.
/// Stable registered adapters do not retain a dismissed WebKit sheet or its cookies.
actor AppleProviderJobs {
    typealias CatalogFetch = @MainActor @Sendable () async throws -> Data
    typealias PositionFetch = @MainActor @Sendable ([String]) async throws -> [CompanionPosition]
    private var catalogs: [String: (String, CatalogFetch)] = [:]
    private var positions: [String: ([String], PositionFetch)] = [:]
    private var activePositions: Set<String> = []

    func stageCatalog(providerID: String, fetch: @escaping CatalogFetch) -> String {
        let key = UUID().uuidString
        catalogs[key] = (providerID, fetch)
        return key
    }
    func removeCatalog(_ key: String) { catalogs.removeValue(forKey: key) }
    func stagePositions(providerID: String, sourceIDs: [String], fetch: @escaping PositionFetch) throws {
        guard !activePositions.contains(providerID) else { throw BookNookServiceError.message("Wait for this source's current position request to finish.") }
        activePositions.insert(providerID)
        positions[providerID] = (sourceIDs, fetch)
    }
    func removePositions(_ providerID: String) { positions.removeValue(forKey: providerID); activePositions.remove(providerID) }
    func execute(_ requestJSON: String) async throws -> BookNookProviderResponse {
        let request = try AppleProviderRequest(requestJSON)
        switch request.operation.kind {
        case "catalog":
            guard let key = request.operation.source_id, let (providerID, fetch) = catalogs.removeValue(forKey: key),
                  providerID == request.provider_id else { throw BookNookServiceError.message("Choose the catalog source again before retrying.") }
            return try AppleFileProviders.catalogReply(requestJSON, bytes: await fetch())
        case "positions":
            guard let sourceIDs = request.operation.source_ids, let (expected, fetch) = positions.removeValue(forKey: request.provider_id),
                  expected == sourceIDs else { throw BookNookServiceError.message("Choose the listening source again before retrying.") }
            let observations = try await fetch(sourceIDs)
            let encoder = JSONEncoder(); encoder.keyEncodingStrategy = .convertToSnakeCase
            let raw = String(decoding: try encoder.encode(observations), as: UTF8.self)
            return try BookNookProviderWire.response(to: requestJSON, resultJSON: "{\"kind\":\"positions\",\"observations\":" + raw + "}")
        default: throw BookNookServiceError.message("This source does not provide the requested operation.")
        }
    }
}

struct AppleCatalogRead {
    let source: BookNookProviderRead
    /// Available only for the existing injected persistence test seam; default
    /// production commit uses the original validated execution, never reconstructed JSON.
    var kindleBytes: Data {
        get throws {
            let execution = try JSONSerialization.jsonObject(with: Data(source.executionJSON.utf8)) as? [String: Any]
            let response = execution?["response"] as? [String: Any]
            let result = response?["result"] as? [String: Any]
            let catalog = result?["catalog"] as? [String: Any]
            guard catalog?["format"] as? String == "kindle", let text = catalog?["json"] as? String else { throw CocoaError(.coderReadCorrupt) }
            return Data(text.utf8)
        }
    }
}

struct ApplePositionRead {
    let source: BookNookProviderRead
    let observations: [CompanionPosition]
}
