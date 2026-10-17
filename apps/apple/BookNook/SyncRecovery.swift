// SPDX-License-Identifier: MPL-2.0
import SwiftUI
import UniformTypeIdentifiers

struct SyncPeer: Decodable, Identifiable {
    let deviceId: String; let label: String; let version: UInt64; let enabled: Bool
    var id: String { deviceId }
}
struct SyncConflict: Decodable, Identifiable {
    let id: String; let collection: String; let recordId: String; let field: String; let local: JSONValue; let incoming: JSONValue
}
struct SyncOutbox: Decodable, Identifiable { let id: String; let peer: String; let exported: Bool }
struct SyncStatus: Decodable { let deviceId: String; let peers: [SyncPeer]; let conflicts: [SyncConflict]; let outbox: [SyncOutbox] }
struct SyncPrepared: Decodable { let batch: JSONValue; let books: Int; let notebooks: Int; let cards: Int; let annotations: Int; let assets: Int; let bytes: Int }
private func syncDecode<T: Decodable>(_ type: T.Type, _ raw: String) throws -> T { let d = JSONDecoder(); d.keyDecodingStrategy = .convertFromSnakeCase; return try d.decode(type, from: Data(raw.utf8)) }
extension DeskRepository {
    func syncStatus() throws -> SyncStatus { try syncDecode(SyncStatus.self, required().syncStatus()) }
    func syncNewKey() throws -> String { try required().syncNewPairingKey() }
    func syncPreparePeer(id: String, label: String, key: String, enabled: Bool) throws -> String { try required().syncPreparePeer(deviceId: id, label: label, keyHex: key, enabled: enabled) }
    func syncApprovePeer(_ raw: String, receipt: AuthorizationReceipt) throws { try required().syncApprovePeer(peerJson: raw, requestJson: receipt.requestJSON, presentationJson: receipt.presentationJSON) }
    func syncPrepare(peer: String) throws -> SyncPrepared { try syncDecode(SyncPrepared.self, required().syncPrepare(peerId: peer)) }
    func syncApprove(id: String, key: String, receipt: AuthorizationReceipt) throws -> String { try required().syncApprove(batchId: id, keyHex: key, requestJson: receipt.requestJSON, presentationJson: receipt.presentationJSON) }
    func syncCancel(id: String) throws { try required().syncCancelPrepared(batchId: id) }
    func syncExport(id: String, url: URL) throws { try required().syncExport(batchId: id, path: url.path) }
    func syncRemove(id: String) throws { try required().syncRemoveOutbox(batchId: id) }
    func syncReceive(url: URL, peer: String, key: String) throws -> DeskSnapshot {
        let result = try required().syncReceive(path: url.path, sourceDevice: peer, keyHex: key)
        return try syncDecode(Response.self, result).state
    }
    func syncResolve(id: String, incoming: Bool) throws -> DeskSnapshot { try syncDecode(DeskSnapshot.self, required().syncResolve(conflictId: id, useIncoming: incoming)) }
}

struct SyncRecoveryView: View {
    @EnvironmentObject var model: DeskModel
    var body: some View { SyncRecoveryContent(authorization: model.authorization) }
}

