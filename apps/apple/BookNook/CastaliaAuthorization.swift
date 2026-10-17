// SPDX-License-Identifier: MPL-2.0
import SwiftUI
import CryptoKit
#if canImport(CastaliaNative)
import CastaliaNative
#endif
#if canImport(BookNookNative)
import BookNookNative
#endif

typealias AuthorizationDisclosure = CastaliaDisclosure
typealias AuthorizationReceipt = CastaliaReceipt
struct AuthorizationConsent: Identifiable {
    let token: String; let title: String; let statements: [String]; let expiresAt: UInt64
    let holderPublicKey: String; let action: String; let payloadDigest: String; let resourceVersion: UInt64
    var id: String { token }
}
enum AuthorizationError: LocalizedError {
    case message(String)
    var errorDescription: String? { if case .message(let text) = self { return text }; return nil }
}

/// App-specific issuer request envelope and independently saved trust. Generic
/// custody never sees a Desk, imports issuer pins, or grants a Book Nook action.
@MainActor private final class BookNookAuthoritySource: CastaliaCurrentTrustPolicySource, CastaliaAuthorityRequestSource {
    let repository: DeskRepository
    init(_ repository: DeskRepository) { self.repository = repository }
    func configuredHolder() async throws -> String? {
        let key = try await repository.connections().castalia?.holderPublicKey
        return key?.isEmpty == false ? key : nil
    }
    func currentPolicy() async throws -> CastaliaPolicySnapshot {
        try snapshot(try await configuration())
    }
    private func configuration() async throws -> CastaliaConfiguration {
        guard let value = try await repository.connections().castalia,
              !value.policyDigest.isEmpty, !value.credentialEndpoint.isEmpty else {
            throw AuthorizationError.message("Configure a trusted issuer, policy digest, and credential endpoint in Connections.")
        }
        return value
    }
    private func snapshot(_ value: CastaliaConfiguration) throws -> CastaliaPolicySnapshot {
        let caller = CastaliaCaller(kind: "terminal", id: value.terminalCaller)
        let trust = CastaliaTrustConfiguration(pins: [CastaliaTrustPin(issuer: value.issuer, keyID: value.keyId,
            publicKey: value.issuerPublicKey, audience: value.audience, callers: [caller])], expectedCaller: caller,
            audience: value.audience, holderPublicKey: value.holderPublicKey, policyDigest: value.policyDigest)
        let encoder = JSONEncoder(); encoder.outputFormatting = [.sortedKeys]
        let generation = SHA256.hash(data: try encoder.encode(value)).map { String(format: "%02x", $0) }.joined()
        return CastaliaPolicySnapshot(trust: trust, generation: generation)
    }
    func request(for intent: CastaliaRequestIntent, policy: CastaliaPolicySnapshot) async throws -> Data {
        let config = try await configuration()
        guard try snapshot(config) == policy else { throw AuthorizationError.message("Authority settings changed. Review a fresh request.") }
        let object: [String: Any] = ["schema": "booknook.credential-request.v1",
            "action_request": try JSONSerialization.jsonObject(with: Data(intent.canonicalRequestJSON.utf8)),
            "disclosure": ["title": intent.disclosure.title, "statements": intent.disclosure.statements],
            "holder_public_key": config.holderPublicKey, "audience": config.audience,
            "caller": ["kind": "terminal", "id": config.terminalCaller], "policy_digest": config.policyDigest]
        let endpoint = try configuredServiceURL(config.credentialEndpoint)
        var req = URLRequest(url: endpoint); req.httpMethod = "POST"; req.timeoutInterval = 25
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.httpBody = try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        if let token = try CustodySecureStorage().read(account: issuerAccessAccount(config.credentialEndpoint)), !token.isEmpty {
            let value = String(decoding: token, as: UTF8.self)
            guard !value.contains("\r"), !value.contains("\n") else { throw AuthorizationError.message("Invalid saved issuer access token.") }
            req.setValue("Bearer \(value)", forHTTPHeaderField: "Authorization")
        }
        return try await ConnectorHTTP.send(req, limit: 262_144)
    }
}

