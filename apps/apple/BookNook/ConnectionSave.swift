// SPDX-License-Identifier: MPL-2.0
import Foundation

enum ConnectionInputError: LocalizedError {
    case message(String)
    var errorDescription: String? { if case .message(let text) = self { return text }; return nil }
}

/// Unsaved access belongs to the exact destination present when entry began.
/// Keep this binding independent of SwiftUI's deferred change notifications.
struct IssuerAccessDraft: Equatable {
    private(set) var value = ""
    private(set) var endpoint: String?

    mutating func update(_ value: String, endpoint: String) {
        if value.isEmpty { clear(); return }
        if self.value.isEmpty { self.endpoint = endpoint }
        self.value = value
    }

    mutating func clear() { value = ""; endpoint = nil }

    @discardableResult mutating func clearIfDestinationChanged(to endpoint: String) -> Bool {
        guard !value.isEmpty, self.endpoint != endpoint else { return false }
        clear()
        return true
    }

    func token(for endpoint: String) throws -> String {
        guard value.isEmpty || self.endpoint == endpoint else {
            throw ConnectionInputError.message("The provider endpoint changed after this access token was entered. Clear the unsaved token and enter access for the new destination before saving.")
        }
        return value
    }
}

/// Validate every optional access input before any settings or secure-store write.
struct ConnectionSaveInputs {
    let audibleAccess: Data?
    let issuerAccess: Data?
    let issuerEndpoint: String

    init(audibleHeaders: String, issuerToken: String, issuerEndpoint: String) throws {
        if audibleHeaders.isEmpty { audibleAccess = nil }
        else {
            let data = Data(audibleHeaders.utf8)
            _ = try Self.parseAudibleHeaders(data)
            audibleAccess = data
        }
        if issuerToken.isEmpty { issuerAccess = nil }
        else {
            guard issuerToken.utf8.count <= 16_384, !issuerToken.contains("\r"), !issuerToken.contains("\n") else {
                throw ConnectionInputError.message("The issuer access token is too long or contains a line break. Check the token before saving.")
            }
            _ = try configuredServiceURL(issuerEndpoint)
            issuerAccess = Data(issuerToken.utf8)
        }
        self.issuerEndpoint = issuerEndpoint
    }

    static func parseAudibleHeaders(_ bytes: Data) throws -> [String: String] {
        guard bytes.count < 32_768,
              let headers = try? JSONSerialization.jsonObject(with: bytes) as? [String: String], !headers.isEmpty else {
            throw ConnectionInputError.message("Supply a JSON object containing account request headers, under 32 KiB.")
        }
        let allowed = Set(["authorization", "x-adp-token", "x-adp-alg", "x-adp-signature", "x-amz-access-token", "client-id"])
        guard headers.allSatisfy({ allowed.contains($0.key.lowercased()) && !$0.value.contains("\r") && !$0.value.contains("\n") }) else {
            throw ConnectionInputError.message("Only account authorization headers without line breaks are accepted.")
        }
        return headers
    }
}

struct ConnectionSaveResult {
    var audibleAccessSaved = false
    var issuerAccessSaved = false
    var failedAccess: String?
    var error: Error?
}

/// Settings and Keychain are separate stores. Return an explicit partial result,
/// so only successfully saved inputs are cleared from the form.
@MainActor func saveConnectionInputs(
    _ inputs: ConnectionSaveInputs,
    saveSettings: () async throws -> Void,
    saveAudible: (Data) throws -> Void,
    saveIssuer: (Data, String) throws -> Void
) async throws -> ConnectionSaveResult {
    try await saveSettings()
    var result = ConnectionSaveResult()
    if let data = inputs.audibleAccess {
        do { try saveAudible(data); result.audibleAccessSaved = true }
        catch { result.failedAccess = "Audible access"; result.error = error; return result }
    }
    if let data = inputs.issuerAccess {
        do { try saveIssuer(data, inputs.issuerEndpoint); result.issuerAccessSaved = true }
        catch { result.failedAccess = "Issuer access"; result.error = error }
    }
    return result
}

func configuredServiceURL(_ string: String) throws -> URL {
    guard let url = URL(string: string), let host = url.host,
          url.scheme == "https" || (url.scheme == "http" && ["localhost", "127.0.0.1", "[::1]", "::1"].contains(host)),
          url.user == nil, url.password == nil, url.fragment == nil, url.query == nil else {
        throw ConnectionInputError.message("Use an HTTPS service URL without credentials, query, or fragment. HTTP is allowed only on loopback.")
    }
    return url
}