private struct SyncRecoveryContent: View {
    @EnvironmentObject var model: DeskModel
    @ObservedObject var authorization: AuthorizationCoordinator
    @State private var status: SyncStatus?
    @State private var identityConfigured = false
    @State private var expectedHolder: String?
    @State private var recoveryStored = false
    @State private var revealPairingKey = false
    @State private var busy = false
    @State private var importing = false
    @State private var exporting = false
    @State private var document: ExportDocument?
    @State private var message: String?
    @State private var removing: SyncOutbox?
    private var deviceId: String { authorization.syncDraft.deviceID }
    private var label: String { authorization.syncDraft.label }
    private var pairingKey: String { authorization.syncDraft.secret }
    private var selectedPeer: String { authorization.syncDraft.selectedPeer }
    private var prerequisites: SyncPrerequisites {
        SyncPrerequisites(configured: identityConfigured, recoveryStored: recoveryStored, expectedHolder: expectedHolder, unlockedHolder: authorization.unlockedPublicKey)
    }
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                HStack(spacing: 12) { SidebarRevealButton(); Text("Keep your reading with you").font(.system(size: 34, design: .serif)) }
                Text("Local writing is saved automatically. Review an encrypted snapshot before transferring it to another device that uses your Castalia identity.").foregroundStyle(.secondary)
                VStack(alignment: .leading, spacing: 10) {
                    Text(prerequisites.canApprove ? "Ready to pair and approve transfers" : "Set up your identity before pairing").font(.headline)
                    prerequisite("Save your identity provider settings in Connections", complete: identityConfigured)
                    prerequisite("Save your encrypted recovery file in Connections", complete: recoveryStored)
                    prerequisite("Unlock the identity used by both devices", complete: prerequisites.identityUnlocked)
                    if !prerequisites.canApprove {
                        Text("Your library and notebook still work offline. Complete these steps in Connections, then return here. Entered pairing details stay in this app session.").font(.callout).foregroundStyle(.secondary)
                        Button("Open Connections") { model.section = "connections" }.buttonStyle(.borderedProminent)
                    }
                }.padding(18).background(.white.opacity(0.6), in: RoundedRectangle(cornerRadius: 12))
                if let status {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("This device’s pairing ID").font(.headline)
                        Text(status.deviceId).font(.caption.monospaced()).textSelection(.enabled)
                    }.padding(18).background(.white.opacity(0.6), in: RoundedRectangle(cornerRadius: 12))
                    DisclosureGroup("Pair another device") {
                        VStack(alignment: .leading, spacing: 12) {
                            Text("Open Sync & recovery on both devices. Copy each device’s ID to the other and enter the same secret pairing key on both. Approve the pair on each device. Transfer the key privately; anyone with it can decrypt snapshots for this pair.").font(.callout).foregroundStyle(.secondary)
                            TextField("Other device’s pairing ID", text: $authorization.syncDraft.deviceID).textFieldStyle(.roundedBorder)
                            TextField("Device name", text: $authorization.syncDraft.label).textFieldStyle(.roundedBorder)
                            SecureField("Shared pairing key", text: $authorization.syncDraft.secret).textFieldStyle(.roundedBorder)
                            HStack {
                                Button("Create pairing key") { run { authorization.syncDraft.secret = try await model.repository.syncNewKey(); revealPairingKey = true } }
                                Button(revealPairingKey ? "Hide key" : "Show key") { revealPairingKey.toggle() }.disabled(pairingKey.isEmpty)
                            }
                            if revealPairingKey { Text(pairingKey).font(.caption.monospaced()).textSelection(.enabled) }
                            Button("Review device pairing") { run { try await pair(id: deviceId, name: label, secret: pairingKey, enabled: true); authorization.syncDraft.secret = ""; revealPairingKey = false } }.disabled(deviceId.isEmpty || label.isEmpty || pairingKey.count != 64).buttonStyle(.borderedProminent)
                        }.padding(.top, 12).disabled(!prerequisites.canApprove)
                    }
                    ForEach(status.peers) { peer in
                        HStack {
                            VStack(alignment: .leading) { Text(peer.label).font(.headline); Text(peer.enabled ? "Paired · same Castalia reader" : "Disabled").font(.caption).foregroundStyle(.secondary) }
                            Spacer()
                            Button(peer.enabled ? "Disable" : "Enable") { run { try await pair(id: peer.id, name: peer.label, secret: savedKey(peer.id), enabled: !peer.enabled) } }.disabled(!prerequisites.canApprove)
                        }.padding(18).background(.white.opacity(0.6), in: RoundedRectangle(cornerRadius: 12))
                    }
                    if status.peers.contains(where: \.enabled) {
                        Divider()
                        Picker("Transfer with", selection: $authorization.syncDraft.selectedPeer) { Text("Choose device").tag(""); ForEach(status.peers.filter(\.enabled)) { Text($0.label).tag($0.id) } }
                        HStack {
                            Button("Review encrypted snapshot") { run { try await prepare() } }.buttonStyle(.borderedProminent).disabled(!prerequisites.canApprove)
                            Button("Import encrypted snapshot") { importing = true }.buttonStyle(.bordered).disabled(!identityConfigured)
                        }.disabled(selectedPeer.isEmpty || model.saving)
                        Text("Each approved snapshot includes the library, notebooks, character history, annotations, reading positions, and accessible book files. Credentials and connection settings are excluded. File transfer needs no Book Nook server.").font(.caption).foregroundStyle(.secondary)
                    }
                    if !status.outbox.isEmpty {
                        Divider(); Text("Approved transfers").font(.title2)
                        ForEach(status.outbox) { batch in
                            HStack {
                                VStack(alignment: .leading) { Text(status.peers.first(where: { $0.id == batch.peer })?.label ?? batch.peer); Text(batch.exported ? "Exported · delivery not confirmed" : "Ready to export").font(.caption).foregroundStyle(.secondary) }
                                Spacer()
                                Button("Export file") { run { try await export(batch.id) } }
                                Button("Remove retry copy") { removing = batch }
                            }
                        }
                    }
                    if !status.conflicts.isEmpty {
                        Divider(); Text("Conflicting edits").font(.title2)
                        Text("Both interpretations are preserved until you choose. Character-card branches remain independently recoverable in card history.").font(.callout).foregroundStyle(.secondary)
                        ForEach(status.conflicts) { conflict in
                            VStack(alignment: .leading, spacing: 12) {
                                Text("\(conflict.collection.capitalized) · \(conflict.field.isEmpty ? conflict.recordId : conflict.field)").font(.headline)
                                Text("On this device at merge").font(.caption.bold()); Text(display(conflict.local)).textSelection(.enabled)
                                Text("From paired device").font(.caption.bold()); Text(display(conflict.incoming)).textSelection(.enabled)
                                HStack {
                                    Button("Keep this device’s current value") { resolve(conflict, incoming: false) }
                                    Button("Use incoming value") { resolve(conflict, incoming: true) }
                                }
                            }.padding(18).background(.white.opacity(0.6), in: RoundedRectangle(cornerRadius: 12))
                        }
                    }
                }
                if busy { ProgressView("Preparing your reviewed action…") }
                if let message { Text(message).font(.callout).foregroundStyle(.secondary) }
            }.padding(36).disabled(busy)
        }
        .task { await refresh() }
        .fileImporter(isPresented: $importing, allowedContentTypes: [.json]) { result in
            run {
                let url = try result.get(); let access = url.startAccessingSecurityScopedResource(); defer { if access { url.stopAccessingSecurityScopedResource() } }
                try await model.drainSaves()
                model.accept(try await model.repository.syncReceive(url: url, peer: selectedPeer, key: savedKey(selectedPeer)))
                model.reloadEditorSessions()
                message = "Snapshot received. Any conflicting edits are listed below."
            }
        }
        .fileExporter(isPresented: $exporting, document: document, contentType: .json, defaultFilename: "BookNook-encrypted-snapshot") { result in if case .failure(let error) = result, !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } }
        .confirmationDialog("Remove the encrypted retry copy?", isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } })) {
            if let batch = removing { Button("Remove retry copy", role: .destructive) { run { try await model.repository.syncRemove(id: batch.id); removing = nil } } }
        } message: { Text("Only do this after confirming delivery, or to cancel the transfer. Your local notebook is retained.") }
    }
    @MainActor func run(_ action: @escaping @MainActor () async throws -> Void) {
        guard !busy else { return }; busy = true
        Task { defer { busy = false }; do { try await action(); await refresh() } catch { if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } } }
    }
    @MainActor func refresh() async {
        do {
            let config = try await model.repository.connections().castalia
            expectedHolder = config?.holderPublicKey
            identityConfigured = config != nil && !(config?.policyDigest.isEmpty ?? true) && !(config?.credentialEndpoint.isEmpty ?? true)
            recoveryStored = try CustodySecureStorage().read(account: RecoveryImportCandidate.account) != nil
            status = try await model.repository.syncStatus()
        } catch { if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } }
    }
    func prerequisite(_ title: String, complete: Bool) -> some View {
        Label(title, systemImage: complete ? "checkmark.circle.fill" : "circle").font(.callout).foregroundStyle(complete ? nookAccent : .secondary)
    }
    func savedKey(_ id: String) throws -> String {
        guard let data = try CustodySecureStorage().read(account: "sync-device:\(id)") else { throw AuthorizationError.message("This device’s pairing key is missing. Pair the device again.") }
        return String(decoding: data, as: UTF8.self)
    }
    @MainActor func pair(id: String, name: String, secret: String, enabled: Bool) async throws {
        let json = try await model.repository.syncPreparePeer(id: id, label: name, key: secret, enabled: enabled)
        guard let raw = try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any], let version = raw["version"] as? NSNumber else { throw CocoaError(.coderReadCorrupt) }
        let receipt = try await model.authorization.authorize(payload: Data(json.utf8), action: "device.enroll", resourceVersion: version.uint64Value,
            disclosure: AuthorizationDisclosure(title: enabled ? "Pair \(name)" : "Disable \(name)", statements: [enabled ? "Allow encrypted private notebook transfers with device \(id), using this pairing key." : "Stop sending and receiving snapshots with device \(id).", "This does not automatically send any notebook content."]))
        // Save custody before committing permission: a failed Keychain write must not leave an unusable approved pair.
        let storage = CustodySecureStorage(); let account = "sync-device:\(id)"
        let previous = try storage.read(account: account)
        try storage.saveEncryptedMaterial(Data(secret.utf8), account: account)
        do { try await model.repository.syncApprovePeer(json, receipt: receipt) }
        catch {
            if let previous { try storage.saveEncryptedMaterial(previous, account: account) }
            else { try storage.remove(account: account) }
            throw error
        }
    }
    @MainActor func prepare() async throws {
        try await model.drainSaves()
        let prepared = try await model.repository.syncPrepare(peer: selectedPeer)
        guard let batch = prepared.batch.object, let id = batch["id"] as? String, let revision = batch["revision"] as? NSNumber else { throw CocoaError(.coderReadCorrupt) }
        do {
            let payload = try JSONSerialization.data(withJSONObject: batch)
            let receipt = try await model.authorization.authorize(payload: payload, action: "sync.send", resourceVersion: revision.uint64Value,
                disclosure: AuthorizationDisclosure(title: "Send a private reading snapshot", statements: ["Transfer \(prepared.books) books, \(prepared.notebooks) notebooks, \(prepared.cards) character cards and \(prepared.annotations) annotations to \(status?.peers.first(where: { $0.id == selectedPeer })?.label ?? selectedPeer).", "Include \(prepared.assets) accessible content files. The snapshot is encrypted with this device pair’s key.", "Only the snapshot at library revision \(revision) is approved; later edits stay local until another approval."]))
            _ = try await model.repository.syncApprove(id: id, key: savedKey(selectedPeer), receipt: receipt)
            message = "Approved snapshot is ready in the transfer queue. Export it when ready."
        } catch { try? await model.repository.syncCancel(id: id); throw error }
    }
    @MainActor func export(_ id: String) async throws {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".json")
        defer { try? FileManager.default.removeItem(at: url) }
        try await model.repository.syncExport(id: id, url: url)
        document = ExportDocument(data: try Data(contentsOf: url)); exporting = true
    }
    func resolve(_ conflict: SyncConflict, incoming: Bool) { run { try await model.drainSaves(); model.accept(try await model.repository.syncResolve(id: conflict.id, incoming: incoming)); model.reloadEditorSessions() } }
    func display(_ value: JSONValue) -> String { if case .string(let text) = value { return text }; guard let data = try? JSONSerialization.data(withJSONObject: value.any, options: [.prettyPrinted, .sortedKeys, .fragmentsAllowed]) else { return "Unavailable" }; return String(decoding: data, as: UTF8.self) }
}
