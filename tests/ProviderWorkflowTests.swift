// SPDX-License-Identifier: MPL-2.0
import Foundation

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
}
