// SPDX-License-Identifier: MIT OR Apache-2.0
// Compiled by an independent consumer of the installed CastaliaNative product.
// Fixture signatures and recovery bytes are deliberately public synthetic data.
import Foundation
#if canImport(CastaliaNative)
import CastaliaNative
#endif

private enum CastaliaSmokeFailure: Error { case assertion(String), syntheticWriteFailure }
private func castaliaSmokeRequire(_ condition: Bool, _ message: String) throws {
    if !condition { throw CastaliaSmokeFailure.assertion(message) }
}
private final class CastaliaSmokeVault {
    var values: [String: [String: Data]] = [:]
}
private final class CastaliaSmokeStorage: CastaliaSecureMaterialStorage {
    let vault: CastaliaSmokeVault
    let namespace: String
    var failWrites = false
    var failReads = false
    init(_ vault: CastaliaSmokeVault, namespace: String) { self.vault = vault; self.namespace = namespace }
    func read(account: String) throws -> Data? {
        if failReads { throw CastaliaSmokeFailure.syntheticWriteFailure }
        return vault.values[namespace]?[account]
    }
    func saveEncryptedMaterial(_ data: Data, account: String) throws {
        if failWrites { throw CastaliaSmokeFailure.syntheticWriteFailure }
        vault.values[namespace, default: [:]][account] = data
    }
}
@MainActor private final class CastaliaSmokePolicy: CastaliaCurrentTrustPolicySource {
    var value: CastaliaPolicySnapshot
    var onHolder: (() async throws -> Void)?
    init(_ value: CastaliaPolicySnapshot) { self.value = value }
    func configuredHolder() async throws -> String? { try await onHolder?(); return value.trust.holderPublicKey }
    func currentPolicy() async throws -> CastaliaPolicySnapshot { value }
}
@MainActor private final class CastaliaSmokeSource: CastaliaAuthorityRequestSource {
    let bytes: Data
    var calls = 0
    var beforeReturn: (() async throws -> Void)?
    init(_ bytes: Data) { self.bytes = bytes }
    func request(for intent: CastaliaRequestIntent, policy: CastaliaPolicySnapshot) async throws -> Data {
        calls += 1
        try await beforeReturn?()
        return bytes
    }
}
@MainActor private final class CastaliaSmokePresenter: CastaliaConsentPresenter {
    var calls = 0
    var approve = true
    var duringReview: (() async throws -> Void)?
    var last: CastaliaConsent?
    func requestConsent(_ consent: CastaliaConsent) async throws -> Bool {
        calls += 1; last = consent
        try await duringReview?()
        return approve
    }
    func cancelConsent(reason: String?) { }
}
@MainActor private func castaliaSmokeReject(_ body: () async throws -> Void) async throws {
    var rejected = false
    do { try await body() } catch { rejected = true }
    try castaliaSmokeRequire(rejected, "An invalid or cancelled authorization succeeded")
}

