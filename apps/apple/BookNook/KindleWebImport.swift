// SPDX-License-Identifier: MPL-2.0
import SwiftUI
import WebKit

@MainActor final class KindleWebImportWorkflow: ObservableObject {
    enum Phase { case idle, reading, saving, saved }
    @Published private(set) var phase = Phase.idle
    @Published private(set) var progress = ""
    var isRunning: Bool { phase == .reading || phase == .saving }
    typealias Persist = @MainActor (DeskModel, Data, String) async throws -> DeskSnapshot
    private let persist: Persist?
    init(persist: Persist? = nil) { self.persist = persist }
    func run(model: DeskModel, connection: ProviderConnection,
             limits: KindleWebCatalogReader.Limits = .init(),
             fetch: @escaping @MainActor @Sendable (URL) async throws -> Data) async throws -> KindleWebCatalogResult {
        guard !isRunning else { throw ConnectionInputError.message("Wait for the current catalog import to finish.") }
        guard connection.schema == "castalia.provider-connection.v1", connection.providerId == "com.amazon.kindle",
              connection.capabilities == ["kindle.catalog"], connection.status == "available", connection.credential == nil else { throw KindleWebError.signIn }
        phase = .reading; progress = "Reading Amazon catalog pages…"
        defer { if phase != .saved { phase = .idle } }
        var collected: KindleWebCatalogResult?
        let read = try await model.repository.fetchKindleCatalog(providerID: AppleProviderIDs.kindleWeb, connection: connection) {
            let result = try await KindleWebCatalogReader.collect(limits: limits, progress: { pages, titles in
                self.progress = "Read \(pages) catalog pages · \(titles) titles. Nothing has been imported yet."
            }, fetch: fetch)
            let catalog = try result.encodedCatalog()
            guard catalog.count <= 32 * 1024 * 1024 else { throw KindleWebError.limit }
            collected = result
            return catalog
        }
        guard let result = collected else { throw KindleWebError.limit }
        try Task.checkCancellation()
        phase = .saving; progress = "Saving catalog privately…"
        let snapshot: DeskSnapshot
        if let persist { snapshot = try await persist(model, read.kindleBytes, connection.connectionId) }
        else { snapshot = try await model.repository.commitCatalog(read) }
        // A commit already in flight can succeed despite cancellation. Publish
        // that durable result and never replace a newer visible revision.
        model.accept(snapshot)
        phase = .saved
        progress = "Imported \(result.catalog.items.count) titles privately from \(result.pageCount) catalog pages. The endpoint finished; this does not prove a complete purchase history. No book text or reading progress was imported."
        return result
    }
}

struct KindleWebImportSheet: View {
    @EnvironmentObject private var model: DeskModel
    @Environment(\.dismiss) private var dismiss
    @StateObject private var session = KindleWebSession()
    @StateObject private var workflow = KindleWebImportWorkflow()
    @State private var operation: Task<Void, Never>?
    @State private var error: String?
    let didImport: (String) -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Import Kindle catalog · United States").font(.title2)
            Text("Experimental connector. Sign in on Amazon below, then import titles and authors. This isolated session is discarded when you close it. Your password and cookies are not copied to Book Nook storage or the companion.").font(.callout)
            Text("US accounts only. Amazon compatibility has not been qualified with a live account. File imports remain available if this endpoint or sign-in flow is unavailable.").font(.caption).foregroundStyle(.secondary)
            Text(session.status).font(.callout).accessibilityIdentifier("kindle-web-session-status")
            KindleWebSurface(web: session.webView)
                .frame(minHeight: 240)
                .disabled(workflow.isRunning)
            if !workflow.progress.isEmpty { Text(workflow.progress).font(.callout).accessibilityIdentifier("kindle-web-import-progress") }
            if let error { Text(error).foregroundStyle(.red).font(.callout).accessibilityIdentifier("kindle-web-import-error") }
            HStack {
                Button(workflow.phase == .saved ? "Done" : (workflow.phase == .reading ? "Cancel import and close" : "Cancel")) {
                    operation?.cancel(); session.close(); dismiss()
                }.disabled(workflow.phase == .saving)
                Spacer()
                if workflow.isRunning { ProgressView().controlSize(.small) }
                Button("Import catalog") {
                    error = nil
                    operation = Task {
                        do {
                            _ = try await workflow.run(model: model, connection: session.descriptor, fetch: { url in try await session.page(url) })
                            didImport(workflow.progress)
                            session.close()
                        } catch {
                            if !bookNookOperationWasCancelled(error) { self.error = bookNookErrorMessage(error) }
                        }
                    }
                }.buttonStyle(.borderedProminent)
                    .disabled(!session.canReadCatalog || workflow.isRunning || workflow.phase == .saved)
            }
        }
        .padding(20)
        .frame(minWidth: 340, idealWidth: 780, maxWidth: .infinity, minHeight: 480, idealHeight: 720, maxHeight: .infinity)
        .interactiveDismissDisabled(workflow.isRunning)
        .onAppear { session.start() }
        .onDisappear { operation?.cancel(); session.close() }
    }
}
#if os(macOS)
private struct KindleWebSurface: NSViewRepresentable {
    let web: WKWebView
    func makeNSView(context: Context) -> WKWebView { web }
    func updateNSView(_ view: WKWebView, context: Context) {}
}
#else
private struct KindleWebSurface: UIViewRepresentable {
    let web: WKWebView
    func makeUIView(context: Context) -> WKWebView { web }
    func updateUIView(_ view: WKWebView, context: Context) {}
}
#endif
