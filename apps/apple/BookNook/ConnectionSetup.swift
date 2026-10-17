// SPDX-License-Identifier: MPL-2.0
import SwiftUI
import UniformTypeIdentifiers
import CryptoKit
#if os(macOS)
import AppKit
#else
import UIKit
#endif

struct CastaliaConfiguration: Codable, Equatable {
    var issuer = ""
    var keyId = ""
    var issuerPublicKey = ""
    var audience = ""
    var terminalCaller = "booknook-native"
    var holderPublicKey = ""
    var policyDigest = ""
    var credentialEndpoint = ""
}
struct ConnectionConfiguration: Codable, Equatable {
    var schema = "booknook.connections.v1"
    var castalia: CastaliaConfiguration?
    var nostrRelays: [String] = []
    var nostrPublicKey: String?
    var audibleMarketplace = "us"
    var audibleTestAsin = ""
    var castaliaProfileName: String?
    var providerConnections: [ProviderConnection] = []
    init() {}
    enum CodingKeys: String, CodingKey {
        case schema, castalia, nostrRelays, nostrPublicKey, audibleMarketplace, audibleTestAsin
        case castaliaProfileName, providerConnections
    }
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        schema = try values.decode(String.self, forKey: .schema)
        castalia = try values.decodeIfPresent(CastaliaConfiguration.self, forKey: .castalia)
        nostrRelays = try values.decode([String].self, forKey: .nostrRelays)
        nostrPublicKey = try values.decodeIfPresent(String.self, forKey: .nostrPublicKey)
        audibleMarketplace = try values.decode(String.self, forKey: .audibleMarketplace)
        audibleTestAsin = try values.decode(String.self, forKey: .audibleTestAsin)
        castaliaProfileName = try values.decodeIfPresent(String.self, forKey: .castaliaProfileName)
        providerConnections = try values.decodeIfPresent([ProviderConnection].self, forKey: .providerConnections) ?? []
    }
}
struct PendingCompanionLogin: Codable {
    let endpoint: String
    let provider: String
    let marketplace: String
    let authorization: CompanionAuthorization
}

private enum ConnectionFileImport {
    case recovery, profile, catalog
    var contentTypes: [UTType] {
        switch self {
        case .recovery: return [.data]
        case .profile: return [.json, .data]
        case .catalog: return [.json, .xml, .data]
        }
    }
}

