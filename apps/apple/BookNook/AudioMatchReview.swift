// SPDX-License-Identifier: MPL-2.0
import SwiftUI

struct AudioEditionCandidate: Decodable, Identifiable {
    let bookId: String; let editionId: String; let editionLabel: String; let title: String; let authors: [String]
    let confidence: String; let reasons: [String]; let warnings: [String]
    var id: String { editionId }
}
struct AudioObservationSelection: Identifiable {
    let id = UUID()
    let observation: CompanionPosition
    let connectionID: String?
    var sourceRead: BookNookProviderRead? = nil
}
extension DeskRepository {
    func audioCandidates(_ observation: CompanionPosition) throws -> [AudioEditionCandidate] {
        let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode([AudioEditionCandidate].self, from: Data(required().audioEditionCandidates(observationJson: observation.wireJSON()).utf8))
    }
    func attachAudio(_ observation: CompanionPosition, pass: String, edition: String?, connection: String? = nil,
                     sourceRead: BookNookProviderRead? = nil) async throws -> DeskSnapshot {
        let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase
        let read: BookNookProviderRead
        if let sourceRead { read = sourceRead }
        else {
            let descriptor = try sourceConnection(connection)
            read = try await fetchPositions(sourceIDs: [observation.asin], connection: descriptor) { _ in [observation] }.source
        }
        let result = try await providerServices().attachAudio(read, observationJSON: observation.wireJSON(), passID: pass, confirmedEditionID: edition)
        return try decoder.decode(Response.self, from: Data(result.utf8)).state
    }

}

struct AudioMatchReview: View {
    @EnvironmentObject var model: DeskModel
    @Environment(\.dismiss) private var dismiss
    let observation: CompanionPosition
    let connectionID: String?
    let sourceRead: BookNookProviderRead?
    @State private var candidates: [AudioEditionCandidate] = []
    @State private var selectedEdition = ""
    @State private var selectedPass = ""
    @State private var busy = false
    @State private var error: String?
    init(observation: CompanionPosition, connectionID: String? = nil, sourceRead: BookNookProviderRead? = nil) {
        self.observation = observation; self.connectionID = connectionID; self.sourceRead = sourceRead
    }
    private var candidate: AudioEditionCandidate? { candidates.first { $0.editionId == selectedEdition } }
    private var passes: [ReadingPass] {
        (model.state?.passes.values.filter { candidate == nil || $0.bookId == candidate?.bookId } ?? []).sorted { $0.startedAt > $1.startedAt }
    }
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text("Review listening position").font(.title2.bold())
                Text(observation.metadata?.title ?? observation.asin).font(.headline)
                Text("Audible \(observation.asin) · \(observation.positionMilliseconds / 1000) seconds").font(.callout)
                Text("Observed \(Date(timeIntervalSince1970: TimeInterval(observation.sourceUpdatedAt)).formatted()). \(observation.freshness(now: UInt64(Date().timeIntervalSince1970)))").font(.caption).foregroundStyle(.secondary)
                if let connectionID { Text("Source connection: \(connectionID)").font(.caption.monospaced()).textSelection(.enabled) }
                Text("Suggestions use source metadata and your library. Review the edition, language, and whether the audiobook is abridged. Confirming an association never moves the ebook or converts audio time to text.").font(.callout)
                Picker("Ebook edition", selection: $selectedEdition) {
                    Text("Keep this position unmatched").tag("")
                    ForEach(candidates) { item in
                        Text("\(item.title) — \(item.authors.joined(separator: ", ")) · \(item.editionLabel)").tag(item.editionId)
                    }
                }.disabled(busy)
                if candidates.isEmpty {
                    Text("No supported edition match was found. You can still save this source position to a reading pass without claiming an edition match.").font(.callout).foregroundStyle(.secondary)
                }
                if let candidate {
                    ForEach(candidate.reasons, id: \.self) { Text($0).font(.callout) }
                    ForEach(candidate.warnings, id: \.self) { Text($0).font(.caption).foregroundStyle(.secondary) }
                }
                Picker("Reading pass", selection: $selectedPass) {
                    Text("Choose a reading pass").tag("")
                    ForEach(passes) { pass in
                        Text("\(model.state?.books[pass.bookId]?.title ?? pass.bookId) · started \(Date(timeIntervalSince1970: TimeInterval(pass.startedAt)).formatted())").tag(pass.id)
                    }
                }.disabled(busy)
                if passes.isEmpty { Text("Open the book in your library first to start a reading pass, then return to this observation.").font(.callout).foregroundStyle(.secondary) }
                if let error { Text(error).foregroundStyle(.red) }
                HStack {
                    Button("Cancel", role: .cancel) { dismiss() }.disabled(busy)
                    Spacer()
                    Button(candidate == nil ? "Save unmatched position" : "Confirm association and save") {
                        busy = true; error = nil
                        let pass = selectedPass
                        let edition = selectedEdition.isEmpty ? nil : selectedEdition
                        Task {
                            defer { busy = false }
                            do {
                                model.accept(try await model.repository.attachAudio(observation, pass: pass, edition: edition, connection: connectionID, sourceRead: sourceRead))
                                dismiss()
                            } catch { if !bookNookOperationWasCancelled(error) { self.error = bookNookErrorMessage(error) } }
                        }
                    }.buttonStyle(.borderedProminent).disabled(busy || selectedPass.isEmpty || !passes.contains(where: { $0.id == selectedPass }))
                }
            }.padding(28)
        }.frame(minWidth: 360, idealWidth: 640)
        .interactiveDismissDisabled(busy)
        .onChange(of: selectedEdition) { _, _ in if !passes.contains(where: { $0.id == selectedPass }) { selectedPass = "" } }
        .task {
            do { candidates = try await model.repository.audioCandidates(observation) }
            catch { self.error = bookNookErrorMessage(error) }
        }
    }
}
