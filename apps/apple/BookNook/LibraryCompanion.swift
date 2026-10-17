// SPDX-License-Identifier: MPL-2.0
import Foundation
import CryptoKit

struct CompanionAuthorization: Codable, Identifiable {
    let authorizationUrl: String
    let sessionId: String?
    let expiresAt: UInt64?
    let requiresVirtualDeviceRegistration: Bool?
    var id: String { sessionId ?? authorizationUrl }
    func validatedURL() throws -> URL {
        let hosts = ["www.amazon.com", "www.amazon.co.uk", "www.amazon.de", "www.amazon.ca", "www.amazon.com.au", "www.audible.com", "www.audible.co.uk", "www.audible.de", "www.audible.ca", "www.audible.com.au"]
        guard let url = URL(string: authorizationUrl), url.scheme == "https", hosts.contains(url.host ?? ""), url.user == nil, url.password == nil, url.port == nil, url.fragment == nil else { throw AuthorizationError.message("Companion login URL is not an approved Amazon/Audible authorization origin.") }
        if let sessionId {
            let now = UInt64(Date().timeIntervalSince1970)
            guard sessionId.count == 64, sessionId.allSatisfy({ $0.isHexDigit && !$0.isUppercase }), let expiresAt, expiresAt > now, expiresAt <= now + 900, requiresVirtualDeviceRegistration == true else { throw AuthorizationError.message("Companion returned an invalid or expired login session.") }
        } else if expiresAt != nil || requiresVirtualDeviceRegistration != nil { throw AuthorizationError.message("Companion returned an incomplete login session.") }
        return url
    }
}

