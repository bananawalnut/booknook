// SPDX-License-Identifier: MPL-2.0
import Foundation
import WebKit

@MainActor func testRecommendationTitleProjection() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("booknook-recommendation-titles-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }

    let multiline = "First line\nSecond line\tby the lake"
    let oversized = String(repeating: "é", count: 1_999) + "🌿 over the provider limit"
    let controlOnly = "\u{0001}"
    let expected = "The Green Wood"
    let csv = """
        Book Id,Title,Author,Exclusive Shelf
        1,"\(multiline)",A,read
        2,\(expected),B,to-read
        3,\(oversized),C,read
        4,\(controlOnly),D,read

        """
    let fixture = root.appendingPathComponent("catalog.csv")
    try csv.write(to: fixture, atomically: true, encoding: .utf8)
    let repository = DeskRepository(directory: root.appendingPathComponent("desk"))
    let model = DeskModel(repository: repository)
    await model.open()
    model.accept(try await repository.importCatalogFile(fixture, format: "goodreads"))
    precondition(Set(model.books.map(\.title)) == Set([multiline, oversized, controlOnly, expected]))
    let match = model.books.first { $0.title == expected }!
    try await model.perform(["op": "set_book_subjects", "book_id": match.id, "subjects": ["nature"]])

    let social = SocialController()
    await social.local(["op": "set_subjects", "subjects": ["nature"]], model)
    precondition(social.error == nil)
    let readIDs = try JSONSerialization.data(withJSONObject: model.books.filter { $0.shelf == "read" }.map(\.id))
    for title in [multiline, oversized, controlOnly] {
        let candidates = model.books.filter { $0.title == title || $0.id == match.id }.map {
            ["book_id": $0.id, "title": $0.title, "subjects": $0.subjects]
        }
        let raw = try JSONSerialization.data(withJSONObject: candidates)
        do {
            _ = try await repository.socialRecommendations(candidates: raw, excluded: readIDs)
            preconditionFailure("Unprojected title unexpectedly passed the provider")
        } catch {
            precondition(bookNookErrorMessage(error).contains("Invalid provider operation"))
        }
    }

    let subjects = ["nature", "wood\tgrain", "\u{0001}"]
    try await model.perform(["op": "set_book_subjects", "book_id": match.id, "subjects": subjects])
    await social.recommend(model)
    precondition(social.recommendationError == nil && social.recommendations.map(\.bookId) == [match.id])
    precondition(social.recommendations[0].reasons == ["Matches your interest in nature"])
    let originalTitles = Set([multiline, oversized, controlOnly, expected])
    precondition(Set(social.catalog.values.map(\.title)) == originalTitles)
    let reopened = DeskRepository(directory: root.appendingPathComponent("desk"))
    let (state, _) = try await reopened.open()
    precondition(Set(state.books.values.map(\.title)) == originalTitles)
    precondition(Set(state.books[match.id]!.subjects) == Set(subjects))
    print("Goodreads metadata projection preserves original titles and subjects and restores unrelated offline recommendations with reasons.")
}