/// App sheet adapter only. Signature preparation, current-trust checks and
/// cancellation live in the same public CastaliaSession other apps consume.
@MainActor private final class BookNookConsentPresenter: CastaliaConsentPresenter {
    private var continuation: CheckedContinuation<Bool, Error>?
    let show: (CastaliaConsent) throws -> Void
    let hide: () -> Void
    init(show: @escaping (CastaliaConsent) throws -> Void, hide: @escaping () -> Void) { self.show = show; self.hide = hide }
    func requestConsent(_ consent: CastaliaConsent) async throws -> Bool {
        guard continuation == nil else { throw AuthorizationError.message("Finish the current consent first.") }
        try Task.checkCancellation()
        return try await withCheckedThrowingContinuation { result in
            do { try show(consent); continuation = result }
            catch { result.resume(throwing: error) }
        }
    }
    func approve() { let result = continuation; continuation = nil; hide(); result?.resume(returning: true) }
    func cancelConsent(reason: String?) {
        let result = continuation; continuation = nil; hide()
        if let reason { result?.resume(throwing: AuthorizationError.message(reason)) }
        else { result?.resume(throwing: CancellationError()) }
    }
}
@MainActor final class AuthorizationCoordinator: ObservableObject {
    @Published private(set) var unlockedPublicKey: String?
    @Published private(set) var pending: AuthorizationConsent?
    @Published private(set) var busy = false
    @Published var error: String?
    @Published var syncDraft = SyncPairingDraft()
    private let repository: DeskRepository
    private var lockTask: Task<Void, Never>?
    private var operationGeneration = UUID()
    private var activeOperation: (action: String, payloadDigest: String, version: UInt64)?
    private lazy var source = BookNookAuthoritySource(repository)
    private lazy var presenter = BookNookConsentPresenter(show: { [weak self] consent in
        guard let self, let operation = self.activeOperation else { throw CancellationError() }
        self.pending = AuthorizationConsent(token: consent.token, title: consent.title, statements: consent.statements,
            expiresAt: consent.expiresAt, holderPublicKey: consent.holderPublicKey, action: operation.action,
            payloadDigest: operation.payloadDigest, resourceVersion: operation.version)
    }, hide: { [weak self] in self?.pending = nil })
    private lazy var session = CastaliaSession(storage: CustodySecureStorage(), policy: source, source: source, presenter: presenter)
    init(repository: DeskRepository) { self.repository = repository }
    func unlock(passphrase: String) async throws -> String {
        guard !busy, pending == nil else { throw AuthorizationError.message("Finish or cancel the current approval first.") }
        lockTask?.cancel(); lockTask = nil; unlockedPublicKey = nil
        operationGeneration = UUID(); let attempt = operationGeneration
        let key = try await session.unlock(passphrase: passphrase)
        try Task.checkCancellation()
        guard operationGeneration == attempt, session.unlockedPublicKey == key else { throw CancellationError() }
        unlockedPublicKey = key; resetIdleLock(); return key
    }
    func lock() async {
        operationGeneration = UUID(); unlockedPublicKey = nil
        session.lock(); lockTask?.cancel(); lockTask = nil
    }
    private func resetIdleLock() {
        lockTask?.cancel()
        lockTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(300)); guard !Task.isCancelled else { return }
            await self?.lock()
        }
    }
    func authorize(payload: Data, action: String, resourceVersion: UInt64, disclosure: AuthorizationDisclosure) async throws -> AuthorizationReceipt {
        guard !busy, pending == nil else { throw AuthorizationError.message("Finish or cancel the current approval first.") }
        guard unlockedPublicKey != nil else { throw AuthorizationError.message("Unlock your Castalia identity in Connections first.") }
        busy = true; operationGeneration = UUID(); let attempt = operationGeneration
        defer { busy = false; activeOperation = nil }
        let canonical = try await repository.canonicalPayload(payload)
        try Task.checkCancellation(); guard operationGeneration == attempt else { throw CancellationError() }
        // This is still the native Book Nook action builder with its action allowlist,
        // holder, payload digest and resource-version binding. The receiver repeats it.
        let actionJSON = try await repository.authorizedAction(payload: Data(canonical.utf8), action: action,
            resourceVersion: resourceVersion, idempotencyKey: UUID().uuidString.lowercased())
        try Task.checkCancellation(); guard operationGeneration == attempt else { throw CancellationError() }
        activeOperation = (action, SHA256.hash(data: Data(canonical.utf8)).map { String(format: "%02x", $0) }.joined(), resourceVersion)
        do {
            let result = try await session.authorize(CastaliaRequestIntent(canonicalRequestJSON: actionJSON, disclosure: disclosure))
            resetIdleLock(); return result
        } catch {
            if !bookNookOperationWasCancelled(error) { self.error = bookNookErrorMessage(error) }
            throw error
        }
    }
    func approve() async { presenter.approve() }
    func cancel(reason: String? = nil) { operationGeneration = UUID(); session.cancel(reason: reason) }
}

