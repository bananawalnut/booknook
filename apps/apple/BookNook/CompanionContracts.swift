// SPDX-License-Identifier: MPL-2.0
import Foundation

enum CompanionKindleImport {
    static let maximumBytes = 32 * 1024 * 1024
    enum Format { case sqliteSnapshot, jsonCatalog, xmlCatalog }

    /// Classify file contents, never its extension. The companion validates its schema.
    static func validate(_ bytes: Data, endpoint: URL) throws -> Format {
        _ = try configuredServiceURL(endpoint.absoluteString)
        guard !bytes.isEmpty, bytes.count <= maximumBytes else {
            throw ConnectionInputError.message("Choose a nonempty catalog or snapshot no larger than 32 MiB.")
        }
        if bytes.starts(with: Data("SQLite format 3\0".utf8)) {
            try requireLiteralLoopback(endpoint)
            return .sqliteSnapshot
        }
        // UTF-32LE begins with the UTF-16LE marker, so check the full BOM first.
        let encoding: String.Encoding
        if bytes.starts(with: [0xFF, 0xFE, 0x00, 0x00]) || bytes.starts(with: [0x00, 0x00, 0xFE, 0xFF]) { encoding = .utf32 }
        else if bytes.starts(with: [0xFF, 0xFE]) || bytes.starts(with: [0xFE, 0xFF]) { encoding = .utf16 }
        else { encoding = .utf8 }
        let text = String(data: bytes, encoding: encoding)
        let first = text?.trimmingCharacters(in: .whitespacesAndNewlines.union(CharacterSet(charactersIn: "\u{feff}"))).first
        guard text?.contains("\0") == false else {
            throw ConnectionInputError.message("Choose catalog JSON/XML or a supported standalone Kindle-for-Mac SQLite snapshot.")
        }
        if first == "{" || first == "[" { return .jsonCatalog }
        if first == "<" { return .xmlCatalog }
        throw ConnectionInputError.message("Choose catalog JSON/XML or a supported standalone Kindle-for-Mac SQLite snapshot.")
    }

    static func requireLiteralLoopback(_ endpoint: URL) throws {
        guard ["http", "https"].contains(endpoint.scheme ?? ""),
              ["127.0.0.1", "[::1]", "::1"].contains(endpoint.host ?? ""),
              endpoint.user == nil, endpoint.password == nil, endpoint.query == nil, endpoint.fragment == nil,
              endpoint.port.map({ (1...65535).contains($0) }) ?? true else {
            throw ConnectionInputError.message("Raw Kindle SQLite snapshots can contain private data. Pair a companion on this device using 127.0.0.1 or [::1], or export catalog JSON locally before importing it into a remote companion. Hostnames, including localhost, are not accepted for raw snapshots.")
        }
    }

    static func configureLocalTransport(_ configuration: URLSessionConfiguration, endpoint: URL) throws {
        try requireLiteralLoopback(endpoint)
        // A nil dictionary uses system proxies. CFNetwork's documented enable keys
        // are active only when nonzero; do not change any system proxy settings.
        configuration.connectionProxyDictionary = ["HTTPEnable": 0, "HTTPSEnable": 0,
            "SOCKSEnable": 0, "ProxyAutoConfigEnable": 0, "ProxyAutoDiscoveryEnable": 0]
    }

    /// Keep the security-scoped grant alive through bounded background I/O.
    static func readFile(_ url: URL) async throws -> Data {
        try Task.checkCancellation()
        let reader = Task.detached(priority: .userInitiated) {
            guard url.isFileURL else { throw ConnectionInputError.message("Choose a local catalog file.") }
            let access = url.startAccessingSecurityScopedResource()
            defer { if access { url.stopAccessingSecurityScopedResource() } }
            let values = try url.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey])
            guard values.isRegularFile == true, (values.fileSize ?? 0) <= maximumBytes else {
                throw ConnectionInputError.message("Choose a regular catalog file no larger than 32 MiB.")
            }
            let handle = try FileHandle(forReadingFrom: url)
            defer { try? handle.close() }
            var bytes = Data()
            while true {
                try Task.checkCancellation()
                let chunk = try handle.read(upToCount: min(1_048_576, maximumBytes + 1 - bytes.count)) ?? Data()
                if chunk.isEmpty { break }
                bytes.append(chunk)
                guard bytes.count <= maximumBytes else { throw ConnectionInputError.message("Catalog exceeds 32 MiB.") }
            }
            return bytes
        }
        return try await withTaskCancellationHandler {
            let bytes = try await reader.value
            try Task.checkCancellation()
            return bytes
        } onCancel: {
            reader.cancel()
        }
    }
}

struct CompanionManifest: Decodable {
    let schema: String
    let name: String
    let capabilities: [String]
    let qualified: Bool
}
struct CompanionAudioMetadata: Codable {
    let title: String?
    let authors: [String]?
    let ebookIdentifiers: [CompanionSourceRef]?
    let language: String?
    var isValid: Bool {
        func text(_ value: String, _ limit: Int) -> Bool { !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && value.utf8.count <= limit && value.unicodeScalars.allSatisfy { !CharacterSet.controlCharacters.contains($0) } }
        return (title == nil || text(title!, 2000)) && (language == nil || text(language!, 100)) &&
            (authors?.count ?? 0) <= 100 && (authors ?? []).allSatisfy { text($0, 500) } &&
            (ebookIdentifiers?.count ?? 0) <= 100 && (ebookIdentifiers ?? []).allSatisfy { text($0.provider, 200) && text($0.id, 2000) }
    }
}
struct CompanionSourceRef: Codable { let provider: String; let id: String }
struct CompanionPosition: Codable, Identifiable {
    let asin: String
    let positionMilliseconds: UInt64
    let sourceUpdatedAt: UInt64
    let fetchedAt: UInt64
    let durationMilliseconds: UInt64?
    let editionIdentifier: String?
    let alignment: String
    let metadata: CompanionAudioMetadata?
    var id: String { "\(asin):\(sourceUpdatedAt):\(positionMilliseconds)" }
    func wireJSON() throws -> String {
        let encoder = JSONEncoder(); encoder.keyEncodingStrategy = .convertToSnakeCase
        return String(decoding: try encoder.encode(self), as: UTF8.self)
    }
    func freshness(now: UInt64) -> String {
        if fetchedAt > 9_007_199_254_740_991 || sourceUpdatedAt > 9_007_199_254_740_991 || now > 9_007_199_254_740_991 || fetchedAt > now + 300 || sourceUpdatedAt > fetchedAt + 300 { return "invalid source time" }
        return now > sourceUpdatedAt + 900 ? "stale; ebook alignment unverified" : "recent source observation; ebook alignment unverified"
    }
}
