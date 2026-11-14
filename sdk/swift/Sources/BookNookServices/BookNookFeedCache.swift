// SPDX-License-Identifier: MIT OR Apache-2.0
import Foundation
#if canImport(BookNookNative)
import BookNookNative
#endif

public struct BookNookFeedCacheProviderV1: Sendable {
    public let schema: String
    public let version: UInt32
    public let id: String
    public let available: Bool
    public let protocols: [String]
}
public struct BookNookFeedCacheConfigurationV1: Sendable {
    public let raw: Data
    public let configurationToken: String
    public let providerToken: String
    public let provider: BookNookFeedCacheProviderV1
}
public struct BookNookFeedCacheTextV1: Sendable {
    public enum Representation: String, Sendable { case text, literalMarkup = "literal-markup", xhtmlText = "xhtml-text" }
    public let representation: Representation
    /// Render literally, including literal-markup. This is never HTML approval.
    public let text: String
}
public struct BookNookFeedCacheDateV1: Sendable {
    public enum Status: String, Sendable { case missing, invalid, parsed }
    public let status: Status
    public let raw: String?
    public let unixSeconds: Int64?
    /// May be 1_000_000_000..<2_000_000_000 for a leap second; seconds then
    /// names the preceding non-leap second. Do not reduce this to Foundation Date.
    public let nanoseconds: UInt32?
}
public struct BookNookFeedCacheIdentityV1: Sendable {
    public enum Kind: String, Sendable { case native, derivedSHA256 = "derived-sha256" }
    public let kind: Kind
    public let value: String
}
public struct BookNookFeedCacheLinkV1: Sendable {
    public let url: String
    public let rel: String
}
public struct BookNookFeedCacheEntryV1: Sendable {
    public let variantKey: String
    public let identity: BookNookFeedCacheIdentityV1
    public let title: BookNookFeedCacheTextV1?
    public let authors: [String]
    public let summary: BookNookFeedCacheTextV1?
    public let content: BookNookFeedCacheTextV1?
    public let omittedContent: Bool
    /// Inert addresses; opening one requires a separate explicit host action.
    public let links: [BookNookFeedCacheLinkV1]
    public let omittedLinks: UInt32
    public let published: BookNookFeedCacheDateV1
    public let updated: BookNookFeedCacheDateV1
    public let firstSourceOrdinal: UInt32
    public let duplicateCount: UInt32
    public let conflictingIdentity: Bool
}
public struct BookNookFeedCacheProvenanceV1: Sendable {
    public let sourceID: String
    public let providerID: String
    public let requestedURL: String
    public let finalURL: String
    public let rawSHA256: String
}
public struct BookNookFeedCacheViewV1: Sendable {
    public let schema: String
    public let sourceID: String
    public let configurationToken: String
    public let cacheToken: String
    public let contextSHA256: String
    public let rawSHA256: String
    public let normalizerProfile: String
    public let provenance: BookNookFeedCacheProvenanceV1
    public let title: BookNookFeedCacheTextV1?
    public let entries: [BookNookFeedCacheEntryV1]
    public let lastCheckUnixSeconds: Int64
    public let lastBodySuccessUnixSeconds: Int64
    public let offline: Bool
}
public struct BookNookFeedCacheStatusV1: Sendable {
    public let sourceID: String
    public let locallyActive: Bool
    public let cached: Bool
    public let lastCheckUnixSeconds: Int64?
    public let lastBodySuccessUnixSeconds: Int64?
    public let lastError: String?
    public let stale: Bool
}