extension DeskRepository {
    func validateRecovery(_ recovery: Data, passphrase: String) throws -> String {
        try CastaliaCustody().validateRecovery(recoveryJson: String(decoding: recovery, as: UTF8.self), passphrase: passphrase)
    }
}

struct RecoveryImportReview: View {
    @Environment(\.dismiss) private var dismiss
    @StateObject private var session: RecoveryImportSession
    @State private var message: String?
    let saved: @MainActor (String) async -> Void

    @MainActor init(candidate: RecoveryImportCandidate, repository: DeskRepository, saved: @escaping @MainActor (String) async -> Void) {
        _session = StateObject(wrappedValue: RecoveryImportSession(candidate: candidate, storage: CustodySecureStorage(), configuredHolder: {
            let key = try await repository.connections().castalia?.holderPublicKey
            return key?.isEmpty == false ? key : nil
        }, validate: { recovery, passphrase in try await repository.validateRecovery(recovery, passphrase: passphrase) }))
        self.saved = saved
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text(session.candidate.replacesSavedFile ? "Replace saved recovery file?" : "Save your recovery file").font(.title2.bold())
            Text(session.candidate.filename).font(.callout).textSelection(.enabled)
            Text("Enter this file’s passphrase to check it on this device. It must match the identity in your saved Connections settings.").foregroundStyle(.secondary)
            if session.candidate.replacesSavedFile {
                Text("Your existing recovery file stays in Keychain unless validation and replacement both succeed. Keep the original file somewhere safe; Book Nook keeps one recovery file on this device.")
            }
            Text("After saving, your identity will be locked. Unlock it in Connections when you are ready to approve an action.").font(.caption).foregroundStyle(.secondary)
            SecureField("Recovery passphrase", text: $session.passphrase).textFieldStyle(.roundedBorder).disabled(session.validating)
            if let message { Text(message).foregroundStyle(.red).font(.callout) }
            if session.validating { ProgressView("Checking the recovery file…") }
            HStack {
                Button("Cancel", role: .cancel) { session.cancel(); dismiss() }
                Spacer()
                Button(session.candidate.replacesSavedFile ? "Validate and replace" : "Validate and save", role: session.candidate.replacesSavedFile ? .destructive : nil) {
                    Task {
                        do { let key = try await session.validateAndSave(); await saved(key); dismiss() }
                        catch is CancellationError { }
                        catch { message = "\(bookNookErrorMessage(error)) Your saved recovery file has not changed. Check the file and passphrase before trying again." }
                    }
                }.buttonStyle(.borderedProminent).disabled(session.validating || session.passphrase.isEmpty)
            }
        }.padding(28).frame(minWidth: 360, idealWidth: 560)
        .interactiveDismissDisabled(session.validating)
        .onDisappear { session.cancel() }
    }
}

struct CustodyLockStatus: View {
    @ObservedObject var coordinator: AuthorizationCoordinator
    var body: some View {
        HStack {
            Label(coordinator.unlockedPublicKey == nil ? "Identity locked" : "Identity unlocked on this device", systemImage: coordinator.unlockedPublicKey == nil ? "lock" : "lock.open")
                .font(.caption).foregroundStyle(.secondary)
            if coordinator.unlockedPublicKey != nil { Button("Lock now") { Task { await coordinator.lock() } }.font(.caption) }
        }
    }
}

