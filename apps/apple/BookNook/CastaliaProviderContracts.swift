// SPDX-License-Identifier: MPL-2.0
import Foundation

/// Public shared Castalia contracts. These references never contain credential bytes.
struct ProviderCredentialReference: Codable, Equatable {
    let reference: String
    let storage: String
    let portability: String
}
struct ProviderConnection: Codable, Equatable, Identifiable {
    let schema: String
    let connectionId: String
    let providerId: String
    let displayName: String
    let capabilities: [String]
    let status: String
    let credential: ProviderCredentialReference?
    enum CodingKeys: String, CodingKey { case schema, connectionId, providerId, displayName, capabilities, status, credential }
    func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        try values.encode(schema, forKey: .schema)
        try values.encode(connectionId, forKey: .connectionId)
        try values.encode(providerId, forKey: .providerId)
        try values.encode(displayName, forKey: .displayName)
        try values.encode(capabilities, forKey: .capabilities)
        try values.encode(status, forKey: .status)
        if let credential { try values.encode(credential, forKey: .credential) }
        else { try values.encodeNil(forKey: .credential) }
    }
    var id: String { connectionId }
    var statusLabel: String {
        switch status {
        case "available": return "Available"
        case "reauthorization_required": return "Sign in again"
        default: return "Unavailable"
        }
    }
    var portabilityLabel: String {
        guard let credential else { return "No credential reference" }
        switch credential.portability {
        case "device_bound": return "Access stays on the source device"
        case "reauthorization_required": return "Sign in again on another device"
        default: return "Portability is defined by the provider"
        }
    }
}
struct CastaliaAuthorityProfile: Codable, Equatable {
    struct Caller: Codable, Equatable { let kind: String; let id: String }
    let schema: String
    let displayName: String
    let issuer: String
    let keyId: String
    let issuerPublicKey: String
    let audience: String
    let caller: Caller
    let policyDigestSha256: String
    let credentialEndpoint: String
}
struct CastaliaProfilePreview: Decodable {
    let schema: String
    let profile: CastaliaAuthorityProfile
    let profileDigestSha256: String
    let settingsDigestSha256: String
}

extension DeskRepository {
    func saveConnectionsIfCurrent(_ settings: ConnectionConfiguration, expected: ConnectionConfiguration) throws -> ConnectionConfiguration {
        let encoder = JSONEncoder(); encoder.keyEncodingStrategy = .convertToSnakeCase
        let result = try required().saveConnectionSettingsIfCurrent(
            settingsJson: String(decoding: encoder.encode(settings), as: UTF8.self),
            expectedSettingsJson: String(decoding: encoder.encode(expected), as: UTF8.self))
        let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(ConnectionConfiguration.self, from: Data(result.utf8))
    }
    func previewCastaliaProfile(_ bytes: Data) throws -> CastaliaProfilePreview {
        let result = try required().previewCastaliaProfile(profileJson: String(decoding: bytes, as: UTF8.self))
        let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(CastaliaProfilePreview.self, from: Data(result.utf8))
    }
    func installCastaliaProfile(_ bytes: Data, preview: CastaliaProfilePreview, holder: String) throws -> ConnectionConfiguration {
        let result = try required().installCastaliaProfile(profileJson: String(decoding: bytes, as: UTF8.self),
            expectedProfileDigest: preview.profileDigestSha256, expectedSettingsDigest: preview.settingsDigestSha256,
            holderPublicKey: holder)
        let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(ConnectionConfiguration.self, from: Data(result.utf8))
    }
    func exportCastaliaProfile() throws -> Data { Data(try required().exportCastaliaProfile().utf8) }
    func saveProviderConnections(_ connections: [ProviderConnection]) throws -> ConnectionConfiguration {
        let encoder = JSONEncoder(); encoder.keyEncodingStrategy = .convertToSnakeCase
        let result = try required().saveProviderConnections(connectionsJson: String(decoding: encoder.encode(connections), as: UTF8.self))
        let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(ConnectionConfiguration.self, from: Data(result.utf8))
    }
    func importKindle(_ catalog: Data, at: UInt64, connection: String) async throws -> DeskSnapshot {
        // Source observation times remain in the original catalog. Native commit
        // records its actual receipt time, independently of this legacy call argument.
        _ = at
        return try await importKindleData(catalog, connection: sourceConnection(connection))
    }
}
