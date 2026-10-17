// SPDX-License-Identifier: MIT OR Apache-2.0
import Foundation
#if canImport(BookNookNative)
import BookNookNative
#endif

public struct CastaliaDisclosure: Codable, Equatable, Sendable {
    public let title: String
    public let statements: [String]
    public init(title: String, statements: [String]) { self.title = title; self.statements = statements }
}
public struct CastaliaCaller: Codable, Equatable, Sendable {
    public let kind: String
    public let id: String
    public init(kind: String, id: String) { self.kind = kind; self.id = id }
}
public struct CastaliaTrustPin: Codable, Equatable, Sendable {
    public let issuer: String
    public let keyID: String
    public let publicKey: String
    public let audience: String
    public let callers: [CastaliaCaller]
    public init(issuer: String, keyID: String, publicKey: String, audience: String, callers: [CastaliaCaller]) {
        self.issuer = issuer; self.keyID = keyID; self.publicKey = publicKey; self.audience = audience; self.callers = callers
    }
    enum CodingKeys: String, CodingKey { case issuer, audience, callers; case keyID = "key_id", publicKey = "public_key" }
}
/// Privileged host configuration. Never derive trust from the incoming issuer response.
public struct CastaliaTrustConfiguration: Codable, Equatable, Sendable {
    public let pins: [CastaliaTrustPin]
    public let expectedCaller: CastaliaCaller
    public let audience: String
    public let holderPublicKey: String
    public let policyDigest: String
    public init(pins: [CastaliaTrustPin], expectedCaller: CastaliaCaller, audience: String, holderPublicKey: String, policyDigest: String) {
        self.pins = pins; self.expectedCaller = expectedCaller; self.audience = audience; self.holderPublicKey = holderPublicKey; self.policyDigest = policyDigest
    }
    enum CodingKeys: String, CodingKey { case pins, audience; case expectedCaller = "expected_caller", holderPublicKey = "holder_public_key", policyDigest = "policy_digest" }
}
public struct CastaliaPolicySnapshot: Equatable, Sendable {
    public let trust: CastaliaTrustConfiguration
    /// Host-owned revision: changing an endpoint, identity selection or policy,
    /// even to an earlier value, should produce a new generation.
    public let generation: String
    public init(trust: CastaliaTrustConfiguration, generation: String) { self.trust = trust; self.generation = generation }
}
public struct CastaliaRequestIntent: Sendable {
    public let canonicalRequestJSON: String
    public let disclosure: CastaliaDisclosure
    public init(canonicalRequestJSON: String, disclosure: CastaliaDisclosure) { self.canonicalRequestJSON = canonicalRequestJSON; self.disclosure = disclosure }
}
public struct CastaliaConsent: Decodable, Identifiable, Sendable {
    public let token: String
    public let title: String
    public let statements: [String]
    public let expiresAt: UInt64
    public let holderPublicKey: String
    public let requestDigestSHA256: String
    public let audience: String
    public let caller: CastaliaCaller
    public var id: String { token }
    enum CodingKeys: String, CodingKey { case token, title, statements, audience, caller; case expiresAt = "expires_at", holderPublicKey = "holder_public_key", requestDigestSHA256 = "request_digest_sha256" }
}
public struct CastaliaReceipt: Sendable {
    public let requestJSON: String
    public let presentationJSON: String
    public init(requestJSON: String, presentationJSON: String) { self.requestJSON = requestJSON; self.presentationJSON = presentationJSON }
}
public enum CastaliaNativeError: LocalizedError {
    case message(String)
    public var errorDescription: String? { if case .message(let text) = self { return text }; return nil }
}
/// A failed write must leave the previous item available. Implementations must
/// namespace records; provider secrets and recovery material are never exports.
public protocol CastaliaSecureMaterialStorage {
    func read(account: String) throws -> Data?
    func saveEncryptedMaterial(_ data: Data, account: String) throws
}
@MainActor public protocol CastaliaCurrentTrustPolicySource {
    func currentPolicy() async throws -> CastaliaPolicySnapshot
    func configuredHolder() async throws -> String?
}
@MainActor public protocol CastaliaAuthorityRequestSource {
    func request(for intent: CastaliaRequestIntent, policy: CastaliaPolicySnapshot) async throws -> Data
}
@MainActor public protocol CastaliaConsentPresenter {
    /// Return true only for a fresh, deliberate user approval of this consent.
    func requestConsent(_ consent: CastaliaConsent) async throws -> Bool
    /// Must end any pending presentation. Cancellation is not approval.
    func cancelConsent(reason: String?)
}
func castaliaJSON<T: Encodable>(_ value: T) throws -> String {
    let encoder = JSONEncoder(); encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
    return String(decoding: try encoder.encode(value), as: UTF8.self)
}
