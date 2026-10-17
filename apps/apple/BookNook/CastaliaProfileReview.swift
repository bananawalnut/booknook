// SPDX-License-Identifier: MPL-2.0
import SwiftUI

struct CastaliaProfileReview: View {
    @Environment(\.dismiss) private var dismiss
    @StateObject private var session: CastaliaProfileImportSession
    @State private var error: String?
    let applied: @MainActor (ConnectionConfiguration) async -> Void

    @MainActor init(candidate: CastaliaProfileCandidate, repository: DeskRepository,
                    applied: @escaping @MainActor (ConnectionConfiguration) async -> Void) {
        _session = StateObject(wrappedValue: CastaliaProfileImportSession(candidate: candidate) { bytes, preview, holder in
            try await repository.installCastaliaProfile(bytes, preview: preview, holder: holder)
        })
        self.applied = applied
    }
    private var profile: CastaliaAuthorityProfile { session.candidate.preview.profile }
    private var changes: [(String, String, String)] {
        guard let previous = session.candidate.previousAuthority else { return [] }
        return [("Issuer", previous.issuer, profile.issuer), ("Issuer key ID", previous.keyId, profile.keyId),
            ("Issuer public key", previous.issuerPublicKey, profile.issuerPublicKey),
            ("Audience", previous.audience, profile.audience), ("Caller", previous.terminalCaller, profile.caller.id),
            ("Policy SHA-256", previous.policyDigest, profile.policyDigestSha256),
            ("Credential endpoint", previous.credentialEndpoint, profile.credentialEndpoint)].filter { $0.1 != $0.2 }
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Review your Castalia provider").font(.title2.bold())
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    Text(profile.displayName).font(.headline)
                    Text(session.candidate.filename).font(.caption).foregroundStyle(.secondary)
                    Text("This public profile contains provider settings. It contains no recovery secrets or membership credentials. Format validation does not prove that you trust the provider or that its service is available.")
                        .font(.callout).foregroundStyle(.secondary)
                    detail("Credential endpoint", profile.credentialEndpoint)
                    detail("Issuer", profile.issuer)
                    detail("Issuer key ID", profile.keyId)
                    detail("Issuer public key", profile.issuerPublicKey, monospaced: true)
                    detail("Audience", profile.audience)
                    detail("Caller", profile.caller.id)
                    detail("Policy SHA-256", profile.policyDigestSha256, monospaced: true)
                    if !changes.isEmpty {
                        Text("Changes from your saved provider").font(.headline)
                        ForEach(Array(changes.enumerated()), id: \.offset) { _, change in
                            VStack(alignment: .leading, spacing: 4) {
                                Text(change.0).font(.caption.bold())
                                Text("Was: \(change.1)").font(.caption.monospaced()).textSelection(.enabled)
                                Text("Becomes: \(change.2)").font(.caption.monospaced()).textSelection(.enabled)
                            }.fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    Divider()
                    Text("Connect an existing identity").font(.headline)
                    Text("Choose the public key of the Castalia identity you already use. Book Nook will use its encrypted recovery file on this device. Each publication or sync still needs your approval.").font(.callout)
                    if let current = session.candidate.configuredHolder {
                        Button("Use my currently configured identity") { session.holderPublicKey = current }
                        detail("Currently configured identity", current, monospaced: true)
                    }
                    if let verified = session.candidate.verifiedRecoveryHolder, verified != session.candidate.configuredHolder {
                        Button("Use the identity from my checked recovery file") { session.holderPublicKey = verified }
                        detail("Identity checked from recovery file", verified, monospaced: true)
                    }
                    if let unlocked = session.candidate.unlockedHolder, unlocked != session.candidate.configuredHolder, unlocked != session.candidate.verifiedRecoveryHolder {
                        Button("Use the identity unlocked on this device") { session.holderPublicKey = unlocked }
                        detail("Unlocked identity", unlocked, monospaced: true)
                    }
                    LabeledContent("Existing identity public key") {
                        TextField("64 lowercase hexadecimal characters", text: $session.holderPublicKey)
                            .textFieldStyle(.roundedBorder).font(.body.monospaced())
                            .accessibilityLabel("Existing Castalia identity public key")
                    }
                    if let current = session.candidate.configuredHolder, current != session.holderPublicKey {
                        Text("This changes the configured identity. Your current recovery file stays saved; it must match the chosen identity before it can unlock. Import a matching recovery file separately if needed.").font(.callout)
                    }
                    Text("Your saved recovery file, access tokens, library and other connection settings will be preserved. This replaces only the Castalia provider settings and selected holder key.").font(.caption).foregroundStyle(.secondary)
                    Toggle("I recognize this provider and want to use these settings", isOn: $session.recognizesProvider)
                    if let error { Text(error).foregroundStyle(.red).accessibilityLabel("Profile installation error: \(error)") }
                }.disabled(session.installing)
            }
            HStack {
                Button("Cancel", role: .cancel) { session.cancel(); dismiss() }.disabled(session.installing)
                Spacer()
                if session.installing { ProgressView("Saving provider…").controlSize(.small) }
                Button("Use provider for this identity") {
                    Task {
                        error = nil
                        do { let settings = try await session.apply(); await applied(settings); dismiss() }
                        catch is CancellationError { }
                        catch { self.error = bookNookErrorMessage(error) }
                    }
                }.buttonStyle(.borderedProminent).disabled(!session.canInstall)
            }
        }.padding(24).frame(minWidth: 360, idealWidth: 620, minHeight: 420, idealHeight: 700)
        .interactiveDismissDisabled(session.installing)
        .onDisappear { session.cancel() }
    }
    private func detail(_ title: String, _ value: String, monospaced: Bool = false) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).font(.caption).foregroundStyle(.secondary)
            Text(value).font(monospaced ? .caption.monospaced() : .callout)
                .textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                .accessibilityLabel("\(title): \(value)")
        }
    }
}