struct LibraryCompanion {
    private let base: URL
    private let account: String
    private let decoder: JSONDecoder = { let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase; return decoder }()
    init(endpoint: String) throws {
        base = try configuredServiceURL(endpoint)
        account = "companion:" + SHA256.hash(data: Data(base.absoluteString.utf8)).map { String(format: "%02x", $0) }.joined()
    }
    private func request(_ path: String, method: String = "GET", body: [String: Any]? = nil, authenticated: Bool = true) throws -> URLRequest {
        var r = URLRequest(url: base.appendingPathComponent(path)); r.httpMethod = method; r.timeoutInterval = 25
        if let body { r.httpBody = try JSONSerialization.data(withJSONObject: body); r.setValue("application/json", forHTTPHeaderField: "Content-Type") }
        if authenticated {
            guard let data = try CustodySecureStorage().read(account: account), let token = String(data: data, encoding: .utf8), token.count == 64, token.allSatisfy({ $0.isHexDigit && !$0.isUppercase }) else { throw AuthorizationError.message("Pair this companion first.") }
            r.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        return r
    }
    func pair(code: String) async throws -> CompanionManifest {
        guard code.count >= 12, code.count <= 128, !code.contains("\n") else { throw AuthorizationError.message("Enter the pairing code displayed by the companion.") }
        let data = try await ConnectorHTTP.send(request("v1/pair", method: "POST", body: ["schema": "booknook.companion-pair.v1", "code": code, "device_name": "Book Nook native"], authenticated: false), limit: 16384)
        struct Reply: Decodable { let schema: String; let token: String; let manifest: CompanionManifest }
        let reply = try decoder.decode(Reply.self, from: data)
        guard reply.schema == "booknook.companion-paired.v1", reply.token.count == 64, reply.token.allSatisfy({ $0.isHexDigit && !$0.isUppercase }), reply.manifest.schema == "booknook.companion.v1" else { throw AuthorizationError.message("Companion returned an unsupported pairing response.") }
        try CustodySecureStorage().saveEncryptedMaterial(Data(reply.token.utf8), account: account)
        return reply.manifest
    }
    func manifest() async throws -> CompanionManifest {
        let data = try await ConnectorHTTP.send(request("v1/capabilities"), limit: 16384)
        let result = try decoder.decode(CompanionManifest.self, from: data)
        guard result.schema == "booknook.companion.v1" else { throw AuthorizationError.message("Unsupported companion contract.") }
        return result
    }
    func providerConnections(marketplace: String) async throws -> [ProviderConnection] {
        guard ["us", "uk", "de", "ca", "au"].contains(marketplace) else { throw AuthorizationError.message("Choose a supported marketplace.") }
        let data = try await ConnectorHTTP.send(request("v1/provider-connections/" + marketplace), limit: 1_048_576)
        guard let raw = String(data: data, encoding: .utf8) else { throw AuthorizationError.message("Provider connections must be UTF-8 JSON.") }
        // Validate the original bytes before Codable can ignore unknown fields.
        let validated = try validateProviderConnections(connectionsJson: raw)
        return try decoder.decode([ProviderConnection].self, from: Data(validated.utf8))
    }
    func disconnect() async throws {
        defer { try? CustodySecureStorage().remove(account: account) }
        _ = try await ConnectorHTTP.send(request("v1/unpair", method: "POST", body: [:]), limit: 16384)
    }
    func beginAuthorization(provider: String, marketplace: String) async throws -> CompanionAuthorization {
        guard ["kindle", "audible"].contains(provider) else { throw AuthorizationError.message("Unsupported source provider.") }
        let data = try await ConnectorHTTP.send(request("v1/accounts/authorize", method: "POST", body: ["provider": provider, "marketplace": marketplace]), limit: 32768)
        let reply = try decoder.decode(CompanionAuthorization.self, from: data)
        _ = try reply.validatedURL()
        return reply
    }
    func completeAuthorization(provider: String, marketplace: String, sessionId: String, returnURL: String) async throws {
        guard returnURL.utf8.count <= 32768 else { throw AuthorizationError.message("The returned authorization URL is too long.") }
        let data = try await ConnectorHTTP.send(request("v1/accounts/authorize/complete", method: "POST", body: ["provider": provider, "marketplace": marketplace, "session_id": sessionId, "return_url": returnURL, "consent_virtual_device": true]), limit: 16384)
        struct Reply: Decodable { let schema: String; let provider: String; let connected: Bool; let positionsQualified: Bool }
        let reply = try decoder.decode(Reply.self, from: data)
        guard reply.schema == "booknook.account-connected.v1", reply.provider == provider, reply.connected else { throw AuthorizationError.message("The source did not confirm account registration.") }
    }
    func cancelAuthorization(provider: String, marketplace: String, sessionId: String) async throws {
        let data = try await ConnectorHTTP.send(request("v1/accounts/authorize/cancel", method: "POST", body: ["provider": provider, "marketplace": marketplace, "session_id": sessionId]), limit: 16384)
        struct Reply: Decodable { let schema: String; let cancelled: Bool }
        let reply = try decoder.decode(Reply.self, from: data)
        guard reply.schema == "booknook.account-cancelled.v1", reply.cancelled else { throw AuthorizationError.message("The source did not confirm login cancellation.") }
    }
    func disconnectAccount(provider: String, marketplace: String) async throws {
        let data = try await ConnectorHTTP.send(request("v1/accounts/disconnect", method: "POST", body: ["provider": provider, "marketplace": marketplace]), limit: 16384)
        struct Reply: Decodable { let schema: String; let provider: String; let disconnected: Bool }
        let reply = try decoder.decode(Reply.self, from: data)
        guard reply.schema == "booknook.account-disconnected.v1", reply.provider == provider, reply.disconnected else { throw AuthorizationError.message("The source did not confirm account disconnection.") }
    }
    func resetUnfinishedRegistration(provider: String, marketplace: String) async throws {
        let data = try await ConnectorHTTP.send(request("v1/accounts/registration/reset", method: "POST", body: ["provider": provider, "marketplace": marketplace, "confirmed_amazon_device_review": true]), limit: 16384)
        struct Reply: Decodable { let schema: String; let provider: String; let reset: Bool }
        let reply = try decoder.decode(Reply.self, from: data)
        guard reply.schema == "booknook.account-registration-reset.v1", reply.provider == provider, reply.reset else { throw AuthorizationError.message("The source did not confirm clearing its unfinished registration.") }
    }
    func probeAudibleAccount(marketplace: String, asin: String) async throws {
        let data = try await ConnectorHTTP.send(request("v1/audible/positions/probe", method: "POST", body: ["provider": "audible", "marketplace": marketplace, "asins": [asin]]), limit: 16384)
        struct Reply: Decodable { let schema: String; let receivedJson: Bool; let positionsQualified: Bool }
        let reply = try decoder.decode(Reply.self, from: data)
        guard reply.schema == "booknook.audible-probe.v1", reply.receivedJson, !reply.positionsQualified else { throw AuthorizationError.message("The source returned an unsupported qualification result.") }
    }
    func renewAccount(provider: String, marketplace: String) async throws {
        guard ["kindle", "audible"].contains(provider) else { throw AuthorizationError.message("Unsupported source provider.") }
        let data = try await ConnectorHTTP.send(request("v1/accounts/renew", method: "POST", body: ["provider": provider, "marketplace": marketplace]), limit: 16_384)
        struct Reply: Decodable { let schema: String; let provider: String; let renewed: Bool }
        let reply = try decoder.decode(Reply.self, from: data)
        guard reply.schema == "booknook.account-renewal.v1", reply.provider == provider, reply.renewed else { throw AuthorizationError.message("The source adapter did not confirm account renewal. Reconnect the account.") }
    }
    @discardableResult func importKindleExport(_ bytes: Data) async throws -> CompanionKindleImport.Format {
        // Reject unsafe destinations before reading pairing credentials or creating a request.
        let format = try CompanionKindleImport.validate(bytes, endpoint: base)
        var upload = try request("v1/kindle/catalog", method: "PUT")
        upload.httpBody = bytes; upload.setValue("application/octet-stream", forHTTPHeaderField: "Content-Type")
        _ = try await ConnectorHTTP.send(upload, limit: 16384, localOnly: format == .sqliteSnapshot)
        return format
    }
    func kindleCatalog() async throws -> Data {
        try await ConnectorHTTP.send(request("v1/kindle/catalog"), limit: 32 * 1024 * 1024)
    }
    func audiblePositions(asins: [String], marketplace: String = "us") async throws -> [CompanionPosition] {
        guard !asins.isEmpty, asins.count <= 100, asins.allSatisfy({ $0.count == 10 && $0.unicodeScalars.allSatisfy({ $0.isASCII && CharacterSet.alphanumerics.contains($0) }) }) else { throw AuthorizationError.message("Enter 1–100 valid ASINs.") }
        let data = try await ConnectorHTTP.send(request("v1/audible/positions", method: "POST", body: ["asins": asins, "marketplace": marketplace]), limit: 4 * 1024 * 1024)
        struct Reply: Decodable { let schema: String; let observations: [CompanionPosition] }
        let result = try decoder.decode(Reply.self, from: data)
        guard result.schema == "booknook.audible-positions.v1", result.observations.allSatisfy({
            asins.contains($0.asin) && $0.sourceUpdatedAt > 0 && $0.sourceUpdatedAt <= 9_007_199_254_740_991 && $0.fetchedAt <= 9_007_199_254_740_991 && $0.sourceUpdatedAt <= $0.fetchedAt + 300 &&
            $0.fetchedAt <= UInt64(Date().timeIntervalSince1970) + 300 && $0.alignment == "unmatched" &&
            ($0.durationMilliseconds == nil || $0.positionMilliseconds <= $0.durationMilliseconds!) && ($0.metadata?.isValid != false)
        }) else { throw AuthorizationError.message("Position evidence is invalid or its time/alignment is unsupported.") }
        return result.observations
    }
}
