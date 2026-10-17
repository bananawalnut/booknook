// SPDX-License-Identifier: MIT OR Apache-2.0
import Foundation
#if canImport(BookNookNative)
import BookNookNative
#endif

/// A Rust-validated source response bound to the provider generation and connection
/// that produced it. Keeping this value does not authorize a later stale commit.
public struct BookNookProviderRead: Sendable {
    public let executionJSON: String
    public let contentBytes: Data
    public init(executionJSON: String, contentBytes: Data = Data()) {
        self.executionJSON = executionJSON
        self.contentBytes = contentBytes
    }
}

/// Public source composition used by the reference application and SDK hosts.
/// Rust owns validation, reconciliation, asset ingestion and social authority checks.
/// Network/platform UI remains in the explicitly injected providers.
public actor BookNookServices {
    public static let localRecommendations = "booknook.local-recommendations"
    public static let nostrTransport = "booknook.nostr"
    public nonisolated let desk: Desk
    private var providers: [any BookNookProvider]
    private var generation: UInt64 = 1

    public init(directory: URL, providers: [any BookNookProvider] = []) throws {
        let host = try BookNookProviderHost(providers)
        self.desk = try Desk.withProviders(directory: directory.path, host: host, generation: 1)
        self.providers = providers
    }

    public func descriptorsJSON() throws -> String { try desk.providerDescriptors() }
    public func registeredDescriptorJSON(providerID: String) throws -> String? {
        try providers.first { try BookNookProviderHost.identifier($0) == providerID }?.descriptorJSON
    }

    /// Replacement invalidates earlier source reads, including those awaiting human
    /// edition review. A connection descriptor never installs executable code itself.
    public func replaceProviders(_ providers: [any BookNookProvider]) throws {
        guard generation < UInt64.max else { throw BookNookServiceError.message("Provider generation exhausted. Reopen the service.") }
        let host = try BookNookProviderHost(providers)
        let next = generation + 1
        try desk.replaceProviderHost(host: host, generation: next)
        self.providers = providers
        generation = next
    }

    public func register(_ provider: any BookNookProvider) throws {
        let id = try BookNookProviderHost.identifier(provider)
        var replacement = try providers.filter { try BookNookProviderHost.identifier($0) != id }
        replacement.append(provider)
        try replaceProviders(replacement)
    }

    public func remove(providerID: String) throws {
        let replacement = try providers.filter { try BookNookProviderHost.identifier($0) != providerID }
        try replaceProviders(replacement)
    }

    /// Fetch does not mutate the notebook. UniFFI does not propagate Swift parent
    /// cancellation to a foreign callback; check again before every synchronous commit.
    public func fetch(providerID: String, operationJSON: String) async throws -> BookNookProviderRead {
        try Task.checkCancellation()
        do {
            let reply = try await desk.providerFetch(providerId: providerID, operationJson: operationJSON)
            try Task.checkCancellation()
            return BookNookProviderRead(executionJSON: reply.responseJson, contentBytes: reply.contentBytes)
        } catch {
            // A cancelled WebKit/network source may throw through the foreign-error
            // bridge before returning. Preserve the parent's cancellation semantics.
            try Task.checkCancellation()
            throw error
        }
    }

    public func commitCatalog(_ read: BookNookProviderRead) throws -> String {
        try Task.checkCancellation()
        guard read.contentBytes.isEmpty else { throw BookNookServiceError.message("A catalog response cannot contain book bytes.") }
        return try desk.providerCommitCatalog(executionJson: read.executionJSON)
    }

    public func importCatalog(providerID: String, sourceID: String) async throws -> String {
        let read = try await fetch(providerID: providerID, operationJSON: BookNookProviderWire.json([
            "kind": "catalog", "source_id": sourceID
        ]))
        return try commitCatalog(read)
    }

    public func commitContent(_ read: BookNookProviderRead, bookID: String) throws -> String {
        try Task.checkCancellation()
        return try desk.providerCommitContent(executionJson: read.executionJSON, contentBytes: read.contentBytes, bookId: bookID)
    }

    public func importContent(providerID: String, bookID: String, sourceID: String, mediaType: String) async throws -> String {
        let read = try await fetch(providerID: providerID, operationJSON: BookNookProviderWire.json([
            "kind": "content", "source_id": sourceID, "media_type": mediaType
        ]))
        return try commitContent(read, bookID: bookID)
    }

    public func positions(providerID: String, sourceIDs: [String]) async throws -> BookNookProviderRead {
        try await fetch(providerID: providerID, operationJSON: BookNookProviderWire.json([
            "kind": "positions", "source_ids": sourceIDs
        ]))
    }

    /// Opaque source-native positions remain unmatched; the SDK does not infer
    /// ebook pages or percentages from an unrelated provider's location vocabulary.
    public func sourcePositions(providerID: String, sourceIDs: [String]) async throws -> BookNookProviderRead {
        try await fetch(providerID: providerID, operationJSON: BookNookProviderWire.json([
            "kind": "source_positions", "source_ids": sourceIDs
        ]))
    }
    public func attachPosition(_ read: BookNookProviderRead, observationJSON: String, passID: String) throws -> String {
        try Task.checkCancellation()
        guard read.contentBytes.isEmpty else { throw BookNookServiceError.message("A position response cannot contain book bytes.") }
        return try desk.providerAttachPosition(executionJson: read.executionJSON, observationJson: observationJSON, passId: passID)
    }

    /// Call after explicit source/edition/pass review. No audio percentage becomes
    /// ebook progress; native code revalidates the captured provider before saving.
    public func attachAudio(_ read: BookNookProviderRead, observationJSON: String,
                            passID: String, confirmedEditionID: String?) throws -> String {
        try Task.checkCancellation()
        guard read.contentBytes.isEmpty else { throw BookNookServiceError.message("A position response cannot contain book bytes.") }
        return try desk.providerAttachAudio(executionJson: read.executionJSON, observationJson: observationJSON,
            passId: passID, confirmedEditionId: confirmedEditionID)
    }

    public func identity(providerID: String, entityIRI: String) async throws -> BookNookProviderRead {
        try await fetch(providerID: providerID, operationJSON: BookNookProviderWire.json([
            "kind": "identity", "entity_iri": entityIRI
        ]))
    }

    /// Native code assembles the social projection only after independently auditing
    /// its receipts. The provider receives no private notebook or group conversations.
    public func recommendations(providerID: String = BookNookServices.localRecommendations,
                                candidatesJSON: String, excludedJSON: String) async throws -> String {
        try Task.checkCancellation()
        do {
            let result = try await desk.providerSocialRecommendations(providerId: providerID,
                candidatesJson: candidatesJSON, excludedJson: excludedJSON)
            try Task.checkCancellation()
            return result
        } catch { try Task.checkCancellation(); throw error }
    }

    /// Delivers previously approved events. Cancellation does not retract a packet
    /// already sent. Only actual provider acknowledgments can advance the native outbox.
    public func exchange(providerID: String = BookNookServices.nostrTransport,
                         secret: Data, discover: Bool) async throws -> String {
        try Task.checkCancellation()
        let desk = self.desk
        return try await Task.detached(priority: .utility) {
            try desk.providerSocialExchange(providerId: providerID, secret: secret, discover: discover)
        }.value
    }
}

