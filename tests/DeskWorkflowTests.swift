// SPDX-License-Identifier: MPL-2.0
import AppKit
import CryptoKit
import Combine
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
                if CommandLine.arguments.contains("--group-replies") { try await testGroupReplies(); exit(0) }
                if CommandLine.arguments.contains("--shared-list-review") { try await testSharedListReview(); exit(0) }
                if CommandLine.arguments.contains("--reading-pass-navigation") { try await testReadingPassNavigation(); exit(0) }
                if CommandLine.arguments.contains("--recommendation-titles") { try await testRecommendationTitleProjection(); exit(0) }
                if CommandLine.arguments.contains("--startup") { try await testDeskStartup(); exit(0) }
                if CommandLine.arguments.contains("--publications") { try await testPublicationWorkflow(); exit(0) }
                if CommandLine.arguments.contains("--source-reading-history") { try await testSourceReadingHistory(); exit(0) }
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
    @MainActor static func testReadingPassNavigation() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("booknook-pass-navigation-" + UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let model = DeskModel(repository: DeskRepository(directory: root))
        await model.open()
        func addBook(_ title: String) async throws -> String {
            let result = try await model.perform(["op": "add_book", "title": title, "author": "Fixture", "at": 1])
            guard let id = result?.object?["book_id"] as? String else { throw CocoaError(.coderReadCorrupt) }
            return id
        }
        func intent() throws -> DeskModel.ReadingPassIntent {
            guard let intent = model.captureReadingPassIntent() else { throw CocoaError(.coderReadCorrupt) }
            return intent
        }
        func expectCancellation(_ action: DeskModel.ReadingPassAction, intent: DeskModel.ReadingPassIntent) async throws {
            do {
                try await model.changeReadingPass(action, intent: intent)
                preconditionFailure("A stale reading-pass request must be canceled")
            } catch is CancellationError {}
        }
        let first = try await addBook("First pass fixture"), second = try await addBook("Second pass fixture")
        try await model.selectBook(second)
        let secondNotebook = model.notebook!
        try await model.selectBook(first)
        let firstNotebook = model.notebook!
        let ink = model.inkSession(notebook: firstNotebook, block: firstNotebook.blocks[0])
        defer { ink.capturePendingStroke = nil }

        let native = try Desk(directory: root.path)
        var captures = 0
        var competingNotebookID: String?
        var competingFailure: Error?
        // Another real store handle commits during the final ink checkpoint,
        // after the pending operation has received its flushed desk snapshot.
        ink.capturePendingStroke = {
            captures += 1
            guard captures == 2 else { return }
            do {
                let data = try JSONSerialization.data(withJSONObject: ["op": "start_pass", "book_id": first, "at": 41])
                let json = try native.execute(commandJson: String(decoding: data, as: UTF8.self))
                let response = try JSONDecoder().decode(JSONValue.self, from: Data(json.utf8)).object
                competingNotebookID = (response?["result"] as? [String: Any])?["id"] as? String
            } catch { competingFailure = error }
        }
        try await model.changeReadingPass(.begin(at: 42), intent: intent())
        ink.capturePendingStroke = nil
        if let competingFailure { throw competingFailure }
        guard let created = model.notebook, let competingNotebookID else { throw CocoaError(.coderReadCorrupt) }
        precondition(captures == 2 && model.state?.notebooks[competingNotebookID]?.bookId == first)
        precondition(model.state?.passes[created.passId]?.startedAt == 42 && created.id != competingNotebookID,
                     "Select the exact notebook returned by the command, even with a concurrent pass on the same book")
        let currentGeneration = model.editorGeneration

        ink.capturePendingStroke = {
            model.selectedBookId = second; model.selectedNotebookId = secondNotebook.id
            model.selectedBookId = first; model.selectedNotebookId = created.id
            ink.capturePendingStroke = nil
        }
        try await expectCancellation(.select(firstNotebook.id), intent: intent())
        precondition(model.selectedNotebookId == created.id && model.editorGeneration == currentGeneration,
                     "A round trip to the same book and pass must invalidate a stale selection")

        let before = model.state!.notebooks.count
        ink.capturePendingStroke = { model.section = "library"; ink.capturePendingStroke = nil }
        try await expectCancellation(.begin(at: 43), intent: intent())
        precondition(model.state?.notebooks.count == before && model.selectedNotebookId == created.id,
                     "Leaving before command dispatch must not create or select a pass")
        model.section = "desk"

        var lateNotebookID: String?
        let observation = model.$state.sink { snapshot in
            guard lateNotebookID == nil, let snapshot,
                  let notebook = snapshot.notebooks.values.first(where: { snapshot.passes[$0.passId]?.startedAt == 44 }) else { return }
            lateNotebookID = notebook.id; model.section = "library"
        }
        defer { observation.cancel() }
        try await expectCancellation(.begin(at: 44), intent: intent())
        precondition(lateNotebookID != nil && model.selectedNotebookId == created.id &&
                     model.editorGeneration == currentGeneration && model.section == "library")
        let reopened = try await DeskRepository(directory: root).open().0
        precondition(reopened.notebooks[lateNotebookID!]?.bookId == first,
                     "After commit a stale request keeps its pass in SQLite without changing the active desk")

        model.section = "desk"
        do {
            try await model.changeReadingPass(.select(secondNotebook.id), intent: intent())
            preconditionFailure("A different book's pass must not become the active notebook")
        } catch let error as BookNookServiceError {
            precondition(bookNookErrorMessage(error).contains("current book"))
        }
        precondition(model.selectedBookId == first && model.selectedNotebookId == created.id)
        try await model.changeReadingPass(.select(firstNotebook.id), intent: intent())
        precondition(model.selectedNotebookId == firstNotebook.id && model.editorGeneration != currentGeneration)
        print("Native reading passes selected the exact command result with a concurrent pass and preserved navigation intent across canceled, committed, and reopened operations.")
    }
    @MainActor static func runWorkflows() async throws {
        try await testDeskStartup()
        try await testReadingPassNavigation()
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
        try await testSourceReadingHistory()
        try await testCompanionCatalogImportWorkflow()
        try await testCastaliaProfileImportWorkflow()
        try await testPublicProviderRepositoryWorkflow()
        try await testRecommendationTitleProjection()
        try await testPublicationWorkflow()
        try await testGroupReplies()
        try await testSharedListReview()
        print("Native desk workflows: duplicate-title identity, failed navigation, explicit sessions, private earlier-character-version drafts, thrown save rejection, unrated Unicode review, list edit/conflict retention, failed social retry with durable draft recovery, reopen and cancellation distinctions passed.")
    }

    @MainActor static func testGroupReplies() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("booknook-group-reply-" + UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let model = DeskModel(repository: DeskRepository(directory: directory))
        await model.open()
        let issuer = try Curve25519.Signing.PrivateKey(rawRepresentation: Data(repeating: 1, count: 32))
        let holder = try Curve25519.Signing.PrivateKey(rawRepresentation: Data(repeating: 2, count: 32))
        var connections = ConnectionConfiguration()
        let config = CastaliaConfiguration(issuer: "synthetic-issuer", keyId: "test",
            issuerPublicKey: hex(issuer.publicKey.rawRepresentation), audience: "booknook-test",
            terminalCaller: "synthetic-booknook-test", holderPublicKey: hex(holder.publicKey.rawRepresentation),
            policyDigest: hex(SHA256.hash(data: Data("synthetic policy".utf8))),
            credentialEndpoint: "http://127.0.0.1:65500/credential")
        connections.castalia = config
        try await model.repository.saveConnections(connections)
        let groupID = try await model.repository.socialOwnedID()
        try await approveFixture(["op": "create_group", "id": groupID, "name": "Synthetic reading group",
            "description": "", "private": true], model: model, config: config, issuer: issuer, holder: holder)
        let hiddenExcerpt = "Original spoiler text stays within its own post."
        let parentID = try await approveFixture(["op": "publish", "group_id": groupID, "post": [
            "kind": "discussion", "book": NSNull(), "body": hiddenExcerpt, "stars": NSNull(),
            "reading_status": NSNull(), "spoiler": true, "reply_to": NSNull()
        ]], model: model, config: config, issuer: issuer, holder: holder).id

        let controller = SocialController()
        await controller.reload(model)
        let initial = controller.snapshot!
        let target = SocialGroupReplyTarget(actor: initial.actor, groupID: groupID, postID: parentID)
        let parent = target.parent(in: initial)
        precondition(parent?.spoilerHidden == true && parent?.post.body.isEmpty == true)
        var draft = SocialGroupReplyDraft(text: "  Reply from the café 日本語.  ", containsSpoilers: true)
        let action = draft.action(for: target, in: initial)!
        let post = action["post"] as! [String: Any]
        precondition(action["group_id"] as? String == groupID && post["reply_to"] as? String == parentID)
        precondition(post["body"] as? String == draft.text && post["spoiler"] as? Bool == true && post["book"] is NSNull)
        precondition(!((try? socialJSON(action)) ?? "").contains(hiddenExcerpt))
        let wrongGroup = SocialGroupReplyTarget(actor: initial.actor, groupID: "another-group", postID: parentID)
        let wrongActor = SocialGroupReplyTarget(actor: "another-reader", groupID: groupID, postID: parentID)
        precondition(draft.action(for: wrongGroup, in: initial) == nil && draft.action(for: wrongActor, in: initial) == nil)
        precondition(SocialGroupReplyDraft(text: "  ").action(for: target, in: initial) == nil)

        // The app uses the real native draft store. The signer stays locked, so
        // there is no Keychain access, consent dialog, issuer call, or relay.
        let result = await controller.publish(action, title: "Publish this group reply", model)
        guard case .savedDraft(let draftID) = result else { preconditionFailure("A denied reply must stay private and recoverable") }
        precondition(controller.error?.contains("Unlock your Castalia identity") == true)
        let event = controller.snapshot!.state.drafts[draftID]!
        let disclosure = socialDisclosure(action: event.action.object!, event: event, state: controller.snapshot?.state)
        precondition(disclosure.contains { $0.contains(parentID) } && disclosure.contains { $0.contains("Synthetic reading group") })
        precondition(!disclosure.contains { $0.contains(hiddenExcerpt) })
        precondition(event.delivery.object?["recipients"] as? [String] == [initial.actor])
        precondition(Set(controller.snapshot!.state.outbox.keys) == Set(initial.state.outbox.keys))

        try await model.repository.socialLocal(Data(try socialJSON(["op": "hide", "event_id": parentID, "hidden": true]).utf8))
        await controller.reload(model)
        precondition(draft.action(for: target, in: controller.snapshot) == nil)
        try await model.repository.socialLocal(Data(try socialJSON(["op": "hide", "event_id": parentID, "hidden": false]).utf8))
        try await model.repository.socialLocal(Data(try socialJSON(["op": "block", "actor": initial.actor, "blocked": true]).utf8))
        await controller.reload(model)
        precondition(draft.action(for: target, in: controller.snapshot) == nil)
        try await model.repository.socialLocal(Data(try socialJSON(["op": "block", "actor": initial.actor, "blocked": false]).utf8))
        await controller.reload(model)

        draft.savedDraftID = draftID
        draft.text += "Edited before another review."
        let retry = await controller.publish(draft.action(for: target, in: controller.snapshot)!, title: "Publish this group reply",
                                             model, replacingDraftID: draft.savedDraftID)
        guard case .savedDraft(let retryID) = retry else { preconditionFailure("An edited reply must remain recoverable") }
        let reopened = DeskModel(repository: DeskRepository(directory: directory))
        await reopened.open()
        let recovered = try await reopened.repository.socialSnapshot(reveal: false)
        let recoveredAction = recovered.state.drafts[retryID]?.action.object
        let recoveredPost = recoveredAction?["post"] as? [String: Any]
        precondition(retryID != draftID && recovered.state.drafts[draftID] == nil)
        precondition(recoveredAction?["group_id"] as? String == groupID && recoveredPost?["reply_to"] as? String == parentID)
        precondition(recoveredPost?["body"] as? String == draft.text && recoveredPost?["spoiler"] as? Bool == true)
        precondition(Set(recovered.state.publications.keys) == Set(initial.state.publications.keys))
        precondition(Set(recovered.state.outbox.keys) == Set(initial.state.outbox.keys))
        try await approveFixture(["op": "close_group", "group_id": groupID],
                                 model: reopened, config: config, issuer: issuer, holder: holder)
        await controller.reload(reopened)
        precondition(!target.permitsVisibleParent(parent!, in: controller.snapshot))
        precondition(controller.snapshot?.state.drafts[retryID] != nil)
        print("Native group replies: exact parent, hidden/blocked/closed refusal, spoiler-safe consent, and denied draft retry after SQLite reopen passed.")
    }

    @MainActor static func testSharedListReview() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("booknook-shared-list-" + UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let model = DeskModel(repository: DeskRepository(directory: directory))
        await model.open()
        let issuer = try Curve25519.Signing.PrivateKey(rawRepresentation: Data(repeating: 1, count: 32))
        let holder = try Curve25519.Signing.PrivateKey(rawRepresentation: Data(repeating: 2, count: 32))
        var connections = ConnectionConfiguration()
        let config = CastaliaConfiguration(issuer: "synthetic-issuer", keyId: "test",
            issuerPublicKey: hex(issuer.publicKey.rawRepresentation), audience: "booknook-test",
            terminalCaller: "synthetic-booknook-test", holderPublicKey: hex(holder.publicKey.rawRepresentation),
            policyDigest: hex(SHA256.hash(data: Data("synthetic policy".utf8))),
            credentialEndpoint: "http://127.0.0.1:65500/credential")
        connections.castalia = config
        try await model.repository.saveConnections(connections)
        let id = try await model.repository.socialOwnedID()
        let local = SocialBook(id: "local-title", title: "Z Published title", author: "Published author")
        let missing = SocialBook(id: "external-title", title: "A External title", author: "External author")
        let references = try [local, missing].map { try socialObject($0) }
        try await approveFixture(["op": "create_list", "id": id, "name": "Original list", "description": "Original description",
                                  "books": references], model: model, config: config, issuer: issuer, holder: holder)

        let controller = SocialController()
        await controller.reload(model)
        let original = controller.snapshot!.state.lists[id]!
        precondition(original.version == 1)
        var draft = SocialListDraft(original)
        draft.name = "Draft title: café 日本語"
        draft.description = "Draft description"
        let changedLocal = SocialBook(id: local.id, title: "Local metadata changed", author: "Local author")
        let added = SocialBook(id: "new-title", title: "New selection", author: "New author")
        precondition(draft.choices(local: [added, changedLocal]) == [local, missing, added])
        let action = try draft.action(id: id)
        let actionJSON = try socialJSON(action)
        let expectedBooks = try socialJSON(["books": references])
        let actualBooks = try socialJSON(["books": action["books"] as! [[String: Any]]])
        precondition(actualBooks == expectedBooks, "A text-only edit must keep exact published references in order")
        var changed = draft
        changed.select(changedLocal, false); changed.select(added, true); changed.select(changedLocal, true)
        changed.select(added, true)
        precondition(changed.books == [missing, added, local], "Explicit changes keep survivors and append selections once")

        // Real native preparation saves an unapproved v1 draft while the signer is locked.
        let first = await controller.publish(action, title: "Review this shared list", model, listReview: draft.review)
        guard case .savedDraft(let draftID) = first else { preconditionFailure("Current shared-list draft must be saved") }
        precondition(controller.error?.contains("Unlock your Castalia identity") == true)
        let stored = controller.snapshot!.state.drafts[draftID]!
        let storedActionJSON = try socialJSON(stored.action.object!)
        precondition(stored.resourceVersion == 1 && storedActionJSON == actionJSON)

        // A different actual repository handle commits v2 before the first reader reviews again.
        let second = DeskModel(repository: DeskRepository(directory: directory))
        await second.open()
        try await approveFixture(["op": "update_list", "id": id, "name": "Current list", "description": "Current description",
                                  "books": references.reversed().map { $0 }], model: second, config: config, issuer: issuer, holder: holder)
        let latest = try await second.repository.socialSnapshot(reveal: false)
        precondition(latest.state.lists[id]?.version == 2)
        let stale = await controller.publish(action, title: "Review this shared list", model, listReview: draft.review)
        precondition(stale == .notSaved && controller.error?.contains("This shared list changed") == true)
        precondition(controller.error?.contains("Your edits are still in the editor") == true)
        let retainedActionJSON = try socialJSON(draft.action(id: id))
        precondition(retainedActionJSON == actionJSON && draft.books == [local, missing])
        await controller.retryDraft(stored, model)
        precondition(controller.error?.contains("This shared list changed") == true)
        precondition(controller.error?.contains("Your saved draft was kept") == true)
        let staleDelete = await controller.publish(["op": "delete_list", "id": id], title: "Delete this shared list", model,
                                                   listReview: SocialListReview(original))
        precondition(staleDelete == .notSaved && controller.error?.contains("This shared list changed") == true)
        precondition(controller.error?.contains("reviewing deletion again") == true && controller.error?.contains("draft") == false)
        let final = try await second.repository.socialSnapshot(reveal: false)
        precondition(final.state.revision == latest.state.revision)
        precondition(final.state.lists[id]?.name == "Current list" && final.state.lists[id]?.books == [missing, local])
        precondition(Set(final.state.publications.keys) == Set(latest.state.publications.keys))
        precondition(Set(final.state.outbox.keys) == Set(latest.state.outbox.keys))
        precondition(Set(final.state.drafts.keys) == Set(latest.state.drafts.keys))
        let retained = final.state.drafts[draftID]!
        let retryActionJSON = try socialJSON(retained.action.object!)
        precondition(retained.resourceVersion == 1 && retryActionJSON == actionJSON)
        precondition(model.authorization.pending == nil)
        print("Native shared lists: ordered public references, explicit selection, stale edit/delete/retry refusal, and exact durable draft retention passed.")
    }

    private static func hex(_ bytes: some Sequence<UInt8>) -> String { bytes.map { String(format: "%02x", $0) }.joined() }

    @MainActor @discardableResult private static func approveFixture(_ action: [String: Any], model: DeskModel,
        config: CastaliaConfiguration, issuer: Curve25519.Signing.PrivateKey, holder: Curve25519.Signing.PrivateKey) async throws -> SocialEventModel {
        // Public synthetic fixture keys are restricted to this isolated desk.
        func canonical(_ value: [String: Any]) throws -> Data {
            try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys, .withoutEscapingSlashes])
        }
        func digest(_ bytes: Data) -> String { hex(SHA256.hash(data: bytes)) }
        func signature(_ key: Curve25519.Signing.PrivateKey, _ domain: String, _ value: [String: Any]) throws -> String {
            try hex(key.signature(for: Data(domain.utf8) + canonical(value)))
        }
        let raw = try await model.repository.socialPrepare(canonical(action))
        let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase
        let event = try decoder.decode(SocialEventModel.self, from: Data(raw.utf8))
        let request = try await model.repository.authorizedAction(payload: Data(raw.utf8), action: "social.execute",
            resourceVersion: event.resourceVersion, idempotencyKey: event.id)
        let requestBytes = Data(request.utf8)
        let disclosure: [String: Any] = ["title": "Synthetic group setup", "statements": ["Synthetic test data only"]]
        let nonce = String(digest(requestBytes).prefix(32))
        let common: [String: Any] = [
            "holder_public_key": config.holderPublicKey, "issuer": config.issuer, "key_id": config.keyId,
            "audience": config.audience, "caller": ["kind": "terminal", "id": config.terminalCaller],
            "request_digest_sha256": digest(requestBytes), "disclosure_digest_sha256": digest(try canonical(disclosure)),
            "nonce": nonce
        ]
        var claims = common
        claims["schema"] = "castalia.request-credential.v1"
        claims["policy_digest_sha256"] = config.policyDigest
        claims["issued_at"] = event.at - 1
        claims["expires_at"] = event.at + 119
        let credential: [String: Any] = ["claims": claims, "signature": try signature(issuer, "castalia.request-credential.v1/signature\0", claims)]
        var presentation = common
        presentation["schema"] = "castalia.credential-presentation.v2"
        presentation["credential_digest_sha256"] = digest(try canonical(credential))
        presentation["issued_at"] = event.at
        presentation["expires_at"] = event.at + 60
        presentation["signature"] = try signature(holder, "castalia.credential-presentation.v2/signature\0", presentation)
        let receiptRequest = try canonical(["schema": "castalia.credential-presentation-request.v2",
            "request_bytes_base64": requestBytes.base64EncodedString(), "disclosure": disclosure, "credential": credential])
        try await model.repository.socialApprove(event: raw, receipt: AuthorizationReceipt(
            requestJSON: String(decoding: receiptRequest, as: UTF8.self),
            presentationJSON: String(decoding: try canonical(presentation), as: UTF8.self)))
        return event
    }
}