@MainActor func testPublicProviderRepositoryWorkflow() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("booknook-provider-app-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let fixture = URL(fileURLWithPath: FileManager.default.currentDirectoryPath).appendingPathComponent("tests/fixtures/provider-sample.epub")
    let alternate = try PackagedAlternateProvider(bytes: Data(contentsOf: fixture))
    let repository = DeskRepository(directory: root, providers: [alternate], recommendationProviderID: alternate.id)
    let model = DeskModel(repository: repository); await model.open()
    precondition(model.error == nil && model.books.isEmpty)
    model.accept(try await repository.importCatalog(providerID: alternate.id, sourceID: "selection"))
    precondition(model.books.count == 1)
    let bookID = model.books[0].id
    try await model.selectBook(bookID)
    let notebook = model.notebook!
    let privateText = "This notebook is private: café 日本語 🦊"
    try await model.perform(["op": "edit_block", "notebook_id": notebook.id, "block_id": notebook.blocks[0].id,
        "base_update": notebook.blocks[0].update, "text": privateText, "at": 42])
    try await model.perform(["op": "update_book", "book_id": bookID, "title": "Reader title", "shelf": "currently-reading", "rating": 4, "review": "Private review"])
    await alternate.setTitle("Updated catalog title")
    model.accept(try await repository.importCatalog(providerID: alternate.id, sourceID: "selection"))
    precondition(model.books.count == 1 && model.books[0].review == "Private review" && model.books[0].rating == 4)
    precondition(model.notebook!.blocks[0].text == privateText)
    model.accept(try await repository.importContent(providerID: alternate.id, book: bookID, sourceID: "edition-1", mediaType: "application/epub+zip"))
    let asset = model.state!.editions.values.flatMap(\.assets).first!
    let assetDirectory = try await repository.assetDirectory(asset.id)
    precondition(FileManager.default.fileExists(atPath: assetDirectory.appendingPathComponent(asset.spine[0]).path))

    let social = SocialController()
    await social.recommend(model)
    precondition(social.recommendationError == nil && social.recommendations.first?.bookId == bookID)
    let requests = await alternate.receivedRequests()
    let recommendation = requests.last { $0.contains("\"recommendations\"") }!
    precondition(!recommendation.contains(privateText) && !recommendation.contains("Private review") && !recommendation.contains(notebook.id))
    let operation = try providerSmokeObject(recommendation)["operation"] as! [String: Any]
    let input = operation["input"] as! [String: Any]
    precondition(Set(input.keys) == Set(["candidates", "preferred_subjects", "shared_ratings", "friends", "excluded"]))

    // Delayed alternate source cannot write after cancellation, even though its
    // callback ignores cancellation and eventually supplies a valid catalog.
    let baseline = model.state!.revision
    await alternate.pauseNext()
    let cancelled = Task { try await repository.importCatalog(providerID: alternate.id, sourceID: "selection") }
    await alternate.waitForRequest(); cancelled.cancel(); await alternate.release()
    var cancelledCorrectly = false
    do { _ = try await cancelled.value } catch is CancellationError { cancelledCorrectly = true }
    precondition(cancelledCorrectly)
    let afterCancel = try await repository.snapshot()
    precondition(afterCancel.revision == baseline)

    // Foreign callback failure must not turn parent cancellation into an error alert.
    await alternate.pauseNext(fail: true)
    let cancelledFailure = Task { try await repository.importCatalog(providerID: alternate.id, sourceID: "selection") }
    await alternate.waitForRequest(); cancelledFailure.cancel(); await alternate.release()
    var cancelledFailureCorrectly = false
    do { _ = try await cancelledFailure.value } catch is CancellationError { cancelledFailureCorrectly = true }
    precondition(cancelledFailureCorrectly)

    // A replaced host invalidates an in-flight fetch, not just a cached UI descriptor.
    await alternate.pauseNext()
    let stale = Task { try await repository.importCatalog(providerID: alternate.id, sourceID: "selection") }
    await alternate.waitForRequest()
    try await repository.registerProvider(alternate)
    await alternate.release()
    try await providerSmokeReject { _ = try await stale.value }
    let afterStale = try await repository.snapshot()
    precondition(afterStale.revision == baseline)
    // Opening DeskModel also executes its normal select/open-notebook command,
    // which advances revision. Inspect the repository reopen before that UI action.
    let reopened = DeskRepository(directory: root, providers: [alternate])
    let (reopenedState, _) = try await reopened.open()
    precondition(reopenedState.revision == baseline)
    precondition(reopenedState.notebooks[notebook.id]!.blocks[0].text == privateText)
    precondition(reopenedState.editions.values.flatMap(\.assets).contains { $0.sha256 == asset.sha256 })
    print("Actual app repository + public alternate provider: catalog/content/recommendation routing, private field preservation, minimized recommendation input, cancellation, generation race and durable reopen passed.")

    // The shipped file picker calls this exact URL entry point, not the
    // generic alternate-provider helper exercised above.
    let mobiURL = URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
        .appendingPathComponent("crates/native/tests/fixtures/mobi6-reader.mobi")
    let original = try Data(contentsOf: mobiURL)
    let accepted = try await reopened.importContent(book: bookID, url: mobiURL)
    model.accept(accepted)
    guard let mobi = model.readerAttachment?.asset, mobi.isMobi6 else {
        preconditionFailure("The normal selected-file provider did not attach MOBI to the active edition")
    }
    precondition(mobi.supportsTextSize && mobi.spine == ["mobi6-v1/book.xhtml"])
    let mobiFolder = try await reopened.assetDirectory(mobi.id)
    let stored = try Data(contentsOf: mobiFolder.appendingPathComponent("original.mobi"))
    let body = try String(contentsOf: mobiFolder.appendingPathComponent(mobi.spine[0]), encoding: .utf8)
    precondition(stored == original && body.contains("A window opened above the reading desk."))
    let source = model.readerSourceContext(resource: mobi.spine[0])!
    try await testAttachedTextReader(model: model, source: source, folder: mobiFolder, spine: mobi.spine,
        passage: "A window opened above the reading desk.", additionalProse: ["Café, 日本語, 🌿."])
    print("Normal selected-file provider preserved MOBI bytes; the contained WKWebView rendered the stable resource and delivered its selected passage.")
}

