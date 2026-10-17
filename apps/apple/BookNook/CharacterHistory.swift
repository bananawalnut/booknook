// SPDX-License-Identifier: MPL-2.0
import SwiftUI

struct CharacterHistoryView: View {
    @EnvironmentObject var model: DeskModel
    @Environment(\.dismiss) var dismiss
    let card: Card
    @StateObject private var shareCommit = DraftCommitState()
    @State private var preparingVersion: String?
    @State private var requestInFlight = false
    var name: String { model.state?.entities[card.characterId]?.name ?? "Character" }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("\(name), over time").font(.system(.title2, design: .serif))
            Text("Every saved interpretation is retained, including its supporting passages. Prepare one version as a private draft, then review its text and audience before approving any publication.").font(.callout).foregroundStyle(.secondary)
            ScrollView {
                VStack(alignment: .leading, spacing: 24) {
                    ForEach(Array(card.versions.enumerated().reversed()), id: \.element.id) { index, version in
                        VStack(alignment: .leading, spacing: 12) {
                            Text("Version \(index + 1) · \(Date(timeIntervalSince1970: Double(version.at)).formatted())").font(.headline)
                            Text("Saved by \(version.reader)").font(.caption).foregroundStyle(.secondary)
                            if let pass = model.state?.passes[version.passId] {
                                Text("Reading pass started \(Date(timeIntervalSince1970: Double(pass.startedAt)).formatted())").font(.caption).foregroundStyle(.secondary)
                            }
                            ForEach(characterHistoryFields(version.interpretation, nameForCharacter: { model.state?.entities[$0]?.name ?? "Character \($0)" })) { field in
                                VStack(alignment: .leading, spacing: 4) {
                                    Text(field.title).font(.caption.bold()).foregroundStyle(.secondary)
                                    ForEach(Array(field.values.enumerated()), id: \.offset) { _, value in Text(value).font(.callout) }
                                }
                            }
                            if !version.positions.isEmpty {
                                DisclosureGroup("Reading positions (\(version.positions.count))") {
                                    VStack(alignment: .leading, spacing: 8) {
                                        if let observations = version.positionObservations, !observations.isEmpty {
                                            ForEach(observations) { observation in
                                                VStack(alignment: .leading, spacing: 3) {
                                                    Text(readingPositionDescription(observation.location)).font(.caption)
                                                    Text("Observed \(Date(timeIntervalSince1970: Double(observation.observedAt)).formatted()) · \(readingAlignmentDescription(observation.alignment))").font(.caption2).foregroundStyle(.secondary)
                                                }
                                            }
                                        } else {
                                            ForEach(Array(version.positions.enumerated()), id: \.offset) { _, position in Text(readingPositionDescription(position)).font(.caption) }
                                        }
                                    }
                                }.font(.caption)
                            }
                            Button { prepare(version) } label: {
                                Label(shareCommit.isSaving && preparingVersion == version.id ? "Saving private draft…" : "Prepare this version for sharing", systemImage: "square.and.arrow.up")
                            }.disabled(requestInFlight)
                                .accessibilityLabel("Prepare \(name) version \(index + 1) as a private sharing draft")
                            if preparingVersion == version.id, let error = shareCommit.error { Text(error).font(.callout).foregroundStyle(.red) }
                            Divider()
                        }.textSelection(.enabled)
                    }
                }.frame(maxWidth: .infinity, alignment: .leading)
            }
            Text("The draft includes this version’s interpretation, relationship names, and supporting quote text. Other versions, notebook text, and private reading positions stay private.").font(.caption).foregroundStyle(.secondary)
            Button("Done") { dismiss() }.disabled(requestInFlight)
        }.padding(28).frame(minWidth: 380, idealWidth: 620, minHeight: 400, idealHeight: 640).interactiveDismissDisabled(requestInFlight)
    }

    private func prepare(_ version: CardVersion) {
        guard !requestInFlight else { return }
        preparingVersion = version.id; requestInFlight = true
        Task {
            defer { requestInFlight = false }
            var savedDraft: ShareDraft?
            await shareCommit.perform({
                savedDraft = try await model.prepareCharacterSharing(cardID: card.id, versionID: version.id)
            }, onSuccess: {
                guard let savedDraft else { return }
                model.sharingDraftId = savedDraft.id
                model.sharingBookId = nil
                dismiss()
                model.section = "community"
            })
        }
    }
}
