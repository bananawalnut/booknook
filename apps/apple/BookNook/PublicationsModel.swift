// SPDX-License-Identifier: MPL-2.0
import Foundation
import SwiftUI

struct PublicationArticle: Identifiable {
    let id: String
    let presentationID: UUID
    let sourceTitle: String
    let entry: BookNookFeedCacheEntryV1
    let provenance: BookNookFeedCacheProvenanceV1
    let rawSHA256: String
    let normalizerProfile: String
    let lastCheck: Int64
    let lastBodySuccess: Int64
}

@MainActor final class PublicationsModel: ObservableObject {
    let repository: PublicationRepository
    @Published private(set) var snapshot: PublicationSnapshot?
    @Published private(set) var entries: [PublicationIndexEntry] = []
    @Published private(set) var loadedDiscovery = false
    @Published private(set) var busy: String?
    @Published var error: String?
    @Published var message: String?
    @Published var importReview: PublicationImportPreview?
    @Published var editReview: PublicationEditPreview?
    @Published var exportReview: PublicationExportPreview?
    @Published var article: PublicationArticle?
    @Published private(set) var exportDelivery: PublicationExportDelivery?
    private var operation: Task<Void, Never>?
    private var presentationGeneration = UUID()
    init(repository: PublicationRepository) { self.repository = repository }
    var sources: [PublicationSubscription] { snapshot?.configuration.feeds.subscriptions ?? [] }
    var collections: [PublicationCollection] { snapshot?.configuration.feeds.collections ?? [] }