@MainActor func testAttachedTextReader(model: DeskModel, source: ReaderSourceContext, folder: URL, spine: [String],
                                        passage: String, additionalProse: [String] = []) async throws {
    let scripts = try ["reader-geometry", "reader"].map {
        try String(contentsOfFile: "apps/apple/Resources/\($0).js", encoding: .utf8)
    }
    let expectedProse = String(decoding: try JSONSerialization.data(withJSONObject: [passage] + additionalProse), as: UTF8.self)
    var readerError: String?
    var selectedQuote: String?
    let receive = model.readerMessages(from: source, currentResource: { source.resource }) { message in
        if message["type"] as? String == "selection" { selectedQuote = message["quote"] as? String }
    }
    let coordinator = ReaderCoordinator { message in
        if message["type"] as? String == "error" { readerError = message["message"] as? String ?? "Reader error" }
        receive(message)
    }
    let web = makeWebView(coordinator, scriptSources: scripts)
    web.frame = CGRect(x: 0, y: 0, width: 640, height: 480)
    defer {
        web.stopLoading()
        web.configuration.userContentController.removeScriptMessageHandler(forName: "booknook")
    }
    let clock = ContinuousClock()
    let deadline = clock.now + .seconds(10)
    func failure(_ message: String) -> NSError {
        NSError(domain: "BookNookTextReaderAcceptance", code: 1, userInfo: [NSLocalizedDescriptionKey: message])
    }
    func wait(for phase: String, until ready: () -> Bool) async throws {
        while clock.now < deadline {
            if let readerError { throw failure(readerError) }
            if ready() { return }
            try await Task.sleep(for: .milliseconds(25))
        }
        throw failure("Timed out waiting for attached text \(phase)")
    }

    let expectedPath = folder.appendingPathComponent(source.resource).standardizedFileURL.path
    let fontSize = 19
    updateWeb(web, coordinator: coordinator, root: folder, resource: source.resource, annotations: [],
        fontSize: fontSize, restorePosition: nil, spine: spine, destination: nil)
    try await wait(for: "file navigation") {
        web.url?.standardizedFileURL.path == expectedPath && !web.isLoading && coordinator.submittedFontSize == fontSize
    }
    var evaluation: String?
    web.evaluateJavaScript("""
        (() => {
          const [passage, ...additional] = \(expectedProse);
          const prose = document.body?.textContent ?? "";
          if (![passage, ...additional].every(text => prose.includes(text))) return "Missing attached prose";
          const resolved = booknookResolve({quote: passage});
          if (resolved.status !== "resolved" || !resolved.rects.length) return "Passage has no rendered text bounds";
          const selection = getSelection();
          selection.removeAllRanges();
          selection.addRange(resolved.range);
          return selection.toString() === passage ? "selected" : "Wrong DOM selection";
        })()
        """) { value, error in
        evaluation = error?.localizedDescription ?? (value as? String) ?? "No JavaScript result"
    }
    try await wait(for: "DOM prose and text bounds") { evaluation != nil }
    guard evaluation == "selected" else { throw failure(evaluation ?? "No JavaScript result") }
    try await wait(for: "selection through the native reader source gate") { selectedQuote == passage }
}
