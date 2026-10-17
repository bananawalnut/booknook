// SPDX-License-Identifier: MPL-2.0
import Foundation

extension DeskRepository {
    func importCatalog(providerID: String, sourceID: String) async throws -> DeskSnapshot {
        let raw = try await providerServices().importCatalog(providerID: providerID, sourceID: sourceID)
        return try providerDecode(Response.self, raw).state
    }
    func importCatalogFile(_ url: URL, format: String) async throws -> DeskSnapshot {
        guard format == "goodreads" || format == "kindle" else { throw BookNookServiceError.message("Unsupported catalog format.") }
        return try await importCatalog(providerID: format == "goodreads" ? AppleProviderIDs.goodreadsFile : AppleProviderIDs.kindleFile, sourceID: url.path)
    }
    func importContent(providerID: String, book: String, sourceID: String, mediaType: String) async throws -> DeskSnapshot {
        let raw = try await providerServices().importContent(providerID: providerID, bookID: book, sourceID: sourceID, mediaType: mediaType)
        return try providerDecode(DeskSnapshot.self, raw)
    }
    func registerProvider(_ provider: any BookNookProvider) async throws { try await providerServices().register(provider) }

    private func ensureJobProvider(id: String, namespace: String, capability: String, connection: ProviderConnection?) async throws {
        let connectionJSON: String?
        if let connection {
            let encoder = JSONEncoder(); encoder.keyEncodingStrategy = .convertToSnakeCase
            connectionJSON = String(decoding: try encoder.encode(connection), as: UTF8.self)
        } else { connectionJSON = nil }
        let descriptor = try BookNookProviderWire.descriptor(id: id, sourceNamespace: namespace,
            capabilities: capability == "catalog" && (connection == nil || connection?.capabilities.contains("kindle.positions") == true) ? ["catalog", "positions"] : [capability], connectionJSON: connectionJSON)
        let services = try providerServices()
        if try await services.registeredDescriptorJSON(providerID: id) == descriptor { return }
        let jobs = sourceJobs
        try await services.register(BookNookClosureProvider(descriptorJSON: descriptor) { request in
            try await jobs.execute(request)
        })
    }

    func sourceConnection(_ id: String?) throws -> ProviderConnection? {
        guard let id else { return nil }
        guard let connection = try connections().providerConnections.first(where: { $0.connectionId == id }) else {
            throw BookNookServiceError.message("The source connection changed. Refresh the source before importing.")
        }
        return connection
    }

    func fetchKindleCatalog(providerID: String = AppleProviderIDs.companionCatalog,
                            connection: ProviderConnection?,
                            fetch: @escaping AppleProviderJobs.CatalogFetch) async throws -> AppleCatalogRead {
        try await ensureJobProvider(id: providerID, namespace: "kindle", capability: "catalog", connection: connection)
        let key = await sourceJobs.stageCatalog(providerID: providerID, fetch: fetch)
        do {
            let read = try await providerServices().fetch(providerID: providerID,
                operationJSON: BookNookProviderWire.json(["kind": "catalog", "source_id": key]))
            await sourceJobs.removeCatalog(key)
            return AppleCatalogRead(source: read)
        } catch {
            await sourceJobs.removeCatalog(key)
            throw error
        }
    }

    func commitCatalog(_ read: AppleCatalogRead) async throws -> DeskSnapshot {
        let raw = try await providerServices().commitCatalog(read.source)
        return try providerDecode(Response.self, raw).state
    }

    func importKindleData(_ bytes: Data, connection: ProviderConnection? = nil) async throws -> DeskSnapshot {
        let read = try await fetchKindleCatalog(providerID: AppleProviderIDs.suppliedCatalog, connection: connection) { bytes }
        return try await commitCatalog(read)
    }

    func fetchPositions(providerID: String = AppleProviderIDs.audible, sourceIDs: [String],
                        connection: ProviderConnection?, fetch: @escaping AppleProviderJobs.PositionFetch) async throws -> ApplePositionRead {
        try await ensureJobProvider(id: providerID, namespace: "audible", capability: "positions", connection: connection)
        try await sourceJobs.stagePositions(providerID: providerID, sourceIDs: sourceIDs, fetch: fetch)
        let read: BookNookProviderRead
        do {
            read = try await providerServices().positions(providerID: providerID, sourceIDs: sourceIDs)
            await sourceJobs.removePositions(providerID)
        } catch {
            await sourceJobs.removePositions(providerID)
            throw error
        }
        struct Execution: Decodable {
            struct Response: Decodable {
                struct Result: Decodable { let observations: [CompanionPosition] }
                let result: Result
            }
            let response: Response
        }
        let execution = try providerDecode(Execution.self, read.executionJSON)
        return ApplePositionRead(source: read, observations: execution.response.result.observations)
    }
}

private func providerDecode<T: Decodable>(_ type: T.Type, _ raw: String) throws -> T {
    let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase
    return try decoder.decode(type, from: Data(raw.utf8))
}
