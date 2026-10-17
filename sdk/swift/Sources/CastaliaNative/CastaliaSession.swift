// SPDX-License-Identifier: MIT OR Apache-2.0
import Foundation
#if canImport(BookNookNative)
import BookNookNative
#endif

/// Generic native authorization coordination, independent of Desk/SQLite. Rust
/// validates exact request/disclosure and signs only a prepared Castalia v2 transcript.
/// The receiver must still validate its application's operation and consume replays.
@MainActor public final class CastaliaSession {
    public private(set) var unlockedPublicKey: String?
    public private(set) var busy = false
    private let custody: CastaliaCustody
    private let storage: any CastaliaSecureMaterialStorage
    private let policy: any CastaliaCurrentTrustPolicySource
    private let source: any CastaliaAuthorityRequestSource
    private let presenter: any CastaliaConsentPresenter
    private let recoveryAccount: String
    private var generation = UUID()
    private var unlocking = false
    private var pendingToken: String?
    private var expiryTask: Task<Void, Never>?
    private var cancellationReason: String?

    public init(storage: any CastaliaSecureMaterialStorage, policy: any CastaliaCurrentTrustPolicySource,
                source: any CastaliaAuthorityRequestSource, presenter: any CastaliaConsentPresenter,
                recoveryAccount: String = "encrypted-recovery", custody: CastaliaCustody = CastaliaCustody()) {
        self.storage = storage; self.policy = policy; self.source = source; self.presenter = presenter
        self.recoveryAccount = recoveryAccount; self.custody = custody
    }
    public func canonicalRequest(_ json: String) throws -> String { try custody.canonicalRequest(requestJson: json) }
    public func validateRecovery(_ data: Data, passphrase: String) async throws -> String {
        let custody = self.custody
        return try await Task.detached(priority: .userInitiated) {
            try custody.validateRecovery(recoveryJson: String(decoding: data, as: UTF8.self), passphrase: passphrase)
        }.value
    }
    public func unlock(passphrase: String) async throws -> String {
        guard !busy, !unlocking else { throw CastaliaNativeError.message("Finish or cancel the current approval or unlock first.") }
        unlocking = true; generation = UUID(); let attempt = generation
        unlockedPublicKey = nil; cancellationReason = nil
        defer { unlocking = false }
        let custody = self.custody
        do {
            // A failed attempt, including a secure-store read failure, must not
            // retain the previous unlocked identity. Isolated validation is separate.
            try custody.lock()
            guard let recovery = try storage.read(account: recoveryAccount) else { throw CastaliaNativeError.message("Import your Castalia recovery file first.") }
            let expected = try await policy.configuredHolder()
            try check(attempt)
            let key = try await Task.detached(priority: .userInitiated) {
                try custody.unlock(recoveryJson: String(decoding: recovery, as: UTF8.self), passphrase: passphrase)
            }.value
            try check(attempt)
            let current = try await policy.configuredHolder()
            try check(attempt)
            guard current == expected, expected == nil || expected == key else {
                throw CastaliaNativeError.message("This recovery file does not match the current identity settings. Your identity remains locked.")
            }
            unlockedPublicKey = key
            return key
        } catch { try? custody.lock(); throw error }
    }
    public func lock() {
        unlockedPublicKey = nil
        cancel()
        try? custody.lock()
    }
    public func cancel(reason: String? = nil) {
        generation = UUID(); cancellationReason = reason
        expiryTask?.cancel(); expiryTask = nil
        if let token = pendingToken { try? custody.cancel(token: token) }
        pendingToken = nil
        presenter.cancelConsent(reason: reason)
    }
    /// Notify the session immediately when privileged configuration is replaced.
    /// This also invalidates an in-flight request if settings later return to old values.
    public func policyDidChange() { cancel(reason: "Authority settings changed. Review a fresh request.") }
    public func authorize(_ intent: CastaliaRequestIntent) async throws -> CastaliaReceipt {
        guard !busy, !unlocking else { throw CastaliaNativeError.message("Finish or cancel the current approval first.") }
        guard let holder = unlockedPublicKey else { throw CastaliaNativeError.message("Unlock your Castalia identity first.") }
        busy = true; cancellationReason = nil; generation = UUID()
        let attempt = generation
        defer {
            expiryTask?.cancel(); expiryTask = nil
            if let token = pendingToken { try? custody.cancel(token: token) }
            pendingToken = nil; busy = false
        }
        return try await withTaskCancellationHandler {
            let initial = try await policy.currentPolicy()
            try check(attempt)
            guard initial.trust.holderPublicKey == holder, try custody.publicKey() == holder else {
                throw CastaliaNativeError.message("The unlocked identity does not match the configured holder.")
            }
            let canonical = try custody.canonicalRequest(requestJson: intent.canonicalRequestJSON)
            let expected = CastaliaRequestIntent(canonicalRequestJSON: canonical, disclosure: intent.disclosure)
            let data = try await source.request(for: expected, policy: initial)
            try check(attempt)
            guard data.count <= 262_144, let request = String(data: data, encoding: .utf8) else { throw CastaliaNativeError.message("Invalid or oversized issuer response.") }
            try await requireCurrent(initial, attempt: attempt, holder: holder)
            let prepared = try custody.prepare(requestJson: request, expectedRequestJson: canonical,
                expectedDisclosureJson: castaliaJSON(intent.disclosure), policyJson: castaliaJSON(initial.trust))
            let consent = try JSONDecoder().decode(CastaliaConsent.self, from: Data(prepared.utf8))
            pendingToken = consent.token
            expiryTask = Task { [weak self] in
                let duration = max(0, TimeInterval(consent.expiresAt) - Date().timeIntervalSince1970)
                try? await Task.sleep(for: .seconds(duration))
                guard !Task.isCancelled else { return }
                self?.cancel(reason: "This approval expired. Review a fresh request.")
            }
            guard try await presenter.requestConsent(consent) else { throw CancellationError() }
            try await requireCurrent(initial, attempt: attempt, holder: holder)
            let presentation = try custody.approve(token: consent.token, currentPolicyJson: castaliaJSON(initial.trust))
            pendingToken = nil
            return CastaliaReceipt(requestJSON: request, presentationJSON: presentation)
        } onCancel: { Task { @MainActor in if self.generation == attempt { self.cancel() } } }
    }
    private func check(_ attempt: UUID) throws {
        try Task.checkCancellation()
        guard generation == attempt else {
            if let cancellationReason { throw CastaliaNativeError.message(cancellationReason) }
            throw CancellationError()
        }
    }
    private func requireCurrent(_ expected: CastaliaPolicySnapshot, attempt: UUID, holder: String) async throws {
        try check(attempt)
        let current = try await policy.currentPolicy()
        try check(attempt)
        guard current == expected, unlockedPublicKey == holder, try custody.publicKey() == holder else {
            throw CastaliaNativeError.message("Authority or identity changed. Review a fresh request.")
        }
    }
}