private enum FeedCacheWireError: Error { case invalidProducedPayload }
private func decodeCacheWire<T: Decodable>(_ type: T.Type, _ bytes: Data, limit: Int) throws -> T {
    guard bytes.count <= limit else { throw FeedCacheWireError.invalidProducedPayload }
    return try JSONDecoder().decode(type, from: bytes)
}
// Only these private wire records decode native-produced payloads. Public DTOs
// are immutable display values, not constructors for durable/imported cache data.
private struct ProviderWire: Decodable {
    let schema: String, version: UInt32, id: String, available: Bool, protocols: [String]
    func value() -> BookNookFeedCacheProviderV1 { .init(schema: schema, version: version, id: id, available: available, protocols: protocols) }
}
private struct TextWire: Decodable {
    let representation: String, text: String
    func value() throws -> BookNookFeedCacheTextV1 {
        guard let kind = BookNookFeedCacheTextV1.Representation(rawValue: representation), text.utf8.count <= 65_536 else { throw FeedCacheWireError.invalidProducedPayload }
        return .init(representation: kind, text: text)
    }
}
private struct DateWire: Decodable {
    let status: String, raw: String?, unix_seconds: Int64?, nanoseconds: UInt32?
    func value() throws -> BookNookFeedCacheDateV1 {
        guard let kind = BookNookFeedCacheDateV1.Status(rawValue: status) else { throw FeedCacheWireError.invalidProducedPayload }
        if kind == .parsed {
            guard raw != nil, unix_seconds != nil, let nanos = nanoseconds, nanos < 2_000_000_000 else { throw FeedCacheWireError.invalidProducedPayload }
        } else {
            guard unix_seconds == nil, nanoseconds == nil, (kind == .missing ? raw == nil : raw != nil) else { throw FeedCacheWireError.invalidProducedPayload }
        }
        return .init(status: kind, raw: raw, unixSeconds: unix_seconds, nanoseconds: nanoseconds)
    }
}
private struct IdentityWire: Decodable {
    let kind: String, value: String
    func typed() throws -> BookNookFeedCacheIdentityV1 {
        guard let kind = BookNookFeedCacheIdentityV1.Kind(rawValue: kind) else { throw FeedCacheWireError.invalidProducedPayload }
        return .init(kind: kind, value: value)
    }
}
private struct LinkWire: Decodable { let url: String, rel: String }
private struct EntryWire: Decodable {
    let identity: IdentityWire, title: TextWire?, authors: [String], summary: TextWire?, content: TextWire?, omitted_content: Bool
    let links: [LinkWire], omitted_links: UInt32, published: DateWire, updated: DateWire
    let first_source_ordinal: UInt32, duplicate_count: UInt32, conflicting_identity: Bool
}
private struct VariantWire: Decodable {
    let variant_key: String, entry: EntryWire
    func value() throws -> BookNookFeedCacheEntryV1 {
        .init(variantKey: variant_key, identity: try entry.identity.typed(), title: try entry.title?.value(), authors: entry.authors,
              summary: try entry.summary?.value(), content: try entry.content?.value(), omittedContent: entry.omitted_content,
              links: entry.links.map { .init(url: $0.url, rel: $0.rel) }, omittedLinks: entry.omitted_links,
              published: try entry.published.value(), updated: try entry.updated.value(), firstSourceOrdinal: entry.first_source_ordinal,
              duplicateCount: entry.duplicate_count, conflictingIdentity: entry.conflicting_identity)
    }
}
private struct ProvenanceWire: Decodable {
    let source_id: String, provider_id: String, requested_url: String, final_url: String, raw_sha256: String
    func value() -> BookNookFeedCacheProvenanceV1 { .init(sourceID: source_id, providerID: provider_id, requestedURL: requested_url, finalURL: final_url, rawSHA256: raw_sha256) }
}
private struct ViewWire: Decodable {
    let schema: String, source_id: String, configuration_token: String, cache_token: String, context_sha256: String, raw_sha256: String, normalizer_profile: String
    let provenance: ProvenanceWire, title: TextWire?, entries: [VariantWire], last_check_unix_seconds: Int64, last_body_success_unix_seconds: Int64, offline: Bool
    func value() throws -> BookNookFeedCacheViewV1 {
        guard schema == "booknook.native-feed-cache-view.v1", normalizer_profile == "booknook.rss2-atom.inert.v1", entries.count <= 500,
              Set(entries.map(\.variant_key)).count == entries.count else { throw FeedCacheWireError.invalidProducedPayload }
        return .init(schema: schema, sourceID: source_id, configurationToken: configuration_token, cacheToken: cache_token,
                     contextSHA256: context_sha256, rawSHA256: raw_sha256, normalizerProfile: normalizer_profile,
                     provenance: provenance.value(), title: try title?.value(), entries: try entries.map { try $0.value() },
                     lastCheckUnixSeconds: last_check_unix_seconds, lastBodySuccessUnixSeconds: last_body_success_unix_seconds, offline: offline)
    }
}
private struct StatusWire: Decodable {
    let source_id: String, locally_active: Bool, cached: Bool, last_check_unix_seconds: Int64?, last_body_success_unix_seconds: Int64?, last_error: String?, stale: Bool
    func value() -> BookNookFeedCacheStatusV1 { .init(sourceID: source_id, locallyActive: locally_active, cached: cached, lastCheckUnixSeconds: last_check_unix_seconds, lastBodySuccessUnixSeconds: last_body_success_unix_seconds, lastError: last_error, stale: stale) }
}

