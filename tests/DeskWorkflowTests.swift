// SPDX-License-Identifier: MPL-2.0
import AppKit
import Foundation

@main struct DeskWorkflowTests {
    @MainActor static func main() throws {
        let app = NSApplication.shared; app.setActivationPolicy(.prohibited)
        if CommandLine.arguments.contains("--reader-ink-anchor") {
            let harness = ReaderInkAnchorHarness(); try harness.run()
            withExtendedLifetime(harness) { app.run() }; return
        }
        Task { @MainActor in
            do {
                if CommandLine.arguments.contains("--ink-recognition") {
                    let root = FileManager.default.temporaryDirectory.appendingPathComponent("ink-recognition-" + UUID().uuidString)
                    defer { try? FileManager.default.removeItem(at: root) }
                    let model = DeskModel(repository: DeskRepository(directory: root)); await model.open()
                    try await testTranscriptionCancellation(model); try await testInkJournalAtomicRules(); exit(0)
                }
                if let index = CommandLine.arguments.firstIndex(of: "--ink-journal-child") {
                    try await runInkJournalCrashChild(directory: URL(fileURLWithPath: CommandLine.arguments[index + 1])); exit(0)
                }
                if let index = CommandLine.arguments.firstIndex(of: "--ink-journal-verify") {
                    try await verifyInkJournalCrashRecovery(directory: URL(fileURLWithPath: CommandLine.arguments[index + 1])); exit(0)
                }
                try await runWorkflows(); exit(0)
            }
            catch { fputs("Desk workflow failed: \(error)\n", stderr); exit(1) }
        }
        app.run()
    }
    @MainActor static func runWorkflows() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("booknook-ui-flow-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        setenv("BOOKNOOK_DATA_DIR", root.path, 1)
        let model = DeskModel()
        await model.open()
        precondition(model.error == nil && model.books.isEmpty)

        // The actual companion API and transport reject raw cache bytes before any
        // Keychain lookup, request transmission or remote service interaction.
        let cache = Data("SQLite format 3\0synthetic private cache".utf8)
        for endpoint in ["https://companion.example.invalid", "http://localhost:8787"] {
            do {
                try await LibraryCompanion(endpoint: endpoint).importKindleExport(cache)
                preconditionFailure("Raw cache upload to a nonliteral origin accepted")
            } catch { precondition(bookNookErrorMessage(error).contains("Raw Kindle SQLite")) }
        }
        var unsafeRequest = URLRequest(url: URL(string: "https://companion.example.invalid/v1/kindle/catalog")!)
        unsafeRequest.httpMethod = "PUT"; unsafeRequest.httpBody = cache
        do {
            _ = try await ConnectorHTTP.send(unsafeRequest, limit: 1024, localOnly: true)
            preconditionFailure("Local-only transport accepted a remote URL")
        } catch { precondition(bookNookErrorMessage(error).contains("Raw Kindle SQLite")) }
        print("Actual companion API and local-only HTTP transport reject raw SQLite for remote or hostname origins before requests or credential reads.")

        let first = try await model.perform(["op": "add_book", "title": "Same title", "author": "First author", "at": 1])
        let second = try await model.perform(["op": "add_book", "title": "Same title", "author": "Second author", "at": 2])
        let firstID = first!.object!["book_id"] as! String
        let secondID = second!.object!["book_id"] as! String
        precondition(firstID != secondID && model.books.count == 2)
        try await model.selectBook(secondID)
        precondition(model.selectedBookId == secondID && model.book?.authors == ["Second author"])
        let notebookID = model.selectedNotebookId
        do { try await model.selectBook("missing"); preconditionFailure("Missing book must fail") }
        catch { precondition(!bookNookErrorMessage(error).contains("NativeError.Failure")) }
        precondition(model.selectedBookId == secondID && model.selectedNotebookId == notebookID)

        // Explicit reading sessions survive an app-model reopen and end only by command.
        let passID = model.notebook!.passId
        let started = try await model.perform(["op": "start_session", "pass_id": passID, "at": 10])
        let sessionID = started!.object!["session_id"] as! String
        precondition(model.state!.sessions[sessionID]!.endedAt == nil)
        let sessionReopen = DeskModel(); await sessionReopen.open()
        precondition(sessionReopen.state!.sessions[sessionID]!.startedAt == 10)
        precondition(sessionReopen.state!.sessions[sessionID]!.endedAt == nil)
        do {
            try await model.perform(["op": "end_session", "session_id": sessionID, "at": 9])
            preconditionFailure("A negative reading interval must be rejected")
        } catch { precondition(model.state!.sessions[sessionID]!.endedAt == nil) }
        try await model.perform(["op": "end_session", "session_id": sessionID, "at": 20])
        precondition(model.state!.sessions[sessionID]!.endedAt == 20)

        // Prepare an earlier character version through the same native controller used
        // by its History button. Later private edits cannot broaden that saved selection.
        let notebook = model.notebook!
        try await model.perform(["op": "edit_block", "notebook_id": notebook.id,
            "block_id": notebook.blocks[0].id, "base_update": notebook.blocks[0].update,
            "text": "Surrounding notebook text must stay private", "at": 21])
        let newCard = try await model.perform(["op": "add_character", "notebook_id": notebook.id,
            "block_id": notebook.blocks[0].id, "name": "Mara", "at": 22])
        let cardID = newCard!.object!["card_id"] as! String
        let firstVersion = model.state!.cards[cardID]!.versions[0].id
        let editionID = model.state!.editions.values.first { $0.bookId == secondID }!.id
        let earlierInterpretation: [String: Any] = ["description": "Trustworthy at the café",
            "traits": ["Careful"], "relationships": [], "observations": ["Waits at the door"],
            "questions": ["Why return?"], "passages": [["kind": "text", "edition_id": editionID,
                "resource": "private-chapter.xhtml", "quote": "A selected supporting quote 日本語",
                "prefix": "Private preceding sentence", "suffix": "Private following sentence"]]]
        let earlier = try await model.perform(["op": "save_card", "card_id": cardID, "parents": [firstVersion],
            "interpretation": earlierInterpretation, "positions": [], "at": 23])
        let earlierID = earlier!.object!["version_id"] as! String
        var laterInterpretation = earlierInterpretation
        laterInterpretation["description"] = "Later private suspicion that must stay private"
        let later = try await model.perform(["op": "save_card", "card_id": cardID, "parents": [earlierID],
            "interpretation": laterInterpretation, "positions": [], "at": 24])
        let selectedDraft = try await model.prepareCharacterSharing(cardID: cardID, versionID: earlierID)
        precondition(selectedDraft.spoiler && selectedDraft.body.contains("Trustworthy at the café"))
        precondition(selectedDraft.body.contains("A selected supporting quote 日本語"))
        for privateText in ["Surrounding notebook", "Later private suspicion", "Private preceding", "Private following", "private-chapter.xhtml", earlierID, notebook.id, notebook.passId] {
            precondition(!selectedDraft.body.contains(privateText), "Unselected/private context must not enter the draft")
        }
        laterInterpretation["description"] = "Another later private change"
        try await model.perform(["op": "save_card", "card_id": cardID,
            "parents": [later!.object!["version_id"] as! String], "interpretation": laterInterpretation,
            "positions": [], "at": 25])
        precondition(model.state!.drafts[selectedDraft.id]!.body == selectedDraft.body)
        let draftCount = model.state!.drafts.count
        do {
            _ = try await model.prepareCharacterSharing(cardID: cardID, versionID: "missing-version")
            preconditionFailure("Missing selected version must fail without navigating or saving")
        } catch { precondition(model.state!.drafts.count == draftCount && model.section == "desk") }
        let unpublished = try await model.repository.socialSnapshot(reveal: false)
        precondition(unpublished.state.publications.isEmpty && unpublished.state.outbox.isEmpty)

        let revision = model.state!.revision
        do {
            try await model.perform(["op": "update_book", "book_id": secondID, "title": "Same title", "shelf": "read", "rating": 6, "review": "Keep this draft"])
            preconditionFailure("Invalid rating must throw to the editor")
        } catch { precondition(model.state!.revision == revision && model.book?.review == "") }
        try await model.perform(["op": "update_book", "book_id": secondID, "title": "Same title", "shelf": "read", "rating": NSNull(), "review": "A private review: café 🌿 日本語"])
        precondition(model.book?.rating == nil && model.book?.review == "A private review: café 🌿 日本語")

        let savedList = try await model.perform(["op": "save_list", "id": NSNull(), "title": "Weekend", "book_ids": [firstID], "expected_version": NSNull()])
        let listID = savedList!.object!["list_id"] as! String
        let version = model.state!.lists[listID]!.version
        try await model.perform(["op": "save_list", "id": listID, "title": "Weekend revised", "book_ids": [firstID, secondID], "expected_version": version])
        do {
            try await model.perform(["op": "save_list", "id": listID, "title": "Stale editor", "book_ids": [], "expected_version": version])
            preconditionFailure("Stale list editor must not dismiss as a successful save")
        } catch { precondition(model.state!.lists[listID]!.title == "Weekend revised") }

        // A local draft needs no configured issuer. Retrying through the actual controller
        // must preserve it when native preparation rejects missing Castalia authority.
        let draftID = UUID().uuidString.lowercased()
        let draftBody = "Keep this draft: café 🌿 日本語"
        let draft: [String: Any] = [
            "schema": "booknook.social-event.v1", "id": draftID,
            "actor": String(repeating: "11", count: 32),
            "at": UInt64(Date().timeIntervalSince1970), "resource_version": 0,
            "delivery": ["kind": "public"],
            "action": ["op": "publish", "group_id": NSNull(), "post": [
                "kind": "note", "book": NSNull(), "body": draftBody,
                "stars": NSNull(), "reading_status": NSNull(), "spoiler": false,
                "reply_to": NSNull()
            ]]
        ]
        try await model.repository.socialLocal(JSONSerialization.data(withJSONObject: ["op": "save_draft", "event": draft]))
        let social = SocialController()
        await social.reload(model)
        precondition(social.actor.isEmpty && social.snapshot!.state.drafts[draftID] != nil)
        await social.retryDraft(social.snapshot!.state.drafts[draftID]!, model)
        precondition(social.error != nil, "Missing authority must reject preparation")
        precondition(social.snapshot!.state.drafts[draftID] != nil, "Failed retry must retain the original durable draft")
        precondition(social.snapshot!.state.publications.isEmpty && social.snapshot!.state.outbox.isEmpty)

        let reopened = DeskModel(); await reopened.open()
        precondition(reopened.state!.books[secondID]!.review == "A private review: café 🌿 日本語")
        precondition(reopened.state!.lists[listID]!.bookIds.count == 2)
        precondition(reopened.state!.sessions[sessionID]!.endedAt == 20)
        precondition(reopened.state!.drafts[selectedDraft.id]!.body == selectedDraft.body)
        let restoredSocial = try await reopened.repository.socialSnapshot(reveal: false)
        let restoredPost = restoredSocial.state.drafts[draftID]?.action.object?["post"] as? [String: Any]
        precondition(restoredPost?["body"] as? String == draftBody, "Failed retry must remain recoverable after reopen")
        precondition(bookNookOperationWasCancelled(CocoaError(.userCancelled)))
        precondition(!bookNookOperationWasCancelled(CocoaError(.fileWriteNoPermission)))
        try await testNotebookInkWorkflow(model)
        try await testTranscriptionCancellation(model)
        try await testReaderInkWorkflow(model)
        try await testReaderContextWorkflow(model)
        try await testCompanionCatalogImportWorkflow()
        try await testCastaliaProfileImportWorkflow()
        try await testPublicProviderRepositoryWorkflow()
        print("Native desk workflows: duplicate-title identity, failed navigation, explicit sessions, private earlier-character-version drafts, thrown save rejection, unrated Unicode review, list edit/conflict retention, failed social retry with durable draft recovery, reopen and cancellation distinctions passed.")
    }
}