    // Swift Task cancellation immediately marks the SDK operation's storage/core
    // flags. Reconciliation runs in a fresh uncanceled task: cancellation is not
    // a promise that a guarded durable commit was rolled back.
    private func start(_ label: String, action: @escaping @MainActor () async throws -> Void) {
        guard operation == nil else { return }
        presentationGeneration = UUID()
        busy = label; error = nil; message = nil
        operation = Task { [weak self] in
            guard let self else { return }
            do { try await action() }
            catch {
                if Task.isCancelled || bookNookOperationWasCancelled(error) {
                    self.message = "Request canceled. The saved state below has been checked; a completed change may already be saved."
                } else { self.error = bookNookErrorMessage(error) }
            }
            let repository = self.repository
            let reconciliation = Task { try await repository.snapshot() }
            do {
                let value = try await reconciliation.value
                if let previous = self.snapshot, previous.token != value.token { self.entries = []; self.article = nil }
                self.snapshot = value
            }
            catch { self.error = "Could not verify saved publication settings: " + bookNookErrorMessage(error) }
            if self.loadedDiscovery {
                let generation = self.presentationGeneration
                let offline = Task { try await repository.discovery() }
                do {
                    let value = try await offline.value
                    if generation == self.presentationGeneration && !Task.isCancelled { self.entries = value }
                }
                catch { self.error = "Could not reopen saved articles: " + bookNookErrorMessage(error) }
            }
            self.busy = nil; self.operation = nil
        }
    }
    func cancelOperation() { presentationGeneration = UUID(); operation?.cancel() }
    private func read<T>(_ action: () async throws -> T) async throws -> T {
        try Task.checkCancellation()
        let generation = presentationGeneration
        let value = try await action()
        try Task.checkCancellation()
        guard presentationGeneration == generation else { throw CancellationError() }
        return value
    }
    private func beginReview() async {
        importReview = nil; editReview = nil; exportReview = nil
        await repository.cancelReviews()
    }
    func loadIfNeeded() { if snapshot == nil { reload() } }
    func reload() { start("Opening saved publications") {} }
    func loadSavedArticles() {
        start("Opening saved articles") { self.entries = try await self.read { try await self.repository.discovery() }; self.loadedDiscovery = true }
    }
    func add(title: String, url: String, format: String) {
        start("Reviewing publication") { await self.beginReview(); self.editReview = try await self.read { try await self.repository.reviewAdd(title: title, url: url, format: format) } }
    }
    func remove(_ id: String) { start("Reviewing removal") { await self.beginReview(); self.editReview = try await self.read { try await self.repository.reviewRemove(id) } } }
    func eligibility(_ source: PublicationSubscription) {
        start("Reviewing export eligibility") { await self.beginReview(); self.editReview = try await self.read { try await self.repository.reviewEligibility(source.id, selected: source.disclosure != "selected") } }
    }
    func enabledSetting(_ source: PublicationSubscription) {
        start("Reviewing enabled setting") { await self.beginReview(); self.editReview = try await self.read { try await self.repository.reviewEnabled(source.id, enabled: !source.enabled) } }
    }
    func acceptEdit() {
        guard let review = editReview else { return }
        start("Saving publication settings") {
            let changed = try await self.repository.confirmEdit(review.id)
            self.editReview = nil
            if changed { self.article = nil; self.entries = [] }
            self.message = changed ? "Settings saved. All sources are paused; use Enable & refresh for each source you want to contact." : "Settings already match. Existing activations were kept."
        }
    }
    func importFile(_ url: URL) {
        start("Reviewing imported settings") {
            await self.beginReview()
            let raw = try await readPublicationImport(url)
            try Task.checkCancellation()
            self.importReview = try await self.read { try await self.repository.reviewImport(raw) }
        }
    }
    func resolveImport(_ choices: [PublicationResolution]) {
        guard let review = importReview else { return }
        start("Reviewing conflict choices") { self.importReview = try await self.read { try await self.repository.resolveImport(review.id, choices: choices) } }
    }
    func acceptImport() {
        guard let review = importReview else { return }
        start("Saving imported settings") {
            let changed = try await self.repository.confirmImport(review.id)
            self.importReview = nil
            if changed { self.article = nil; self.entries = [] }
            self.message = changed ? "Settings imported. All sources are paused until you enable them." : "Settings already match. Existing activations were kept."
        }
    }
    func selectExport(_ selection: PublicationExportSelection) {
        start("Reviewing settings export") { await self.beginReview(); self.exportReview = try await self.read { try await self.repository.reviewExport(selection) } }
    }
    func acceptExport(presentationID: UUID) {
        guard let review = exportReview else { return }
        start("Preparing selected settings") {
            let data = try await self.read { try await self.repository.confirmExport(review.id) }
            self.exportDelivery = .init(id: UUID(), presentationID: presentationID, data: data); self.exportReview = nil
        }
    }
    @discardableResult func finishExport(_ id: UUID) -> Bool {
        guard exportDelivery?.id == id else { return false }
        exportDelivery = nil; return true
    }
    func cancelReview() {
        presentationGeneration = UUID()
        operation?.cancel()
        importReview = nil; editReview = nil; exportReview = nil
        Task { await repository.cancelReviews() }
    }
    func refresh(_ source: PublicationSubscription) {
        let active = snapshot?.statuses[source.id]?.locallyActive == true
        start(active ? "Refreshing publication" : "Enabling publication and refreshing") {
            try await self.repository.refresh(source.id, activate: !active)
            self.message = "Publication checked. Its text is saved for offline reading."
        }
    }
    func pause(_ id: String) { start("Pausing publication") { try await self.repository.pause(id); self.message = "Publication paused. Saved articles remain available offline." } }
    func openArticle(_ item: PublicationIndexEntry, presentationID: UUID = UUID()) {
        start("Opening saved article") {
            self.article = try await self.read {
                let view = try await self.repository.offline(item.sourceID)
                guard let entry = view.entries.first(where: { $0.variantKey == item.variantKey }) else { throw PublicationFailure.message("This saved article has changed. Reopen the saved articles list.") }
                return PublicationArticle(id: item.id, presentationID: presentationID, sourceTitle: item.sourceTitle, entry: entry, provenance: view.provenance,
                                          rawSHA256: view.rawSHA256, normalizerProfile: view.normalizerProfile, lastCheck: view.lastCheckUnixSeconds, lastBodySuccess: view.lastBodySuccessUnixSeconds)
            }
        }
    }
}

func publicationDateLabel(_ date: BookNookFeedCacheDateV1) -> String {
    publicationDateLabel(PublicationIndexDate(date))
}
func publicationDateLabel(_ date: PublicationIndexDate) -> String {
    switch date.status {
    case .missing: return "Undated · no publisher date"
    case .invalid: return "Invalid publisher date: " + String((date.raw ?? "").prefix(160))
    case .parsed: return "Published " + (date.raw ?? "")
    }
}
func publicationCheckLabel(_ seconds: Int64?) -> String {
    guard let seconds else { return "Never" }
    return Date(timeIntervalSince1970: TimeInterval(seconds)).formatted(date: .abbreviated, time: .shortened)
}