/// Standalone host-local SDK. Share this same object across windows. All public
/// operations are async; no Desk, custody, networking activation or UI side effect
/// is implied by opening/importing. There is no external read-admission method.
public final class BookNookFeedCacheV1: @unchecked Sendable {
    private let handle: FeedCacheControllerV1
    private init(_ handle: FeedCacheControllerV1) { self.handle = handle }
    private static func construct(_ make: @escaping @Sendable () throws -> FeedCacheControllerV1) async throws -> BookNookFeedCacheV1 {
        try Task.checkCancellation()
        let opening = Task.detached(priority: .utility) {
            try Task.checkCancellation()
            let handle = try make()
            if Task.isCancelled { try? await handle.close(operation: FeedCacheOperationV1()); throw CancellationError() }
            return handle
        }
        let handle = try await withTaskCancellationHandler { try await opening.value } onCancel: { opening.cancel() }
        if Task.isCancelled { try? await handle.close(operation: FeedCacheOperationV1()); throw CancellationError() }
        return BookNookFeedCacheV1(handle)
    }
    public static func open(directory: URL, expectedConfiguration: Data? = nil, provider: any BookNookFeedSourceProviderV1) async throws -> BookNookFeedCacheV1 {
        guard directory.isFileURL else { throw FeedCacheWireError.invalidProducedPayload }
        return try await construct { try FeedCacheControllerV1(directory: directory.path, expectedConfiguration: expectedConfiguration, host: SourceBridge(provider)) }
    }
    public static func publicHTTPS(directory: URL, expectedConfiguration: Data? = nil) async throws -> BookNookFeedCacheV1 {
        guard directory.isFileURL else { throw FeedCacheWireError.invalidProducedPayload }
        return try await construct { try FeedCacheControllerV1.publicHttps(directory: directory.path, expectedConfiguration: expectedConfiguration) }
    }
    private func perform<T>(_ action: (FeedCacheOperationV1) async throws -> T) async throws -> T {
        try Task.checkCancellation()
        let operation = FeedCacheOperationV1()
        return try await withTaskCancellationHandler { try await action(operation) } onCancel: { operation.cancel() }
        // No post-await check: cancellation never retracts an accepted commit.
    }
    public func configuration() async throws -> BookNookFeedCacheConfigurationV1 {
        try await perform { operation in
            let value = try await handle.configuration(operation: operation)
            guard value.raw.count <= 266_240 else { throw FeedCacheWireError.invalidProducedPayload }
            let provider = try decodeCacheWire(ProviderWire.self, Data(value.providerDescriptorJson.utf8), limit: 16_384)
            return .init(raw: value.raw, configurationToken: value.configurationToken, providerToken: value.providerToken, provider: provider.value())
        }
    }
    public func replaceConfiguration(expectedToken: String, raw: Data) async throws {
        try await perform { try await handle.replaceConfiguration(expectedToken: expectedToken, raw: raw, operation: $0) }
    }
    public func replaceProvider(expectedToken: String, provider: any BookNookFeedSourceProviderV1) async throws {
        try await perform { try await handle.replaceProvider(expectedToken: expectedToken, host: SourceBridge(provider), operation: $0) }
    }
    public func activate(sourceID: String) async throws { try await perform { try await handle.activate(sourceId: sourceID, operation: $0) } }
    public func pause(sourceID: String) async throws { try await perform { try await handle.pause(sourceId: sourceID, operation: $0) } }
    public func refresh(sourceID: String) async throws -> BookNookFeedCacheViewV1 {
        try await perform { try decodeCacheWire(ViewWire.self, await handle.refresh(sourceId: sourceID, operation: $0), limit: 5 * 1024 * 1024).value() }
    }
    public func offline(sourceID: String) async throws -> BookNookFeedCacheViewV1 {
        try await perform { try decodeCacheWire(ViewWire.self, await handle.offline(sourceId: sourceID, operation: $0), limit: 5 * 1024 * 1024).value() }
    }
    public func status(sourceID: String) async throws -> BookNookFeedCacheStatusV1 {
        try await perform { try decodeCacheWire(StatusWire.self, await handle.status(sourceId: sourceID, operation: $0), limit: 16_384).value() }
    }
    /// Cleanup is intentionally awaited even if this Swift task is cancelled.
    public func close() async throws { try await handle.close(operation: FeedCacheOperationV1()) }
}