struct CastaliaConsentModifier: ViewModifier {
    @ObservedObject var coordinator: AuthorizationCoordinator
    @Environment(\.scenePhase) private var scenePhase
    func body(content: Content) -> some View {
        content.sheet(item: Binding(get: { coordinator.pending }, set: { if $0 == nil { coordinator.cancel() } })) { consent in
            VStack(alignment: .leading, spacing: 18) {
                Text(consent.title).font(.title2.bold())
                Text("Approve this action with Castalia").foregroundStyle(.secondary)
                ForEach(Array(consent.statements.enumerated()), id: \.offset) { _, statement in Text(statement).fixedSize(horizontal: false, vertical: true) }
                Divider()
                LabeledContent("Action", value: consent.action)
                LabeledContent("Resource version", value: String(consent.resourceVersion))
                Text("Identity: \(consent.holderPublicKey)").font(.caption.monospaced()).textSelection(.enabled)
                Text("Payload SHA-256: \(consent.payloadDigest)").font(.caption.monospaced()).textSelection(.enabled)
                Text("Expires \(Date(timeIntervalSince1970: TimeInterval(consent.expiresAt)).formatted(date: .omitted, time: .standard))").font(.caption).foregroundStyle(.secondary)
                HStack { Button("Cancel", role: .cancel) { coordinator.cancel() }; Spacer(); Button("Approve once") { Task { await coordinator.approve() } }.buttonStyle(.borderedProminent) }
            }.padding(28).frame(minWidth: 360, idealWidth: 560)
            .interactiveDismissDisabled()
        }
        .onChange(of: scenePhase) { _, phase in if phase == .background { Task { await coordinator.lock() } } }
    }
}
extension View { func castaliaConsent(_ coordinator: AuthorizationCoordinator) -> some View { modifier(CastaliaConsentModifier(coordinator: coordinator)) } }

func issuerAccessAccount(_ endpoint: String) -> String {
    "castalia-issuer:" + SHA256.hash(data: Data(endpoint.utf8)).map { String(format: "%02x", $0) }.joined()
}
/// No redirects, cookies, or credential forwarding; limits streamed response bytes before allocation.
enum ConnectorHTTP {
    static func send(_ request: URLRequest, limit: Int, localOnly: Bool = false) async throws -> Data {
        let configuration = URLSessionConfiguration.ephemeral
        if localOnly {
            guard let endpoint = request.url else { throw AuthorizationError.message("Missing local companion URL.") }
            try CompanionKindleImport.configureLocalTransport(configuration, endpoint: endpoint)
        }
        configuration.timeoutIntervalForRequest = 25; configuration.timeoutIntervalForResource = 30
        let session = URLSession(configuration: configuration, delegate: ConnectorNoRedirects(), delegateQueue: nil)
        defer { session.invalidateAndCancel() }
        let (stream, response) = try await session.bytes(for: request)
        let code = (response as? HTTPURLResponse)?.statusCode ?? 0
        let successful = (200...299).contains(code)
        let byteLimit = successful ? limit : min(limit, 16_384)
        guard response.expectedContentLength <= byteLimit else {
            if !successful { throw AuthorizationError.message(ConnectorFailure.message(status: code, body: nil)) }
            throw AuthorizationError.message("Service response exceeds the size limit.")
        }
        var data = Data()
        for try await byte in stream {
            guard data.count < byteLimit else {
                if !successful { throw AuthorizationError.message(ConnectorFailure.message(status: code, body: nil)) }
                throw AuthorizationError.message("Service response exceeds the size limit.")
            }
            data.append(byte)
        }
        guard successful else { throw AuthorizationError.message(ConnectorFailure.message(status: code, body: data)) }
        return data
    }
}
private final class ConnectorNoRedirects: NSObject, URLSessionTaskDelegate {
    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) { completionHandler(nil) }
}
