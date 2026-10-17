// SPDX-License-Identifier: MIT OR Apache-2.0
import Foundation
import Combine

public struct CastaliaRecoveryCandidate: Identifiable {
    public static let account = "encrypted-recovery"
    public static let maximumBytes = 1_048_576
    public let id = UUID()
    public let filename: String
    public let data: Data
    public let previous: Data?
    public let accountName: String
    public var replacesSavedFile: Bool { previous != nil }

    public init(data: Data, filename: String, storage: any CastaliaSecureMaterialStorage, account: String = Self.account) throws {
        guard !data.isEmpty, data.count <= Self.maximumBytes,
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              object["schema"] as? String == "castalia.wallet-custody.v1",
              object["ciphertext"] is String else {
            throw CastaliaRecoveryError.message("Choose an encrypted Castalia recovery file, at most 1 MiB. Your saved recovery file has not changed.")
        }
        self.filename = filename
        self.data = data
        accountName = account
        previous = try storage.read(account: account)
    }

    public static func readFile(_ url: URL, storage: any CastaliaSecureMaterialStorage, account: String = Self.account) throws -> Self {
        let file = try FileHandle(forReadingFrom: url)
        defer { try? file.close() }
        let data = try file.read(upToCount: maximumBytes + 1) ?? Data()
        return try Self(data: data, filename: url.lastPathComponent, storage: storage, account: account)
    }
}

public enum CastaliaRecoveryError: LocalizedError {
    case message(String)
    public var errorDescription: String? { if case .message(let text) = self { return text }; return nil }
}

/// Candidate bytes stay in memory until isolated native validation and explicit user intent
/// succeed. No secret or staging file is written outside the secure-store commit.
@MainActor public final class CastaliaRecoveryImportSession: ObservableObject {
    public let candidate: CastaliaRecoveryCandidate
    @Published public var passphrase = ""
    @Published public private(set) var validating = false
    private let storage: any CastaliaSecureMaterialStorage
    private let validate: (Data, String) async throws -> String
    private let configuredHolder: () async throws -> String?
    private var generation = UUID()
    private var cancelled = false

    public init(candidate: CastaliaRecoveryCandidate, storage: any CastaliaSecureMaterialStorage,
         configuredHolder: @escaping () async throws -> String?,
         validate: @escaping (Data, String) async throws -> String) {
        self.candidate = candidate; self.storage = storage
        self.configuredHolder = configuredHolder; self.validate = validate
    }

    public func cancel() {
        cancelled = true; generation = UUID(); passphrase = ""; validating = false
    }

    public func validateAndSave() async throws -> String {
        guard !cancelled, !validating else { throw CancellationError() }
        guard !passphrase.isEmpty else { throw CastaliaRecoveryError.message("Enter this file’s recovery passphrase.") }
        let attempt = generation
        var secret = passphrase
        passphrase = ""; validating = true
        defer { secret = ""; if generation == attempt { validating = false } }
        let expected = try await configuredHolder()
        let publicKey = try await validate(candidate.data, secret)
        try Task.checkCancellation()
        guard generation == attempt, !cancelled else { throw CancellationError() }
        let current = try await configuredHolder()
        try Task.checkCancellation()
        guard current == expected else {
            throw CastaliaRecoveryError.message("Your identity settings changed while checking the file. Review them and try again; your saved recovery file has not changed.")
        }
        guard expected == nil || expected == publicKey else {
            throw CastaliaRecoveryError.message("This file belongs to a different identity from your saved Connections settings. Your existing recovery file has not changed.")
        }
        guard generation == attempt, !cancelled else { throw CancellationError() }
        guard try storage.read(account: candidate.accountName) == candidate.previous else {
            throw CastaliaRecoveryError.message("The saved recovery file changed in another window. Choose the file again before replacing it.")
        }
        try storage.saveEncryptedMaterial(candidate.data, account: candidate.accountName)
        cancelled = true; generation = UUID(); validating = false
        return publicKey
    }
}