struct ConnectionSetupView: View {
    @EnvironmentObject var model: DeskModel
    @State private var configuration = ConnectionConfiguration()
    @State private var savedConfiguration: ConnectionConfiguration?
    @State private var castalia = CastaliaConfiguration()
    @State private var useCastalia = false
    @State private var relays = ""
    @State private var nostrKey = ""
    @State private var audibleHeaders = ""
    @State private var status = ""
    @State private var testing = false
    @State private var savingConnections = false
    @State private var importingFile = false
    @State private var fileImport: ConnectionFileImport = .profile
    @State private var readingProfile = false
    @State private var profileCandidate: CastaliaProfileCandidate?
    @State private var exportingProfile = false
    @State private var profileDocument: ExportDocument?
    @State private var recoveryCandidate: RecoveryImportCandidate?
    @State private var verifiedRecoveryHolder: String?
    @State private var recoveryStatus = "No encrypted recovery material stored."
    @State private var headersStored = false
    @State private var removingRecovery = false
    @State private var unlockPassphrase = ""
    @State private var issuerAccessDraft = IssuerAccessDraft()
    @State private var unlocking = false
    @State private var companionURL = ""
    @State private var pairingCode = ""
    @State private var companionStatus = "No companion paired."
    @State private var companionCapabilities: [String] = []
    @State private var companionQualified = false
    @State private var providerRegistryGeneration = UUID()
    @StateObject private var catalogImport = CompanionCatalogImportWorkflow()
    @State private var companionObservations: [CompanionPosition] = []
    @State private var companionObservationSource: BookNookProviderRead?
    @State private var companionObservationConnectionID: String?
    @State private var reviewingAudio: AudioObservationSelection?
    @State private var pendingLogin: PendingCompanionLogin?
    @State private var forgettingCompanionLogin = false
    @State private var returnedLoginURL = ""
    @State private var registrationConsent = false
    @State private var disconnectingAudible = false
    @State private var resettingAudibleRegistration = false
    @State private var importingKindleWeb = false
    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            DisclosureGroup("Sign in with Castalia") {
                VStack(alignment: .leading, spacing: 12) {
                    Text("Bring your existing Castalia identity. There is no Book Nook membership or separate account. Your library and private notebook also work offline without signing in.").font(.callout).foregroundStyle(.secondary)
                    Text("1. Bring your identity").font(.headline)
                    Button("Import encrypted .castalia-recovery file") { chooseFile(.recovery) }
                    Text(recoveryStatus).font(.caption).foregroundStyle(.secondary)
                    Text("Check your existing recovery file with its passphrase. Its public identity will be available in the next step; no key needs to be copied by hand.").font(.caption).foregroundStyle(.secondary)
                    Button("Remove this device’s stored recovery material", role: .destructive) { removingRecovery = true }.font(.caption)
                    Text("2. Review your provider").font(.headline)
                    if let name = configuration.castaliaProfileName { Text("Provider: \(name)").font(.headline) }
                    HStack {
                        Button(readingProfile ? "Checking profile…" : "Import public provider profile") { chooseFile(.profile) }.disabled(readingProfile || savingConnections)
                        Button("Export public provider profile") { Task { await exportProviderProfile() } }.disabled(configuration.castalia == nil || readingProfile || savingConnections)
                    }
                    Text("The public profile carries no recovery file or access token. Importing it does not grant permission to publish or sync.").font(.caption).foregroundStyle(.secondary)
                    DisclosureGroup("Advanced manual provider settings") {
                    VStack(alignment: .leading, spacing: 12) {
                    Toggle("Configure trusted issuer", isOn: $useCastalia)
                    TextField("Issuer identifier", text: $castalia.issuer)
                    TextField("Issuer key ID", text: $castalia.keyId)
                    TextField("Issuer public key (lowercase hex)", text: $castalia.issuerPublicKey)
                    TextField("Audience", text: $castalia.audience)
                    TextField("Approved terminal caller ID", text: $castalia.terminalCaller)
                    TextField("Your Castalia holder public key", text: $castalia.holderPublicKey)
                    TextField("Policy SHA-256 digest", text: $castalia.policyDigest)
                    TextField("Issuer credential endpoint (HTTPS)", text: Binding(get: { castalia.credentialEndpoint }, set: { endpoint in
                        clearIssuerDraftIfDestinationChanged(to: endpoint)
                        castalia.credentialEndpoint = endpoint
                    }))
                    Text("Existing manual settings remain supported. Choose Save connections after editing these fields.").font(.caption).foregroundStyle(.secondary)
                    }.padding(.top, 10)
                    }
                    Text("3. Unlock when you need to approve an action").font(.headline)
                    SecureField("Issuer access token (optional; saved in Keychain)", text: Binding(get: { issuerAccessDraft.value }, set: { issuerAccessDraft.update($0, endpoint: castalia.credentialEndpoint) }))
                    Button("Remove this issuer’s saved access", role: .destructive) { do { try CustodySecureStorage().remove(account: issuerAccessAccount(castalia.credentialEndpoint)); issuerAccessDraft.clear(); status = "Issuer access token removed from this device." } catch { if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } } }.font(.caption)
                    Text("Trust is configured here, independently of incoming requests. Rolodex attribution and Nostr keys do not grant permission.").font(.caption).foregroundStyle(.secondary)
                    SecureField("Recovery passphrase", text: $unlockPassphrase)
                    HStack {
                        Button(unlocking ? "Unlocking…" : "Unlock Castalia") { Task { await unlockCustody() } }.disabled(unlocking || unlockPassphrase.isEmpty)
                    }
                    CustodyLockStatus(coordinator: model.authorization)
                    Text("The current connection uses secure storage on this device. Unlock runs locally. Every sync or publication needs its own approval. The identity locks after five minutes or when Book Nook enters the background.").font(.caption).foregroundStyle(.secondary)
                }.textFieldStyle(.roundedBorder).padding(.top, 14)
            }
            Divider()
            DisclosureGroup("Nostr relay exchange") {
                VStack(alignment: .leading, spacing: 12) {
                    TextField("wss:// relay URLs, one per line", text: $relays, axis: .vertical).lineLimit(2...5)
                    TextField("Nostr public key (lowercase hex)", text: $nostrKey)
                    Text("The transport key must be explicitly bound to your Castalia identity before publication. No private key is derived or generated here.").font(.caption).foregroundStyle(.secondary)
                    Button("Test first relay") { Task { await probeRelay() } }.disabled(testing || relays.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }.textFieldStyle(.roundedBorder).padding(.top, 14)
            }
            Divider()
            DisclosureGroup("Library companion") {
                VStack(alignment: .leading, spacing: 12) {
                    TextField("Companion URL (HTTPS, or loopback HTTP)", text: $companionURL).textFieldStyle(.roundedBorder).disabled(testing)
                    SecureField("One-time pairing code shown by your companion", text: $pairingCode).textFieldStyle(.roundedBorder)
                    HStack {
                        Button("Pair companion") { Task { await pairCompanion() } }.disabled(testing || pairingCode.isEmpty)
                        Button("Refresh capabilities") { Task { await refreshCompanion() } }.disabled(testing || companionURL.isEmpty)
                    }
                    Text(companionStatus).font(.caption).foregroundStyle(.secondary)
                    if !companionCapabilities.isEmpty {
                        Text(companionQualified ? "Source adapter reports qualified access; live account acceptance remains separate." : "This companion has not reported qualified live account access.").font(.caption).foregroundStyle(.secondary)
                        Text("Available: " + companionCapabilities.map(capabilityName).joined(separator: ", ")).font(.caption)
                    }
                    ForEach(configuration.providerConnections) { connection in
                        VStack(alignment: .leading, spacing: 4) {
                            Text(connection.displayName).font(.headline)
                            Text("\(connection.statusLabel) · \(connection.portabilityLabel)").font(.caption)
                            Text("Connection: \(connection.connectionId)").font(.caption.monospaced()).textSelection(.enabled)
                            Text("Provider: \(connection.providerId)").font(.caption).textSelection(.enabled)
                            Text("Capabilities: \(connection.capabilities.joined(separator: ", "))").font(.caption).foregroundStyle(.secondary)
                        }.accessibilityElement(children: .combine)
                    }
                    if !configuration.providerConnections.isEmpty {
                        Text("These are public source references. Moving a reference does not move its secret credentials or make another device signed in.").font(.caption).foregroundStyle(.secondary)
                    }
                    if !supports("kindle.authorize") && !supports("audible.authorize") {
                        Text("Companion account login needs a source adapter installed there. The separate experimental Kindle US web sign-in and local catalog imports are available under Kindle library.").font(.callout).foregroundStyle(.secondary)
                    }
                    Button("Disconnect companion", role: .destructive) { Task { await disconnectCompanion() } }.disabled(testing || pendingLogin != nil)
                    Text("Disconnect any companion Audible account before unpairing. Unpairing revokes this device’s companion token; it does not deregister an Amazon device.").font(.caption).foregroundStyle(.secondary)
                    Text("Your companion connects to the data sources you choose. Pairing exchanges the displayed code for a device token kept in Keychain. Account login stays on the source provider’s authorization page.").font(.caption).foregroundStyle(.secondary)
                    if let login = pendingLogin {
                        VStack(alignment: .leading, spacing: 10) {
                            Text("Finish \(login.provider.capitalized) login (\(login.marketplace.uppercased()))").font(.headline)
                            Text("Companion: \(URL(string: login.endpoint)?.host ?? login.endpoint)").font(.caption)
                            Text("Complete login and any MFA in your browser. Copy the full final Amazon URL whose path is /ap/maplanding and paste it below. Treat that URL as a secret. It is never saved in your notebook or library.").font(.caption)
                            if let expires = login.authorization.expiresAt { Text("Session expires \(Date(timeIntervalSince1970: TimeInterval(expires)).formatted()).").font(.caption) }
                            SecureField("Final Amazon authorization URL", text: $returnedLoginURL).textFieldStyle(.roundedBorder).disabled(testing)
                            Toggle("Register a virtual Audible device in my Amazon account", isOn: $registrationConsent).disabled(testing)
                            Text("The optional unofficial adapter identifies this registration as Audible for iPhone. It appears in Amazon’s device list and stores its access credentials in the companion’s OS credential store. Playback stays in Audible. Successful registration does not establish live position accuracy or provide Kindle access. Position reads preserve source times and require your edition review.").font(.caption).foregroundStyle(.secondary)
                            HStack {
                                Button("Open login page") { Task { await reopenCompanionLogin(login.authorization) } }.disabled(testing)
                                Button("Complete registration") { Task { await completeCompanionLogin() } }.disabled(testing || !registrationConsent || returnedLoginURL.isEmpty)
                                Button("Cancel login") { Task { await cancelCompanionLogin() } }.disabled(testing)
                            }
                            Button("Forget saved login on this device", role: .destructive) { forgettingCompanionLogin = true }.disabled(testing)
                        }.padding().background(.secondary.opacity(0.06), in: RoundedRectangle(cornerRadius: 8))
                    }
                }.padding(.top, 14)
            }
            Divider()
            DisclosureGroup("Audible listening position") {
                VStack(alignment: .leading, spacing: 12) {
                    Picker("Marketplace", selection: $configuration.audibleMarketplace) { Text("United States").tag("us"); Text("United Kingdom").tag("uk"); Text("Germany").tag("de"); Text("Canada").tag("ca"); Text("Australia").tag("au") }.disabled(testing)
                    TextField("A book ASIN to test", text: $configuration.audibleTestAsin).textFieldStyle(.roundedBorder)
                    SecureField(headersStored ? "Replace stored request-header JSON (optional)" : "Request-header JSON from your local connector", text: $audibleHeaders).textFieldStyle(.roundedBorder)
                    Text("A paired companion can provide renewed account access and timestamped position observations. The direct request-header probe is an advanced diagnostic. Playback stays in Audible.").font(.caption).foregroundStyle(.secondary)
                    ForEach(companionObservations) { observation in
                        Button("Review \(observation.metadata?.title ?? observation.asin) · \(observation.positionMilliseconds / 1000) s") {
                            reviewingAudio = AudioObservationSelection(observation: observation, connectionID: companionObservationConnectionID, sourceRead: companionObservationSource)
                        }.disabled(testing)
                    }
                    Button("Read position through companion") { Task { await lookupCompanionPosition() } }.disabled(testing || configuration.audibleTestAsin.isEmpty || !supports("audible.positions"))
                    Button("Connect Audible account in companion") { Task { await startCompanionAccount("audible") } }.disabled(testing || pendingLogin != nil || !supports("audible.authorize"))
                    Button("Renew Audible account access") { Task { await renewCompanionAccount("audible") } }.disabled(testing || !supports("account.renew"))
                    Button("Probe companion account access") { Task { await probeCompanionAccount() } }.disabled(testing || !supports("audible.positions.probe") || configuration.audibleTestAsin.isEmpty)
                    Button("Disconnect companion Audible account", role: .destructive) { disconnectingAudible = true }.disabled(testing || !supports("account.disconnect"))
                    Button("Resolve unfinished Audible registration") { resettingAudibleRegistration = true }.disabled(testing || !supports("account.registration.reset"))
                    Button("Test position access") { Task { await probeAudible() } }.disabled(testing || configuration.audibleTestAsin.isEmpty)
                    if headersStored { Button("Disconnect stored Audible access", role: .destructive) { do { try CustodySecureStorage().remove(account: "audible-request-headers"); headersStored = false; audibleHeaders = ""; status = "Stored Audible access removed from this device." } catch { if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } } } }
                }.padding(.top, 14)
            }
            Divider()
            DisclosureGroup("Kindle library") {
                VStack(alignment: .leading, spacing: 12) {
                    Button("Sign in to Kindle US and import catalog…") { importingKindleWeb = true }.disabled(testing)
                    Text("Experimental, US-only web sign-in imports titles and authors directly. It uses a temporary session, needs no companion, and does not import book text or reading positions. Live Amazon compatibility remains unqualified.").font(.caption).foregroundStyle(.secondary)
                    Button("Connect Kindle account in companion") { Task { await startCompanionAccount("kindle") } }.disabled(testing || pendingLogin != nil || !supports("kindle.authorize"))
                    Button("Renew Kindle account access") { Task { await renewCompanionAccount("kindle") } }.disabled(testing || !supports("kindle.authorize") || !supports("account.renew"))
                    Button("Choose catalog JSON/XML or Mac cache snapshot") { chooseFile(.catalog) }.disabled(testing || !supports("kindle.catalog.import"))
                    Button("Refresh Kindle library") { Task { await importCompanionLibrary() } }.disabled(testing || !supports("kindle.catalog"))
                    Text("Catalog JSON/XML is sent to your paired companion. A Kindle-for-Mac SQLite snapshot is accepted only by a companion on this device at 127.0.0.1 or [::1]. For a companion on another device, export catalog JSON locally first.").font(.caption).foregroundStyle(.secondary)
                    Text("Quit Kindle and follow the companion’s local export instructions to create a complete standalone backup. Copying the database file alone can omit recent entries held in a separate journal. Keep Kindle’s live database unchanged.").font(.caption).foregroundStyle(.secondary)
                    Text("Cache import reads ASINs, titles and authors only. It does not prove a complete purchased library, import book text, or infer progress from last-access times.").font(.caption).foregroundStyle(.secondary)
                    Text("Refresh imports catalog metadata and progress only when the source explicitly supplies it. Private edits and books without accessible text are preserved. Attach EPUB/PDF content separately, or use Import → Kindle catalog JSON for a normalized local export.").font(.caption).foregroundStyle(.secondary)
                }.padding(.top, 14)
            }
            HStack { Button(savingConnections ? "Saving…" : "Save connections") { Task { await save() } }.buttonStyle(.borderedProminent).disabled(savingConnections); if testing { ProgressView().controlSize(.small) }; Text(status).font(.caption).foregroundStyle(.secondary) }
        }.padding(22).background(nookAccent.opacity(0.04), in: RoundedRectangle(cornerRadius: 12))
        .confirmationDialog("Forget only this device’s saved pending login? This works when the companion is unavailable. It does not cancel a remote session, deregister an Amazon device, or delete companion account credentials. Check the companion and Amazon if registration may already have occurred.", isPresented: $forgettingCompanionLogin, titleVisibility: .visible) {
            Button("Forget local pending login", role: .destructive) {
                do {
                    try clearCompanionLogin()
                    status = "Pending login forgotten on this device. No remote cancellation or account removal was performed."
                } catch { model.error = bookNookErrorMessage(error) }
            }
        }
        .confirmationDialog("Deregister this paired device’s virtual Audible registration from Amazon and remove its stored credentials? Other Audible devices remain registered.", isPresented: $disconnectingAudible, titleVisibility: .visible) {
            Button("Disconnect Audible account", role: .destructive) { Task { await disconnectCompanionAccount() } }
        }
        .confirmationDialog("First check Amazon’s device list and remove the unfinished virtual Audible device if it exists. This clears only this paired device’s local uncertainty marker so you can try login again; it does not remove or verify any Amazon device. Saved account credentials cannot be reset here.", isPresented: $resettingAudibleRegistration, titleVisibility: .visible) {
            Button("I checked and removed the unfinished Amazon device") { Task { await resetCompanionRegistration() } }
        }
        .confirmationDialog("Remove stored recovery material from this device? Keep your original recovery file to recover the wallet later.", isPresented: $removingRecovery, titleVisibility: .visible) {
            Button("Remove from this device", role: .destructive) { Task { await model.authorization.lock(); do { try CustodySecureStorage().remove(account: "encrypted-recovery"); verifiedRecoveryHolder = nil; recoveryStatus = "No encrypted recovery material stored." } catch { if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } } } }
        }
        .sheet(isPresented: $importingKindleWeb) {
            KindleWebImportSheet { status = $0 }.environmentObject(model)
        }
        .sheet(item: $reviewingAudio) { selection in AudioMatchReview(observation: selection.observation, connectionID: selection.connectionID, sourceRead: selection.sourceRead).environmentObject(model) }
        .sheet(item: $profileCandidate) { candidate in
            CastaliaProfileReview(candidate: candidate, repository: model.repository) { settings in
                await model.authorization.lock()
                configuration.castalia = settings.castalia
                configuration.castaliaProfileName = settings.castaliaProfileName
                configuration.providerConnections = settings.providerConnections
                var baseline = savedConfiguration ?? settings
                baseline.castalia = settings.castalia; baseline.castaliaProfileName = settings.castaliaProfileName
                baseline.providerConnections = settings.providerConnections
                savedConfiguration = baseline
                let clearedAccessDraft = issuerAccessDraft.clearIfDestinationChanged(to: settings.castalia?.credentialEndpoint ?? "")
                castalia = settings.castalia ?? CastaliaConfiguration(); useCastalia = settings.castalia != nil
                unlockPassphrase = ""
                status = "Public provider settings saved for the selected identity. Stored recovery material and saved access tokens are unchanged. Unlock a matching recovery file to approve actions."
                if clearedAccessDraft { status += " The unsaved access token was cleared because the destination changed. Enter access for the new provider if needed." }
                profileCandidate = nil
            }
        }
        .sheet(item: $recoveryCandidate) { candidate in
            RecoveryImportReview(candidate: candidate, repository: model.repository) { key in
                await model.authorization.lock()
                unlockPassphrase = ""
                if castalia.holderPublicKey.isEmpty { castalia.holderPublicKey = key }
                verifiedRecoveryHolder = key
                recoveryStatus = "Recovery file verified and saved. Enter its passphrase to unlock when needed."
                recoveryCandidate = nil
            }
        }
        .onChange(of: companionURL) { _, _ in
            companionCapabilities = []; companionQualified = false; companionObservations = []; companionObservationConnectionID = nil; companionObservationSource = nil
            clearProviderRegistry()
        }
        .onChange(of: configuration.audibleMarketplace) { _, _ in
            companionObservations = []; companionObservationConnectionID = nil; companionObservationSource = nil; clearProviderRegistry()
        }
        .onChange(of: castalia.credentialEndpoint) { _, endpoint in
            clearIssuerDraftIfDestinationChanged(to: endpoint)
        }
        .task {
            do { configuration = try await model.repository.connections(); savedConfiguration = configuration; castalia = configuration.castalia ?? CastaliaConfiguration(); useCastalia = configuration.castalia != nil; relays = configuration.nostrRelays.joined(separator: "\n"); nostrKey = configuration.nostrPublicKey ?? ""; headersStored = try CustodySecureStorage().read(account: "audible-request-headers") != nil; if try CustodySecureStorage().read(account: "encrypted-recovery") != nil { recoveryStatus = "Encrypted recovery material is stored in this device’s Keychain." } }
            catch { if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } }
            companionURL = UserDefaults.standard.string(forKey: "booknook.companion.url") ?? ""
            do { if let bytes = try CustodySecureStorage().read(account: "pending-companion-login") { pendingLogin = try JSONDecoder().decode(PendingCompanionLogin.self, from: bytes) } }
            catch { model.error = bookNookErrorMessage(error) }
            if !companionURL.isEmpty { await refreshCompanion() }
        }
        .fileImporter(isPresented: $importingFile, allowedContentTypes: fileImport.contentTypes) { result in
            // One presentation owner serves all three buttons. Keep the route
            // independently of the presentation Boolean, which SwiftUI clears.
            receiveImportedFile(result, for: fileImport)
        }
        .fileExporter(isPresented: $exportingProfile, document: profileDocument, contentType: .json, defaultFilename: "Castalia-public-provider") { result in
            if case .failure(let error) = result, !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) }
        }
    }
    private func chooseFile(_ kind: ConnectionFileImport) {
        fileImport = kind
        importingFile = true
    }
    private func receiveImportedFile(_ result: Result<URL, Error>, for kind: ConnectionFileImport) {
        switch kind {
        case .catalog:
            Task {
                do { try await importCatalogFile(result.get()) }
                catch { if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } }
            }
        case .profile:
            Task {
                do { try await importProviderProfile(result.get()) }
                catch { if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } }
            }
        case .recovery:
            if case .failure(let error) = result, bookNookOperationWasCancelled(error) { return }
            do {
                let url = try result.get(); let access = url.startAccessingSecurityScopedResource()
                defer { if access { url.stopAccessingSecurityScopedResource() } }
                recoveryCandidate = try RecoveryImportCandidate.readFile(url, storage: CustodySecureStorage())
            } catch { if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } }
        }
    }
    func save() async {
        guard !savingConnections else { return }
        savingConnections = true
        defer { savingConnections = false }
        let enteredHeaders = audibleHeaders
        let enteredAccessDraft = issuerAccessDraft
        do {
            guard let baseline = savedConfiguration else { throw ConnectionInputError.message("Reopen Connections to load its saved settings before saving. Your entries have not been changed.") }
            let inputs = try ConnectionSaveInputs(audibleHeaders: enteredHeaders, issuerToken: enteredAccessDraft.token(for: castalia.credentialEndpoint), issuerEndpoint: castalia.credentialEndpoint)
            var settings = configuration
            if settings.castalia != (useCastalia ? castalia : nil) { settings.castaliaProfileName = nil }
            settings.castalia = useCastalia ? castalia : nil
            settings.nostrRelays = relays.split(whereSeparator: \.isNewline).map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
            settings.nostrPublicKey = nostrKey.isEmpty ? nil : nostrKey
            let result = try await saveConnectionInputs(inputs, saveSettings: {
                savedConfiguration = try await model.repository.saveConnectionsIfCurrent(settings, expected: baseline)
                // Invalidate outstanding approval even if a later edit returns to the old pins.
                if settings.castalia != baseline.castalia { await model.authorization.lock() }
            }, saveAudible: { bytes in
                try CustodySecureStorage().saveEncryptedMaterial(bytes, account: "audible-request-headers")
            }, saveIssuer: { bytes, endpoint in
                try CustodySecureStorage().saveEncryptedMaterial(bytes, account: issuerAccessAccount(endpoint))
            })
            configuration.castalia = settings.castalia
            configuration.castaliaProfileName = settings.castaliaProfileName
            if result.audibleAccessSaved {
                if audibleHeaders == enteredHeaders { audibleHeaders = "" }
                headersStored = true
            }
            if result.issuerAccessSaved, issuerAccessDraft == enteredAccessDraft { issuerAccessDraft.clear() }
            if let error = result.error {
                status = "Connection settings saved. Some access details still need saving."
                model.error = "Connection settings were saved, but \(result.failedAccess ?? "access details") could not be saved: \(bookNookErrorMessage(error)) Unsaved access entries remain in this form. Choose Save connections to retry."
            } else {
                status = "Saved on this device. Connection validation is separate."
            }
        } catch {
            status = "Connections were not saved. Your entries are still here."
            if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) }
        }
    }
    func clearIssuerDraftIfDestinationChanged(to endpoint: String) {
        if issuerAccessDraft.clearIfDestinationChanged(to: endpoint) {
            status = "The unsaved access token was cleared because its destination changed. Enter access for the new provider if needed. Saved Keychain tokens are unchanged."
        }
    }
    func importProviderProfile(_ url: URL) async throws {
        guard !readingProfile else { throw ConnectionInputError.message("Wait for the current profile to finish checking.") }
        readingProfile = true; defer { readingProfile = false }
        let bytes = try await CastaliaProfileCandidate.readFile(url)
        let preview = try await model.repository.previewCastaliaProfile(bytes)
        let settings = try await model.repository.connections()
        try Task.checkCancellation()
        profileCandidate = CastaliaProfileCandidate(filename: url.lastPathComponent, bytes: bytes, preview: preview,
            configuredHolder: settings.castalia?.holderPublicKey, unlockedHolder: model.authorization.unlockedPublicKey,
            verifiedRecoveryHolder: verifiedRecoveryHolder, previousAuthority: settings.castalia)
    }
    func exportProviderProfile() async {
        do {
            profileDocument = ExportDocument(data: try await model.repository.exportCastaliaProfile())
            exportingProfile = true
        } catch { if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } }
    }
    func unlockCustody() async {
        unlocking = true
        let passphrase = unlockPassphrase
        unlockPassphrase = ""
        defer { unlocking = false }
        do {
            let key = try await model.authorization.unlock(passphrase: passphrase)
            if castalia.holderPublicKey.isEmpty { castalia.holderPublicKey = key }
            recoveryStatus = "Encrypted recovery file verified."
        } catch { if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } }
    }
    func companion() throws -> LibraryCompanion { try LibraryCompanion(endpoint: companionURL) }
    func pairCompanion() async {
        testing = true; defer { testing = false }
        do { let client = try companion(); let capabilities = try await client.pair(code: pairingCode); pairingCode = ""; UserDefaults.standard.set(companionURL, forKey: "booknook.companion.url"); companionCapabilities = capabilities.capabilities; companionQualified = capabilities.qualified; companionStatus = "Paired with \(capabilities.name)."; await updateProviderRegistryStatus() }
        catch { if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } }
    }
    func refreshCompanion() async {
        testing = true; defer { testing = false }
        do {
            let endpoint = companionURL
            let value = try await LibraryCompanion(endpoint: endpoint).manifest()
            guard companionURL == endpoint else { return }
            companionCapabilities = value.capabilities; companionQualified = value.qualified; companionStatus = "Connected to \(value.name)."
            await updateProviderRegistryStatus()
        }
        catch { companionCapabilities = []; companionQualified = false; companionStatus = bookNookErrorMessage(error) }
    }
    func disconnectCompanion() async {
        do { try await companion().disconnect(); companionCapabilities = []; companionQualified = false; companionStatus = "Companion disconnected."; UserDefaults.standard.removeObject(forKey: "booknook.companion.url"); companionURL = "" }
        catch { if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } }
    }
    func supports(_ capability: String) -> Bool { !companionURL.isEmpty && companionCapabilities.contains(capability) }
    func clearProviderRegistry() {
        let generation = UUID(); providerRegistryGeneration = generation
        configuration.providerConnections = []
        Task {
            do {
                guard generation == providerRegistryGeneration else { return }
                let saved = try await model.repository.saveProviderConnections([])
                if generation == providerRegistryGeneration { savedConfiguration?.providerConnections = saved.providerConnections }
            } catch { if generation == providerRegistryGeneration { model.error = bookNookErrorMessage(error) } }
        }
    }
    func refreshProviderRegistry(endpoint: String, marketplace: String) async throws -> [ProviderConnection] {
        let generation = providerRegistryGeneration
        guard companionURL == endpoint, configuration.audibleMarketplace == marketplace else { throw CancellationError() }
        guard supports("provider_connections_v1") else { return [] }
        let values = try await LibraryCompanion(endpoint: endpoint).providerConnections(marketplace: marketplace)
        try Task.checkCancellation()
        guard generation == providerRegistryGeneration, companionURL == endpoint, configuration.audibleMarketplace == marketplace else { throw CancellationError() }
        let saved = try await model.repository.saveProviderConnections(values)
        guard generation == providerRegistryGeneration, companionURL == endpoint, configuration.audibleMarketplace == marketplace else { throw CancellationError() }
        configuration.providerConnections = saved.providerConnections
        savedConfiguration?.providerConnections = saved.providerConnections
        return values
    }
    func updateProviderRegistryStatus() async {
        guard supports("provider_connections_v1") else { return }
        do { _ = try await refreshProviderRegistry(endpoint: companionURL, marketplace: configuration.audibleMarketplace) }
        catch { if !bookNookOperationWasCancelled(error) { companionStatus += " Source references could not be refreshed. \(bookNookErrorMessage(error))" } }
    }
    func availableProviderConnection(provider: String, capability: String, endpoint: String, marketplace: String) async throws -> String? {
        guard companionURL == endpoint, configuration.audibleMarketplace == marketplace else { throw CancellationError() }
        guard supports("provider_connections_v1") else { return nil }
        let connections = try await refreshProviderRegistry(endpoint: endpoint, marketplace: marketplace)
        let matching = connections.filter { $0.providerId == provider && $0.status == "available" && $0.capabilities.contains(capability) }
        guard matching.count == 1 else { throw SetupError.message("The selected source must report one available \(capability) connection. Refresh capabilities or reconnect the source before importing.") }
        return matching[0].connectionId
    }
    func capabilityName(_ capability: String) -> String {
        ["provider_connections_v1": "Public source connection references", "provider.connections.list": "Source connection list", "kindle.catalog.import": "Kindle file import", "kindle.catalog": "Kindle catalog", "kindle.authorize": "Kindle account login", "audible.positions": "Audible positions", "audible.authorize": "Audible account login", "account.renew": "account renewal", "account.authorize.complete": "login completion", "account.authorize.cancel": "login cancellation", "account.disconnect": "account disconnection", "account.registration.reset": "unfinished registration recovery", "audible.positions.probe": "Audible access diagnostic"][capability] ?? "additional source capability"
    }
    func renewCompanionAccount(_ provider: String) async {
        testing = true; defer { testing = false }
        do {
            try await companion().renewAccount(provider: provider, marketplace: configuration.audibleMarketplace)
            status = "The source adapter confirmed \(provider.capitalized) account renewal. Refresh its data to check access."
            await refreshCompanion()
        } catch { if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } }
    }
    func startCompanionAccount(_ provider: String) async {
        testing = true; defer { testing = false }
        do {
            let endpoint = companionURL, marketplace = configuration.audibleMarketplace
            let authorization = try await LibraryCompanion(endpoint: endpoint).beginAuthorization(provider: provider, marketplace: marketplace)
            if authorization.sessionId != nil {
                let pending = PendingCompanionLogin(endpoint: endpoint, provider: provider, marketplace: marketplace, authorization: authorization)
                pendingLogin = pending
                try CustodySecureStorage().saveEncryptedMaterial(JSONEncoder().encode(pending), account: "pending-companion-login")
                returnedLoginURL = ""; registrationConsent = false
            }
            let url = try authorization.validatedURL()
            #if os(macOS)
            NSWorkspace.shared.open(url)
            #else
            await UIApplication.shared.open(url)
            #endif
            status = authorization.sessionId == nil ? "Finish account authorization in your browser, then refresh the companion." : "Complete browser login and MFA, then paste the final URL under Library companion. Account registration has not occurred yet."
        } catch { if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } }
    }
    func clearCompanionLogin() throws {
        try CustodySecureStorage().remove(account: "pending-companion-login")
        pendingLogin = nil; returnedLoginURL = ""; registrationConsent = false
    }
    func reopenCompanionLogin(_ authorization: CompanionAuthorization) async {
        do {
            let url = try authorization.validatedURL()
            #if os(macOS)
            NSWorkspace.shared.open(url)
            #else
            await UIApplication.shared.open(url)
            #endif
        } catch { model.error = bookNookErrorMessage(error) }
    }
    func completeCompanionLogin() async {
        guard let pending = pendingLogin, let session = pending.authorization.sessionId, registrationConsent else { return }
        testing = true; defer { testing = false }
        do {
            try await LibraryCompanion(endpoint: pending.endpoint).completeAuthorization(provider: pending.provider, marketplace: pending.marketplace, sessionId: session, returnURL: returnedLoginURL.trimmingCharacters(in: .whitespacesAndNewlines))
            returnedLoginURL = ""
            try clearCompanionLogin()
            status = "Audible account registered for this paired device. Read a position to validate source access, then review its edition before saving. No reading progress was inferred by login."
            await refreshCompanion()
        } catch { if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } }
    }
    func cancelCompanionLogin() async {
        guard let pending = pendingLogin, let session = pending.authorization.sessionId else { return }
        testing = true; defer { testing = false }
        do {
            try await LibraryCompanion(endpoint: pending.endpoint).cancelAuthorization(provider: pending.provider, marketplace: pending.marketplace, sessionId: session)
            try clearCompanionLogin()
            status = "Pending login cancelled."
        } catch { if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } }
    }
    func disconnectCompanionAccount() async {
        testing = true; defer { testing = false }
        do { try await companion().disconnectAccount(provider: "audible", marketplace: configuration.audibleMarketplace); status = "This paired device’s Audible registration was removed."; await updateProviderRegistryStatus() }
        catch { if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } }
    }
    func resetCompanionRegistration() async {
        testing = true; defer { testing = false }
        let endpoint = companionURL, marketplace = configuration.audibleMarketplace
        do {
            try await LibraryCompanion(endpoint: endpoint).resetUnfinishedRegistration(provider: "audible", marketplace: marketplace)
            if let pending = pendingLogin, pending.endpoint == endpoint, pending.marketplace == marketplace, pending.provider == "audible" { try clearCompanionLogin() }
            status = "The unfinished local registration was cleared after your Amazon-device review. You can start a new login. No remote removal was performed."
        } catch { if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } }
    }
    func probeCompanionAccount() async {
        testing = true; defer { testing = false }
        do { try await companion().probeAudibleAccount(marketplace: configuration.audibleMarketplace, asin: configuration.audibleTestAsin); status = "Audible returned authenticated JSON through the companion. Timestamp normalization is still unqualified, so no progress was saved." }
        catch { if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } }
    }
    func importCatalogFile(_ url: URL) async throws {
        guard !testing else { throw SetupError.message("Wait for the current companion request to finish.") }
        testing = true; defer { testing = false }
        let endpoint = companionURL, marketplace = configuration.audibleMarketplace
        let source = try LibraryCompanion(endpoint: endpoint)
        let refresh = try await catalogImport.run(file: url, source: source, model: model, connection: {
            try await availableProviderConnection(provider: "com.amazon.kindle", capability: "kindle.catalog", endpoint: endpoint, marketplace: marketplace)
        }, validateSource: {
            guard endpoint == companionURL, marketplace == configuration.audibleMarketplace else { throw CancellationError() }
        }) { status = $0 }
        // The captured source owns the operation; never apply its capabilities to a different URL.
        guard companionURL == endpoint else { return }
        companionCapabilities = refresh.capabilities
        companionQualified = refresh.qualified
        companionStatus = refresh.status
        await updateProviderRegistryStatus()
    }
    func importCompanionLibrary() async {
        testing = true; defer { testing = false }
        do {
            let endpoint = companionURL, marketplace = configuration.audibleMarketplace
            let source = try LibraryCompanion(endpoint: endpoint)
            let connection = try await availableProviderConnection(provider: "com.amazon.kindle", capability: "kindle.catalog", endpoint: endpoint, marketplace: marketplace)
            let descriptor = try await model.repository.sourceConnection(connection)
            let read = try await model.repository.fetchKindleCatalog(connection: descriptor) { try await source.kindleCatalog() }
            try Task.checkCancellation()
            guard endpoint == companionURL, marketplace == configuration.audibleMarketplace else { throw CancellationError() }
            model.accept(try await model.repository.commitCatalog(read))
            status = "Kindle catalog refreshed. Imported information remains private."
        }
        catch { if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } }
    }
    func lookupCompanionPosition() async {
        testing = true; defer { testing = false }
        do {
            let endpoint = companionURL, marketplace = configuration.audibleMarketplace, generation = providerRegistryGeneration
            let connection = try await availableProviderConnection(provider: "com.audible", capability: "audible.positions", endpoint: endpoint, marketplace: marketplace)
            let source = try LibraryCompanion(endpoint: endpoint)
            let descriptor = try await model.repository.sourceConnection(connection)
            let read = try await model.repository.fetchPositions(sourceIDs: [configuration.audibleTestAsin], connection: descriptor) { ids in
                try await source.audiblePositions(asins: ids, marketplace: marketplace)
            }
            try Task.checkCancellation()
            guard endpoint == companionURL, marketplace == configuration.audibleMarketplace, generation == providerRegistryGeneration else { throw CancellationError() }
            let observations = read.observations
            companionObservations = observations; companionObservationConnectionID = connection; companionObservationSource = read.source
            status = observations.map { "\($0.asin): \($0.positionMilliseconds / 1000) s, source observed \(Date(timeIntervalSince1970: TimeInterval($0.sourceUpdatedAt)).formatted()), \($0.freshness(now: UInt64(Date().timeIntervalSince1970))))" }.joined(separator: "; ")
            if observations.isEmpty { status = "No source-observed position available. No progress was inferred." }
        }
        catch { if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } }
    }
    func probeAudible() async {
        testing = true; defer { testing = false }
        do {
            let asin = configuration.audibleTestAsin
            guard asin.count == 10, asin.unicodeScalars.allSatisfy({ CharacterSet.alphanumerics.contains($0) && $0.isASCII }) else { throw SetupError.message("Enter a 10-character ASIN.") }
            let bytes = audibleHeaders.isEmpty ? try CustodySecureStorage().read(account: "audible-request-headers") : Data(audibleHeaders.utf8)
            guard let bytes else { throw SetupError.message("Configure account request headers first.") }
            let hosts = ["us": "api.audible.com", "uk": "api.audible.co.uk", "de": "api.audible.de", "ca": "api.audible.ca", "au": "api.audible.com.au"]
            guard let host = hosts[configuration.audibleMarketplace], let url = URL(string: "https://\(host)/1.0/annotations/lastpositions?asins=\(asin)") else { throw SetupError.message("Unsupported marketplace.") }
            var request = URLRequest(url: url); request.timeoutInterval = 20
            for (name, value) in try ConnectionSaveInputs.parseAudibleHeaders(bytes) { request.setValue(value, forHTTPHeaderField: name) }
            let sessionConfiguration = URLSessionConfiguration.ephemeral
            sessionConfiguration.timeoutIntervalForRequest = 20
            sessionConfiguration.timeoutIntervalForResource = 25
            let session = URLSession(configuration: sessionConfiguration, delegate: NoRedirects(), delegateQueue: nil); defer { session.invalidateAndCancel() }
            let (data, response) = try await session.data(for: request)
            let code = (response as? HTTPURLResponse)?.statusCode ?? 0
            guard (200...299).contains(code) else { throw SetupError.message("Audible returned HTTP \(code). Access may be expired or unsupported.") }
            guard data.count <= 4 * 1024 * 1024, (try? JSONSerialization.jsonObject(with: data)) != nil else { throw SetupError.message("Position response was not qualified JSON.") }
            status = "Audible returned JSON. Position field and timestamp qualification is still required; no position has been inferred."
        } catch { if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } }
    }
    func probeRelay() async {
        testing = true; defer { testing = false }
        do {
            guard let first = relays.split(whereSeparator: \.isNewline).first, let url = URL(string: String(first)), url.scheme == "wss", url.user == nil, url.password == nil else { throw SetupError.message("Enter a wss relay URL without credentials.") }
            let sessionConfiguration = URLSessionConfiguration.ephemeral
            sessionConfiguration.timeoutIntervalForRequest = 15
            sessionConfiguration.timeoutIntervalForResource = 20
            let session = URLSession(configuration: sessionConfiguration, delegate: NoRedirects(), delegateQueue: nil); defer { session.invalidateAndCancel() }
            let task = session.webSocketTask(with: url); task.resume(); defer { task.cancel(with: .normalClosure, reason: nil) }
            try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in task.sendPing { error in if let error { continuation.resume(throwing: error) } else { continuation.resume() } } }
            status = "Relay answered a WebSocket ping. Identity binding and publish permission still require Castalia."
        } catch { if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } }
    }
}
private enum SetupError: LocalizedError { case message(String); var errorDescription: String? { if case .message(let value) = self { return value }; return nil } }
private final class NoRedirects: NSObject, URLSessionTaskDelegate {
    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) { completionHandler(nil) }
}
