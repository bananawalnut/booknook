// SPDX-License-Identifier: MPL-2.0
import Foundation
import Combine

struct CastaliaProfileCandidate: Identifiable {
    let id = UUID()
    let filename: String
    let bytes: Data
    let preview: CastaliaProfilePreview
    let configuredHolder: String?
    let unlockedHolder: String?
    let verifiedRecoveryHolder: String?
    let previousAuthority: CastaliaConfiguration?

    static let maximumBytes = 8_192
    static func readFile(_ url: URL) async throws -> Data {
        let task = Task.detached(priority: .userInitiated) {
            try Task.checkCancellation()
            let access = url.startAccessingSecurityScopedResource()
            defer { if access { url.stopAccessingSecurityScopedResource() } }
            let file = try FileHandle(forReadingFrom: url)
            defer { try? file.close() }
            let values = try url.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey])
            guard values.isRegularFile == true, (values.fileSize ?? maximumBytes + 1) <= maximumBytes else {
                throw ConnectionInputError.message("Choose a public Castalia profile file no larger than 8 KiB.")
            }
            let bytes = try file.read(upToCount: maximumBytes + 1) ?? Data()
            try Task.checkCancellation()
            guard !bytes.isEmpty, bytes.count <= maximumBytes, String(data: bytes, encoding: .utf8) != nil else {
                throw ConnectionInputError.message("Choose a UTF-8 public Castalia profile no larger than 8 KiB.")
            }
            return bytes
        }
        return try await withTaskCancellationHandler { try await task.value } onCancel: { task.cancel() }
    }
}

/// This is the same explicit preview/install boundary used by the native review sheet.
/// A successful install is reported even if the caller is cancelled after its commit.
@MainActor final class CastaliaProfileImportSession: ObservableObject {
    typealias Install = @MainActor (Data, CastaliaProfilePreview, String) async throws -> ConnectionConfiguration
    let candidate: CastaliaProfileCandidate
    @Published var holderPublicKey: String
    @Published var recognizesProvider = false
    @Published private(set) var installing = false
    private var cancelled = false
    private var installed = false
    private let install: Install

    init(candidate: CastaliaProfileCandidate, install: @escaping Install) {
        self.candidate = candidate; self.install = install
        holderPublicKey = candidate.configuredHolder ?? candidate.verifiedRecoveryHolder ?? candidate.unlockedHolder ?? ""
    }
    var validHolder: Bool {
        holderPublicKey.utf8.count == 64 && holderPublicKey.utf8.allSatisfy { (48...57).contains($0) || (97...102).contains($0) }
    }
    var canInstall: Bool { recognizesProvider && validHolder && !installing && !cancelled && !installed }
    func cancel() { if !installing { cancelled = true } }
    func apply() async throws -> ConnectionConfiguration {
        guard canInstall else { throw ConnectionInputError.message("Review the provider and choose your existing identity’s lowercase public key before continuing.") }
        try Task.checkCancellation()
        let holder = holderPublicKey
        installing = true
        defer { installing = false }
        let result = try await install(candidate.bytes, candidate.preview, holder)
        installed = true
        return result
    }
}
