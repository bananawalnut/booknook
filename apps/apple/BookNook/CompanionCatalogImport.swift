// SPDX-License-Identifier: MPL-2.0
import Foundation
import Combine

@MainActor protocol CompanionCatalogSource {
    @discardableResult func importKindleExport(_ bytes: Data) async throws -> CompanionKindleImport.Format
    func kindleCatalog() async throws -> Data
    func manifest() async throws -> CompanionManifest
}
extension LibraryCompanion: CompanionCatalogSource {}

struct CompanionCatalogRefresh {
    let capabilities: [String]
    let qualified: Bool
    let status: String
}

/// The file picker and non-UI acceptance exercise this same import sequence.
/// A completed local commit remains successful if the later capability read fails.
@MainActor final class CompanionCatalogImportWorkflow: ObservableObject {
    typealias ReadFile = @MainActor (URL) async throws -> Data
    typealias Persist = @MainActor (DeskModel, Data) async throws -> DeskSnapshot
    @Published private(set) var isRunning = false
    private let readFile: ReadFile
    private let persist: Persist?

    init(readFile: ReadFile? = nil, persist: Persist? = nil) {
        self.readFile = readFile ?? { try await CompanionKindleImport.readFile($0) }
        self.persist = persist
    }

    static func persistCatalog(_ catalog: Data, model: DeskModel) async throws -> DeskSnapshot {
        try await model.repository.importKindleData(catalog)
    }

    func run(file: URL, source: any CompanionCatalogSource, model: DeskModel,
             connection: @MainActor () async throws -> String? = { nil },
             validateSource: @MainActor () throws -> Void = {},
             didImport: (String) -> Void = { _ in }) async throws -> CompanionCatalogRefresh {
        guard !isRunning else { throw ConnectionInputError.message("Wait for the current companion request to finish.") }
        isRunning = true
        defer { isRunning = false }
        try Task.checkCancellation()
        let bytes = try await readFile(file)
        try Task.checkCancellation()
        let format = try await source.importKindleExport(bytes)
        try Task.checkCancellation()
        let connectionID = try await connection()
        let descriptor = try await model.repository.sourceConnection(connectionID)
        let read = try await model.repository.fetchKindleCatalog(connection: descriptor) { try await source.kindleCatalog() }
        // A source may finish even after cancellation. Do not begin a local commit then.
        try Task.checkCancellation()
        try validateSource()
        let snapshot: DeskSnapshot
        if let persist, connectionID == nil { snapshot = try await persist(model, read.kindleBytes) }
        else { snapshot = try await model.repository.commitCatalog(read) }
        // Once committed, publish its state even if cancellation arrived during persistence.
        // Other edits may have completed while its response was in flight.
        model.accept(snapshot)
        didImport(format == .sqliteSnapshot
            ? "Cached Kindle titles and authors imported privately. No reading progress or book content came from the snapshot."
            : "Kindle catalog imported. Imported information remains private.")
        do {
            try Task.checkCancellation()
            let manifest = try await source.manifest()
            try Task.checkCancellation()
            return CompanionCatalogRefresh(capabilities: manifest.capabilities, qualified: manifest.qualified,
                                           status: "Connected to \(manifest.name).")
        } catch {
            var message = "Your catalog was imported. Choose Refresh capabilities to update available companion actions."
            if !bookNookOperationWasCancelled(error) { message += " \(bookNookErrorMessage(error))" }
            return CompanionCatalogRefresh(capabilities: [], qualified: false, status: message)
        }
    }
}
