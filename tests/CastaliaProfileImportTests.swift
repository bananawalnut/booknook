// SPDX-License-Identifier: MPL-2.0
import Foundation

@MainActor private final class ProfileInstallGate {
    var continuation: CheckedContinuation<Void, Never>?
    func wait() async { await withCheckedContinuation { continuation = $0 } }
    func release() { continuation?.resume(); continuation = nil }
}
@MainActor private final class ProfileCatalogSource: CompanionCatalogSource {
    let bytes: Data
    var events: [String] = []
    init(_ bytes: Data) { self.bytes = bytes }
    func importKindleExport(_ bytes: Data) async throws -> CompanionKindleImport.Format { events.append("upload"); return .jsonCatalog }
    func kindleCatalog() async throws -> Data { events.append("catalog"); return bytes }
    func manifest() async throws -> CompanionManifest {
        events.append("manifest")
        return CompanionManifest(schema: "booknook.companion.v1", name: "Synthetic source", capabilities: ["provider_connections_v1", "kindle.catalog"], qualified: false)
    }
}

@MainActor func testCastaliaProfileImportWorkflow() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("booknook-profile-flow-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    let previous = ProcessInfo.processInfo.environment["BOOKNOOK_DATA_DIR"]
    defer {
        if let previous { setenv("BOOKNOOK_DATA_DIR", previous, 1) } else { unsetenv("BOOKNOOK_DATA_DIR") }
        try? FileManager.default.removeItem(at: root)
    }
    setenv("BOOKNOOK_DATA_DIR", root.path, 1)
    let model = DeskModel(); await model.open()
    precondition(model.error == nil)
    let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase
    let legacy = Data(#"{"schema":"booknook.connections.v1","castalia":null,"nostr_relays":[],"nostr_public_key":null,"audible_marketplace":"us","audible_test_asin":""}"#.utf8)
    let old = try decoder.decode(ConnectionConfiguration.self, from: legacy)
    precondition(old.castaliaProfileName == nil && old.providerConnections.isEmpty)

    var settings = old
    settings.audibleMarketplace = "ca"
    settings.audibleTestAsin = "B000000001"
    settings.nostrRelays = ["wss://relay.example.invalid/"]
    settings.nostrPublicKey = String(repeating: "b", count: 64)
    try await model.repository.saveConnections(settings)
    let kindle = ProviderConnection(schema: "castalia.provider-connection.v1", connectionId: "synthetic.kindle.catalog",
        providerId: "com.amazon.kindle", displayName: "Synthetic cache", capabilities: ["kindle.catalog"], status: "available", credential: nil)
    let audio = ProviderConnection(schema: "castalia.provider-connection.v1", connectionId: "synthetic.audible.ca",
        providerId: "com.audible", displayName: "Synthetic existing Audible account", capabilities: ["audible.positions"], status: "available",
        credential: ProviderCredentialReference(reference: "synthetic.opaque.reference", storage: "secure_local", portability: "device_bound"))
    let registered = try await model.repository.saveProviderConnections([kindle, audio])
    precondition(registered.providerConnections == [kindle, audio], "Required credential:null must survive Swift encoding and native validation")
    let profileObject: [String: Any] = ["schema": "castalia.native-authority-profile.v1", "display_name": "Synthetic existing provider",
        "issuer": "synthetic-existing-issuer", "key_id": "key-1", "issuer_public_key": String(repeating: "a", count: 64),
        "audience": "booknook", "caller": ["kind": "terminal", "id": "booknook-native"],
        "policy_digest_sha256": String(repeating: "c", count: 64), "credential_endpoint": "https://issuer.example.invalid/v1/credentials"]
    let bytes = try JSONSerialization.data(withJSONObject: profileObject, options: [.sortedKeys])
    let holder = String(repeating: "d", count: 64)
    func fails(_ action: () async throws -> Void) async {
        do { try await action(); preconditionFailure("Expected profile workflow rejection") } catch { }
    }
    for key in ["holder_public_key", "private_key", "access_token", "membership"] {
        var invalid = profileObject; invalid[key] = "must not import"
        let encoded = try JSONSerialization.data(withJSONObject: invalid)
        await fails { _ = try await model.repository.previewCastaliaProfile(encoded) }
    }
    let file = root.appendingPathComponent("provider.json")
    try bytes.write(to: file)
    let read = try await CastaliaProfileCandidate.readFile(file)
    precondition(read == bytes)
    let large = root.appendingPathComponent("oversized.json")
    try Data(repeating: 32, count: CastaliaProfileCandidate.maximumBytes + 1).write(to: large)
    await fails { _ = try await CastaliaProfileCandidate.readFile(large) }
    let preview = try await model.repository.previewCastaliaProfile(bytes)
    precondition(preview.profile.displayName == "Synthetic existing provider")
    let uninstalled = try await model.repository.connections()
    precondition(uninstalled.castalia == nil, "Preview must not install trust or identity")
    let candidate = CastaliaProfileCandidate(filename: "provider.json", bytes: bytes, preview: preview, configuredHolder: nil, unlockedHolder: nil, verifiedRecoveryHolder: holder, previousAuthority: nil)
    let session = CastaliaProfileImportSession(candidate: candidate) { data, preview, key in
        try await model.repository.installCastaliaProfile(data, preview: preview, holder: key)
    }
    precondition(session.holderPublicKey == holder, "A checked recovery identity must be available without copying a key")
    await fails { _ = try await session.apply() }
    session.recognizesProvider = true; session.holderPublicKey = "not a public key"
    await fails { _ = try await session.apply() }
    session.holderPublicKey = holder

    // Use a second actual Desk handle to change settings after the sheet's preview.
    // Native compare-and-install must reject the stale review instead of erasing it.
    let competing = DeskModel(); await competing.open()
    var newer = try await competing.repository.connections()
    newer.nostrRelays.append("wss://newer.example.invalid/")
    try await competing.repository.saveConnections(newer)
    await fails { _ = try await session.apply() }
    precondition(!session.installing && session.holderPublicKey == holder && session.recognizesProvider)
    let afterFailure = try await model.repository.connections()
    precondition(afterFailure.castalia == nil && afterFailure.nostrRelays == newer.nostrRelays)

    let freshPreview = try await model.repository.previewCastaliaProfile(bytes)
    let fresh = CastaliaProfileCandidate(filename: "provider.json", bytes: bytes, preview: freshPreview, configuredHolder: nil, unlockedHolder: holder, verifiedRecoveryHolder: nil, previousAuthority: nil)
    let cancelled = CastaliaProfileImportSession(candidate: fresh) { _, _, _ in preconditionFailure("Cancelled import must not invoke persistence") }
    cancelled.holderPublicKey = holder; cancelled.recognizesProvider = true; cancelled.cancel()
    await fails { _ = try await cancelled.apply() }

    let gate = ProfileInstallGate()
    let committing = CastaliaProfileImportSession(candidate: fresh) { data, preview, key in
        let saved = try await model.repository.installCastaliaProfile(data, preview: preview, holder: key)
        await gate.wait() // Simulate cancellation after a real SQLite commit.
        return saved
    }
    committing.holderPublicKey = holder; committing.recognizesProvider = true
    let task = Task { try await committing.apply() }
    while gate.continuation == nil { await Task.yield() }
    task.cancel(); gate.release()
    let saved = try await task.value
    precondition(saved.castalia?.holderPublicKey == holder && saved.castaliaProfileName == "Synthetic existing provider")
    precondition(saved.nostrRelays == newer.nostrRelays && saved.nostrPublicKey == newer.nostrPublicKey)
    precondition(saved.audibleMarketplace == "ca" && saved.audibleTestAsin == "B000000001")
    precondition(saved.providerConnections == [kindle, audio], "Profile installation must preserve the source registry")
    var staleManualDraft = newer
    staleManualDraft.audibleMarketplace = "uk"
    await fails { _ = try await competing.repository.saveConnectionsIfCurrent(staleManualDraft, expected: newer) }
    let afterStaleManual = try await model.repository.connections()
    precondition(afterStaleManual == saved, "A stale manual settings form must not undo the explicitly installed provider")
    precondition(!committing.installing && !committing.canInstall)
    await fails { _ = try await committing.apply() }
    let exported = try await model.repository.exportCastaliaProfile()
    let exportedObject = try JSONSerialization.jsonObject(with: exported) as! [String: Any]
    precondition(exportedObject["holder_public_key"] == nil && exportedObject["access_token"] == nil && exportedObject["membership"] == nil)
    precondition(exportedObject["display_name"] as? String == "Synthetic existing provider")
    let reopened = DeskModel(); await reopened.open()
    let recovered = try await reopened.repository.connections()
    precondition(recovered.castalia == saved.castalia && recovered.castaliaProfileName == saved.castaliaProfileName)
    precondition(recovered.nostrRelays == newer.nostrRelays)

    // Exercise the actual file-import controller's post-upload connection resolver,
    // then attach a source observation through the same native wrapper as the review UI.
    let catalog = Data(#"{"schema":"booknook.kindle-catalog.v1","items":[{"asin":"B000000001","title":"Provider-linked book","authors":["Synthetic writer"]}]}"#.utf8)
    let source = ProfileCatalogSource(catalog)
    let catalogFile = root.appendingPathComponent("catalog.json"); try catalog.write(to: catalogFile)
    let flow = CompanionCatalogImportWorkflow()
    _ = try await flow.run(file: catalogFile, source: source, model: model, connection: {
        source.events.append("connection"); return kindle.connectionId
    })
    precondition(source.events == ["upload", "connection", "catalog", "manifest"])
    precondition(model.books.count == 1)
    try await model.selectBook(model.books[0].id)
    let pass = model.notebook!.passId
    let now = UInt64(Date().timeIntervalSince1970)
    let observation = CompanionPosition(asin: "B000000001", positionMilliseconds: 12_345,
        sourceUpdatedAt: now, fetchedAt: now, durationMilliseconds: 60_000,
        editionIdentifier: nil, alignment: "unmatched", metadata: nil)
    model.accept(try await model.repository.attachAudio(observation, pass: pass, edition: nil, connection: audio.connectionId))
    precondition(model.state!.positions.values.contains { $0.providerConnectionId == audio.connectionId })
    let exportedDesk = root.appendingPathComponent("desk.json")
    try await model.repository.export(to: exportedDesk)
    let archive = try JSONSerialization.jsonObject(with: Data(contentsOf: exportedDesk)) as! [String: Any]
    let state = archive["state"] as! [String: Any]
    let books = state["books"] as! [String: [String: Any]]
    let provenance = books.values.first!["provenance"] as! [[String: Any]]
    precondition(provenance.contains { $0["provider_connection_id"] as? String == kindle.connectionId })
    let exportedBytes = try Data(contentsOf: exportedDesk)
    precondition(!String(decoding: exportedBytes, as: UTF8.self).contains("synthetic.opaque.reference"), "Private device registry/credential handles must not leak through archive export")
    let cleared = try await model.repository.saveProviderConnections([])
    precondition(cleared.castalia == saved.castalia && cleared.nostrRelays == saved.nostrRelays)
    await fails { _ = try await model.repository.attachAudio(observation, pass: pass, edition: nil, connection: audio.connectionId) }
    await fails { _ = try await model.repository.importKindle(catalog, at: now, connection: kindle.connectionId) }
    print("Castalia public profile: actual strict native validation, legacy settings decode, bounded file read, explicit trust/holder, stale-preview rejection across SQLite handles, preserved unrelated settings, cancellation after durable commit, public-only export and reopen passed; no Keychain or network operations.")
    print("Source connection workflow: required nullable credential round-trip, post-upload connection resolution, Kindle provenance, captured audio connection, archive separation and cleared-reference rejection passed through actual Swift/native SQLite boundaries.")
}