/// The host generates castalia-generic-fixture.json immediately before execution
/// with `cargo run -p booknook-native --example castalia-fixture`. Its actual issuer
/// signature is valid for 120 seconds. No application database or network is used.
@MainActor func runPackagedCastaliaSmoke(fixtureRoot: URL) async throws {
    let bytes = try Data(contentsOf: fixtureRoot.appendingPathComponent("castalia-generic-fixture.json"))
    guard let fixture = try JSONSerialization.jsonObject(with: bytes) as? [String: Any],
          let passphrase = fixture["passphrase"] as? String,
          let publicKey = fixture["public_key"] as? String else { throw CastaliaSmokeFailure.assertion("Missing synthetic fixture") }
    func data(_ key: String) throws -> Data {
        guard let value = fixture[key] else { throw CastaliaSmokeFailure.assertion("Missing fixture field") }
        return try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys, .withoutEscapingSlashes])
    }
    let trust = try JSONDecoder().decode(CastaliaTrustConfiguration.self, from: data("policy"))
    let originalPolicy = CastaliaPolicySnapshot(trust: trust, generation: "synthetic-initial-policy")
    let policy = CastaliaSmokePolicy(originalPolicy)
    let disclosure = try JSONDecoder().decode(CastaliaDisclosure.self, from: data("disclosure"))
    let source = CastaliaSmokeSource(try data("request"))
    let presenter = CastaliaSmokePresenter()
    let vault = CastaliaSmokeVault()
    let first = CastaliaSmokeStorage(vault, namespace: "synthetic.first-app")
    let second = CastaliaSmokeStorage(vault, namespace: "synthetic.second-app")
    let old = Data("original synthetic recovery marker".utf8)
    try first.saveEncryptedMaterial(old, account: "encrypted-recovery")
    let recovery = try data("recovery")
    try second.saveEncryptedMaterial(recovery, account: "encrypted-recovery")
    let session = CastaliaSession(storage: second, policy: policy, source: source, presenter: presenter)
    let intent = CastaliaRequestIntent(canonicalRequestJSON: try session.canonicalRequest(String(decoding: data("expected_request"), as: UTF8.self)), disclosure: disclosure)

    // Real encrypted recovery validation cannot modify live custody or either vault.
    let validated = try await session.validateRecovery(recovery, passphrase: passphrase)
    try castaliaSmokeRequire(validated == publicKey && session.unlockedPublicKey == nil, "Validation changed live custody")
    try castaliaSmokeRequire(try first.read(account: "encrypted-recovery") == old, "Other namespace changed")
    let candidate = try CastaliaRecoveryCandidate(data: recovery, filename: "synthetic.castalia-recovery", storage: second)
    let recoverySession = CastaliaRecoveryImportSession(candidate: candidate, storage: second, configuredHolder: { publicKey },
        validate: { data, pass in try await session.validateRecovery(data, passphrase: pass) })
    second.failWrites = true; recoverySession.passphrase = passphrase
    try await castaliaSmokeReject { _ = try await recoverySession.validateAndSave() }
    try castaliaSmokeRequire(try second.read(account: "encrypted-recovery") == recovery, "Failed replacement lost original bytes")
    try castaliaSmokeRequire(try first.read(account: "encrypted-recovery") == old, "Failed replacement crossed namespace")
    try castaliaSmokeRequire(recoverySession.passphrase.isEmpty, "Recovery passphrase was retained in the form")
    second.failWrites = false

    // Cancellation while the configured identity is being resolved cannot unlock later.
    policy.onHolder = { session.lock(); await Task.yield() }
    try await castaliaSmokeReject { _ = try await session.unlock(passphrase: passphrase) }
    try castaliaSmokeRequire(session.unlockedPublicKey == nil, "Late unlock revived cancelled identity")
    policy.onHolder = nil
    let unlocked = try await session.unlock(passphrase: passphrase)
    try castaliaSmokeRequire(unlocked == publicKey, "Recovery public key mismatch")
    second.failReads = true
    try await castaliaSmokeReject { _ = try await session.unlock(passphrase: passphrase) }
    second.failReads = false
    try castaliaSmokeRequire(session.unlockedPublicKey == nil, "Failed recovery read retained the previous identity")
    _ = try await session.unlock(passphrase: passphrase)
    let receipt = try await session.authorize(intent)
    try castaliaSmokeRequire(presenter.calls == 1 && presenter.last?.audience == "second-castalia-app", "Second app did not receive its own consent")
    try castaliaSmokeRequire(presenter.last?.title == disclosure.title && presenter.last?.statements == disclosure.statements, "Consent changed the expected disclosure")
    let exported: [String: Any] = ["request": try JSONSerialization.jsonObject(with: Data(receipt.requestJSON.utf8)),
        "presentation": try JSONSerialization.jsonObject(with: Data(receipt.presentationJSON.utf8))]
    let receiptURL = ProcessInfo.processInfo.environment["CASTALIA_SMOKE_RECEIPT_PATH"].map { URL(fileURLWithPath: $0) }
        ?? fixtureRoot.appendingPathComponent("castalia-swift-receipt.json")
    try JSONSerialization.data(withJSONObject: exported, options: [.sortedKeys]).write(to: receiptURL, options: .atomic)

    presenter.approve = false
    try await castaliaSmokeReject { _ = try await session.authorize(intent) }
    presenter.approve = true
    let reviewed = presenter.calls
    let wrongDisclosure = CastaliaRequestIntent(canonicalRequestJSON: intent.canonicalRequestJSON,
        disclosure: CastaliaDisclosure(title: "Changed by a second application", statements: disclosure.statements))
    try await castaliaSmokeReject { _ = try await session.authorize(wrongDisclosure) }
    try castaliaSmokeRequire(presenter.calls == reviewed, "Mismatched disclosure reached approval")

    // An issuer request await cannot install stale independently configured trust.
    source.beforeReturn = { policy.value = CastaliaPolicySnapshot(trust: trust, generation: "changed-during-request"); await Task.yield() }
    try await castaliaSmokeReject { _ = try await session.authorize(intent) }
    try castaliaSmokeRequire(presenter.calls == reviewed, "Stale request reached approval")
    source.beforeReturn = nil; policy.value = originalPolicy
    // The same applies to a policy change while the real consent UI would be open.
    presenter.duringReview = { policy.value = CastaliaPolicySnapshot(trust: trust, generation: "changed-during-consent"); await Task.yield() }
    try await castaliaSmokeReject { _ = try await session.authorize(intent) }
    presenter.duringReview = nil; policy.value = originalPolicy
    // Explicit cancellation and identity locking invalidate late successful responses.
    source.beforeReturn = { session.cancel(); await Task.yield() }
    try await castaliaSmokeReject { _ = try await session.authorize(intent) }
    source.beforeReturn = { session.lock(); await Task.yield() }
    try await castaliaSmokeReject { _ = try await session.authorize(intent) }
    source.beforeReturn = nil
    try castaliaSmokeRequire(session.unlockedPublicKey == nil && !session.busy, "Late response revived locked custody")
    try castaliaSmokeRequire(try first.read(account: "encrypted-recovery") == old, "Unrelated namespace changed")
    print("CastaliaNative consumer: real second-app receipt, disclosure binding, current-trust checks across awaits, denial/cancel/lock, isolated validation and namespaced failure-preserving recovery passed; no Desk, network or live Keychain access.")
}
