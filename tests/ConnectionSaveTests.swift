// SPDX-License-Identifier: MPL-2.0
import Foundation

@main struct ConnectionSaveTests {
    @MainActor static func main() async throws {
        let headers = #"{"authorization":"synthetic test access"}"#
        let endpoint = "https://issuer.example.invalid/credentials"
        let plan = try ConnectionSaveInputs(audibleHeaders: headers, issuerToken: "synthetic-token", issuerEndpoint: endpoint)
        var writes: [String] = []
        func fails(_ action: () throws -> Void) {
            do { try action(); preconditionFailure("Expected input rejection") } catch { }
        }
        let otherEndpoint = "https://other-issuer.example.invalid/credentials"
        var draft = IssuerAccessDraft()
        draft.update("synthetic-token", endpoint: endpoint)
        let entered = draft
        let sameDestinationToken = try draft.token(for: endpoint)
        precondition(sameDestinationToken == "synthetic-token")
        precondition(!draft.clearIfDestinationChanged(to: endpoint) && draft == entered)
        // A save before SwiftUI's endpoint onChange must still reject relocation.
        fails { _ = try draft.token(for: otherEndpoint) }
        draft.update("synthetic-token-edited", endpoint: otherEndpoint)
        fails { _ = try draft.token(for: otherEndpoint) }
        precondition(draft.endpoint == endpoint, "Editing a retained draft must not rebind its destination")
        precondition(draft.clearIfDestinationChanged(to: otherEndpoint))
        precondition(draft.value.isEmpty && draft.endpoint == nil)
        draft.update("new-provider-token", endpoint: otherEndpoint)
        let newToken = try draft.token(for: otherEndpoint)
        precondition(newToken == "new-provider-token")
        fails { _ = try draft.token(for: endpoint) }
        draft.clear()
        precondition(draft.value.isEmpty && draft.endpoint == nil)
        fails { _ = try ConnectionSaveInputs(audibleHeaders: headers, issuerToken: "invalid\ntoken", issuerEndpoint: endpoint) }
        fails { _ = try ConnectionSaveInputs(audibleHeaders: "not JSON", issuerToken: "synthetic", issuerEndpoint: endpoint) }
        fails { _ = try ConnectionSaveInputs(audibleHeaders: #"{"cookie":"synthetic"}"#, issuerToken: "", issuerEndpoint: endpoint) }
        fails { _ = try ConnectionSaveInputs(audibleHeaders: #"{"authorization":"value\nline"}"#, issuerToken: "", issuerEndpoint: endpoint) }
        fails { _ = try ConnectionSaveInputs(audibleHeaders: "", issuerToken: String(repeating: "a", count: 16_385), issuerEndpoint: endpoint) }
        for url in ["http://issuer.example.invalid/credentials", "https://name:secret@issuer.example.invalid", "https://issuer.example.invalid?token=synthetic", "https://issuer.example.invalid/#fragment"] {
            fails { _ = try ConnectionSaveInputs(audibleHeaders: "", issuerToken: "synthetic", issuerEndpoint: url) }
        }
        precondition(writes.isEmpty)
        let empty = try ConnectionSaveInputs(audibleHeaders: "", issuerToken: "", issuerEndpoint: "")
        let noSecrets = try await saveConnectionInputs(empty, saveSettings: { writes.append("settings") }, saveAudible: { _ in preconditionFailure() }, saveIssuer: { _, _ in preconditionFailure() })
        precondition(writes == ["settings"] && noSecrets.error == nil)

        writes = []
        do {
            _ = try await saveConnectionInputs(plan, saveSettings: { throw ConnectionInputError.message("Synthetic settings failure") }, saveAudible: { _ in writes.append("audible") }, saveIssuer: { _, _ in writes.append("issuer") })
            preconditionFailure("Settings failure accepted")
        } catch { }
        precondition(writes.isEmpty)

        let firstFailure = try await saveConnectionInputs(plan, saveSettings: { writes.append("settings") }, saveAudible: { _ in throw ConnectionInputError.message("Synthetic secure-store failure") }, saveIssuer: { _, _ in writes.append("issuer") })
        precondition(writes == ["settings"] && firstFailure.failedAccess == "Audible access" && !firstFailure.audibleAccessSaved && !firstFailure.issuerAccessSaved)

        writes = []
        let partial = try await saveConnectionInputs(plan, saveSettings: { writes.append("settings") }, saveAudible: { _ in writes.append("audible") }, saveIssuer: { _, _ in throw ConnectionInputError.message("Synthetic secure-store failure") })
        precondition(writes == ["settings", "audible"] && partial.audibleAccessSaved && !partial.issuerAccessSaved && partial.failedAccess == "Issuer access" && partial.error != nil)

        writes = []
        let complete = try await saveConnectionInputs(plan, saveSettings: { writes.append("settings") }, saveAudible: { data in precondition(data == Data(headers.utf8)); writes.append("audible") }, saveIssuer: { data, url in precondition(data == Data("synthetic-token".utf8) && url == endpoint); writes.append("issuer") })
        precondition(writes == ["settings", "audible", "issuer"] && complete.audibleAccessSaved && complete.issuerAccessSaved && complete.error == nil)
        let unavailable = ConnectorFailure.message(status: 503, body: Data(#"{"code":"source_not_configured","error":"private account token"}"#.utf8))
        precondition(unavailable.contains("no live") && !unavailable.contains("private account"))
        precondition(!ConnectorFailure.message(status: 500, body: Data(#"{"error":"private account token"}"#.utf8)).contains("private account"))
        precondition(ConnectorFailure.message(status: 503, body: Data(repeating: 120, count: 20_000)).contains("unavailable"))
        let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase
        let oldObservation = Data(#"{"asin":"B012345678","position_milliseconds":12345,"source_updated_at":100,"fetched_at":200,"duration_milliseconds":50000,"edition_identifier":null,"alignment":"unmatched"}"#.utf8)
        let oldPosition = try decoder.decode(CompanionPosition.self, from: oldObservation)
        precondition(oldPosition.metadata == nil && oldPosition.freshness(now: 2000).contains("stale"))
        var richer = try JSONSerialization.jsonObject(with: oldObservation) as! [String: Any]
        richer["metadata"] = ["title":"Café", "authors":["Writer"], "ebook_identifiers":[["provider":"kindle","id":"B098765432"]], "language":"en"]
        let position = try decoder.decode(CompanionPosition.self, from: JSONSerialization.data(withJSONObject: richer))
        precondition(position.metadata?.isValid == true)
        let roundTrip = try JSONSerialization.jsonObject(with: Data(position.wireJSON().utf8)) as! [String: Any]
        precondition(NSDictionary(dictionary: roundTrip["metadata"] as! [String: Any]) == NSDictionary(dictionary: richer["metadata"] as! [String: Any]))
        let sqlite = Data("SQLite format 3\0synthetic private cache bytes".utf8)
        let remote = URL(string: "https://companion.example.invalid")!
        for local in ["http://127.0.0.1:8787", "https://127.0.0.1:9443/prefix", "http://[::1]:8787", "https://[::1]"] {
            let format = try CompanionKindleImport.validate(sqlite, endpoint: URL(string: local)!)
            precondition(format == .sqliteSnapshot)
        }
        for address in ["https://companion.example.invalid", "https://127.0.0.1.example.invalid", "https://localhost.example.invalid", "http://localhost:8787", "https://LOCALHOST", "https://127.1", "https://2130706433", "https://[::ffff:127.0.0.1]", "https://[::2]", "https://127.0.0.1@remote.example.invalid", "https://remote@127.0.0.1", "https://127.0.0.1?target=remote", "https://127.0.0.1#fragment", "https://127.0.0.1:0"] {
            fails { _ = try CompanionKindleImport.validate(sqlite, endpoint: URL(string: address)!) }
        }
        let catalogJSON = Data(#"{"schema":"booknook.kindle-catalog.v1","items":[],"note":"SQLite format 3"}"#.utf8)
        let catalogXML = Data("<?xml version=\"1.0\"?><meta_data/>".utf8)
        for endpoint in [remote, URL(string: "http://localhost:8787")!, URL(string: "http://[::1]:8787")!] {
            let jsonFormat = try CompanionKindleImport.validate(catalogJSON, endpoint: endpoint)
            let xmlFormat = try CompanionKindleImport.validate(catalogXML, endpoint: endpoint)
            precondition(jsonFormat == .jsonCatalog && xmlFormat == .xmlCatalog)
            let utf16XML = "<?xml version=\"1.0\" encoding=\"UTF-16\"?><meta_data/>".data(using: .utf16)!
            let utf16Format = try CompanionKindleImport.validate(utf16XML, endpoint: endpoint)
            precondition(utf16Format == .xmlCatalog)
            let utf32XML = "<?xml version=\"1.0\" encoding=\"UTF-32\"?><meta_data/>"
            for bytes in [Data([0xFF, 0xFE, 0, 0]) + utf32XML.data(using: .utf32LittleEndian)!,
                          Data([0, 0, 0xFE, 0xFF]) + utf32XML.data(using: .utf32BigEndian)!] {
                let utf32Format = try CompanionKindleImport.validate(bytes, endpoint: endpoint)
                precondition(utf32Format == .xmlCatalog)
            }
        }
        fails { _ = try CompanionKindleImport.validate(Data([0, 1, 2, 3]), endpoint: remote) }
        fails { _ = try CompanionKindleImport.validate(Data(), endpoint: remote) }
        let configuration = URLSessionConfiguration.ephemeral
        configuration.connectionProxyDictionary = ["HTTPEnable": 1, "HTTPProxy": "synthetic.invalid", "ProxyAutoConfigEnable": 1]
        fails { try CompanionKindleImport.configureLocalTransport(configuration, endpoint: remote) }
        precondition(configuration.connectionProxyDictionary?["HTTPEnable"] as? Int == 1)
        try CompanionKindleImport.configureLocalTransport(configuration, endpoint: URL(string: "http://127.0.0.1")!)
        precondition(configuration.connectionProxyDictionary?.count == 5)
        precondition(configuration.connectionProxyDictionary?.values.allSatisfy { ($0 as? Int) == 0 } == true)
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("booknook-kindle-file-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let disguisedSnapshot = directory.appendingPathComponent("catalog.json")
        try sqlite.write(to: disguisedSnapshot)
        let selectedBytes = try await CompanionKindleImport.readFile(disguisedSnapshot)
        precondition(selectedBytes == sqlite)
        fails { _ = try CompanionKindleImport.validate(selectedBytes, endpoint: remote) }
        let largeFile = directory.appendingPathComponent("oversized.sqlite")
        _ = FileManager.default.createFile(atPath: largeFile.path, contents: nil)
        let handle = try FileHandle(forWritingTo: largeFile)
        try handle.truncate(atOffset: UInt64(CompanionKindleImport.maximumBytes + 1)); try handle.close()
        do { _ = try await CompanionKindleImport.readFile(largeFile); preconditionFailure("Oversized file accepted") } catch { }
        do { _ = try await CompanionKindleImport.readFile(directory); preconditionFailure("Directory accepted") } catch { }
        let gate = AsyncStream<Void>.makeStream()
        let cancelledRead = Task {
            for await _ in gate.stream { }
            return try await CompanionKindleImport.readFile(disguisedSnapshot)
        }
        cancelledRead.cancel(); gate.continuation.finish()
        do { _ = try await cancelledRead.value; preconditionFailure("Cancelled file read accepted") }
        catch is CancellationError { }
        let snapshotErrors = ["kindle_snapshot_wal": "backup", "kindle_snapshot_schema": "schema", "kindle_snapshot_invalid": "snapshot", "kindle_snapshot_runtime": "runtime", "kindle_snapshot_local_only": "loopback"]
        for (code, instruction) in snapshotErrors {
            let body = try JSONSerialization.data(withJSONObject: ["code": code, "error": "private database field"])
            let message = ConnectorFailure.message(status: 400, body: body)
            precondition(message.contains(instruction) && !message.contains("private database"))
        }
        print("Connection save: all input validated before writes; settings failure, first/second secure-store failure, explicit partial result, empty inputs, and ordered successful save, bounded non-secret errors, and optional audio-metadata compatibility passed (in-memory callbacks; no live Keychain).")
        print("Issuer access draft: exact destination captured during entry, unchanged destination retained, destination changes cleared, and save before deferred UI notification rejects token relocation.")
        print("Kindle file policy: SQLite magic independent of extension, literal IPv4/IPv6 only, lookalike/hostname/user-info/query/fragment rejection, remote JSON/XML including BOM UTF-16 and both UTF-32 endians, per-session proxy override, bounded background file reads and caller cancellation passed.")
    }
}