/// The bridge snapshots descriptor bytes without discarding unknown/duplicate keys.
/// Its dictionary is used only after the native constructor validates the registry.
private final class BookNookProviderHost: ProviderHost, @unchecked Sendable {
    private let registryJSON: String
    private let sources: [String: any BookNookProvider]
    private struct Identifier: Decodable { let id: String }
    private struct RequestIdentity: Decodable { let provider_id: String }
    static func identifier(_ provider: any BookNookProvider) throws -> String {
        try JSONDecoder().decode(Identifier.self, from: Data(provider.descriptorJSON.utf8)).id
    }
    init(_ providers: [any BookNookProvider]) throws {
        var sources: [String: any BookNookProvider] = [:]
        for provider in providers {
            let id = try Self.identifier(provider)
            guard sources[id] == nil else { throw BookNookServiceError.message("A provider ID was registered twice: \(id)") }
            sources[id] = provider
        }
        self.sources = sources
        self.registryJSON = "{\"schema\":\"booknook.provider-registry.v1\",\"providers\":[" + providers.map(\.descriptorJSON).joined(separator: ",") + "]}"
    }
    func descriptorsJson() throws -> String { registryJSON }
    func execute(requestJson: String) async throws -> ProviderReply {
        do {
            let request = try JSONDecoder().decode(RequestIdentity.self, from: Data(requestJson.utf8))
            guard let provider = sources[request.provider_id] else { throw BookNookServiceError.message("The selected source is no longer registered.") }
            let response = try await provider.execute(requestJSON: requestJson)
            return ProviderReply(responseJson: response.responseJSON, contentBytes: response.contentBytes)
        } catch {
            // Foreign callbacks must return the declared native error, not an opaque
            // unexpected-foreign-exception string that loses the actionable message.
            throw NativeError.Failure(message: error.localizedDescription)
        }
    }
}
