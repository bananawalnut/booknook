// SPDX-License-Identifier: MPL-2.0
import AppKit
import Foundation
import WebKit

@MainActor private final class KindleGate {
    var continuation: CheckedContinuation<Void, Never>?
    func pause() async { await withCheckedContinuation { continuation = $0 } }
    func wait() async throws { for _ in 0..<200 { if continuation != nil { return }; try await Task.sleep(for: .milliseconds(5)) }; preconditionFailure("Gate was not reached") }
    func release() { continuation?.resume(); continuation = nil }
}
@MainActor private func page(_ asin: String = "B000000001", title: String = "A café in 日本語", token: Any? = nil) throws -> Data {
    var value: [String: Any] = ["itemsList": [["asin": asin, "title": title, "authors": ["Writer, Test:"], "percentageRead": 78, "webReaderUrl": "https://example.invalid/private"]], "libraryType": "BOOKS", "sortType": "recency"]
    if let token { value["paginationToken"] = token }
    return try JSONSerialization.data(withJSONObject: value)
}
@MainActor private func rejected(_ action: () async throws -> Void) async throws {
    var caught = false
    do { try await action() } catch { caught = true }
    precondition(caught, "Invalid source was accepted")
}
@MainActor private func parserTests() async throws {
    let data = try page()
    let decoded = try KindleWebCatalogReader.parse(data)
    precondition(decoded.itemsList[0].title == "A café in 日本語")
    let output = try JSONSerialization.jsonObject(with: JSONEncoder().encode(KindleWebCatalog(items: decoded.itemsList))) as! [String: Any]
    let item = (output["items"] as! [[String: Any]])[0]
    precondition(item["position"] is NSNull && item["observed_at"] is NSNull)
    precondition(Set(item.keys) == Set(["asin", "title", "authors", "position", "observed_at"]))
    for bytes in [Data("<html>Sign in</html>".utf8), Data("{}".utf8), try page(token: ""), try page(token: 10), try page("invalid"), try page(title: ""), try page(token: "bad\ntoken")] {
        try await rejected { _ = try KindleWebCatalogReader.parse(bytes) }
    }
    let token = "page+&other=value/=日本語"
    let url = try KindleWebCatalogReader.pageURL(token: token)
    let query = URLComponents(url: url, resolvingAgainstBaseURL: false)!.queryItems!
    precondition(query.count == 5 && query.last?.name == "paginationToken" && query.last?.value == token)
    var requests: [URL] = []
    let result = try await KindleWebCatalogReader.collect { request in
        requests.append(request)
        return try page(requests.count == 1 ? "B000000001" : "B000000002", token: requests.count == 1 ? token : nil)
    }
    precondition(requests.count == 2 && result.pageCount == 2 && result.catalog.items.count == 2 && result.coverage == "endpoint_exhausted")
    precondition(URLComponents(url: requests[1], resolvingAgainstBaseURL: false)!.queryItems!.last?.value == token)
    var n = 0
    let dedup = try await KindleWebCatalogReader.collect { _ in n += 1; return try page(token: n == 1 ? "next" : nil) }
    precondition(dedup.catalog.items.count == 1 && dedup.pageCount == 2)
    try await rejected { _ = try await KindleWebCatalogReader.collect { _ in try page(token: "repeated") } }
    n = 0
    try await rejected { _ = try await KindleWebCatalogReader.collect { _ in n += 1; return try page(title: "Version \(n)", token: n == 1 ? "next" : nil) } }
    for limit in [KindleWebCatalogReader.Limits(pageBytes: 2), .init(totalBytes: 2), .init(pages: 1), .init(items: 0), .init(seconds: 0)] {
        try await rejected { _ = try await KindleWebCatalogReader.collect(limits: limit) { _ in try page(token: "next") } }
    }
    let empty = try await KindleWebCatalogReader.collect { _ in Data(#"{"itemsList":[],"libraryType":"BOOKS","sortType":"recency","paginationToken":null}"#.utf8) }
    precondition(empty.catalog.items.isEmpty && empty.pageCount == 1)
    print("Kindle parser/paging: Unicode, metadata-only output, absent/null termination, token encoding, malformed pages, deduplication, changing records and all five resource limits passed.")
}

@MainActor private func workflowTests() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("booknook-kindle-web-\(UUID())")
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    let old = ProcessInfo.processInfo.environment["BOOKNOOK_DATA_DIR"]
    defer { if let old { setenv("BOOKNOOK_DATA_DIR", old, 1) } else { unsetenv("BOOKNOOK_DATA_DIR") }; try? FileManager.default.removeItem(at: root) }
    setenv("BOOKNOOK_DATA_DIR", root.path, 1)
    let model = DeskModel(); await model.open(); precondition(model.error == nil)
    let connection = ProviderConnection(schema: "castalia.provider-connection.v1", connectionId: "kindle.web.us.synthetic", providerId: "com.amazon.kindle", displayName: "Synthetic session", capabilities: ["kindle.catalog"], status: "available", credential: nil)
    let before = model.state!.revision
    let failure = KindleWebImportWorkflow(); var count = 0
    try await rejected { _ = try await failure.run(model: model, connection: connection) { _ in count += 1; if count > 1 { throw KindleWebError.unavailable }; return try page(token: "next") } }
    precondition(model.state!.revision == before && model.books.isEmpty && !failure.isRunning)
    let gate = KindleGate(); let cancelled = KindleWebImportWorkflow()
    let task = Task { try await cancelled.run(model: model, connection: connection) { _ in await gate.pause(); return try page() } }
    try await gate.wait(); task.cancel(); gate.release()
    do { _ = try await task.value; preconditionFailure("Cancelled fetch committed") } catch is CancellationError { }
    precondition(model.state!.revision == before && model.books.isEmpty)
    let good = KindleWebImportWorkflow()
    _ = try await good.run(model: model, connection: connection) { _ in try page() }
    precondition(model.books.count == 1 && good.phase == .saved)
    let bookID = model.books[0].id
    try await model.perform(["op": "update_book", "book_id": bookID, "title": "Private edited title", "shelf": "reading", "rating": 4, "review": "Private review"])
    _ = try await good.run(model: model, connection: connection) { _ in try page(title: "Changed provider title") }
    precondition(model.books.count == 1 && model.books[0].title == "Private edited title")
    precondition(model.state!.positions.isEmpty)
    let stable = model.state!.revision
    let badPersist = KindleWebImportWorkflow(persist: { desk, _, _ in try await desk.repository.execute(Data(#"{"op":"import_kindle","json":"invalid","at":1}"#.utf8)) })
    try await rejected { _ = try await badPersist.run(model: model, connection: connection) { _ in try page("B000000002") } }
    precondition(model.state!.revision == stable && model.books.count == 1)
    let commitGate = KindleGate()
    let delayed = KindleWebImportWorkflow(persist: { desk, bytes, source in
        let snapshot = try await desk.repository.execute(JSONSerialization.data(withJSONObject: ["op":"import_kindle_from_connection", "json":String(decoding:bytes,as:UTF8.self),"provider_connection_id":source,"at":101]))
        await commitGate.pause(); return snapshot
    })
    let committed = Task { try await delayed.run(model: model, connection: connection) { _ in try page("B000000003") } }
    try await commitGate.wait()
    try await model.perform(["op":"add_book", "title":"Concurrent private entry", "author":"Writer", "at":102])
    let newer = model.state!.revision
    committed.cancel(); commitGate.release(); _ = try await committed.value
    precondition(delayed.phase == .saved && model.state!.revision == newer && model.books.count == 3)
    let reopened = DeskModel(); await reopened.open()
    precondition(reopened.books.count == 3 && reopened.books.contains { $0.title == "Private edited title" })
    let exported = root.appendingPathComponent("catalog-backup.json")
    try await reopened.repository.export(to: exported)
    let state = String(decoding: try Data(contentsOf: exported), as: UTF8.self)
    precondition(state.contains(connection.connectionId) && !state.contains("percentageRead") && !state.contains("webReaderUrl"))
    let settings = try await reopened.repository.connections()
    precondition(settings.providerConnections.isEmpty, "Web session must not replace the companion registry")
    print("Kindle workflow: actual native SQLite atomic failure/cancellation, successful import, repeated import preserving private edits, no reading-position inference, postcommit cancellation, monotonic state and reopen passed.")
}

@MainActor private func webTests() async throws {
    for url in ["https://www.amazon.com/ap/signin", "https://amazon.com/ap/mfa", "https://read.amazon.com/notebook"] { precondition(KindleWebSession.allowedNavigation(URL(string:url)!)) }
    for url in ["http://read.amazon.com/notebook", "https://read.amazon.com.evil.invalid/", "https://user:secret@read.amazon.com/", "https://read.amazon.com:444/", "file:///tmp/test", "https://read.amazon.co.uk/", "javascript:void(0)"] { precondition(!KindleWebSession.allowedNavigation(URL(string:url)!)) }
    let first = KindleWebSession(requestTimeout: .milliseconds(80)), second = KindleWebSession()
    precondition(!first.webView.configuration.websiteDataStore.isPersistent && !second.webView.configuration.websiteDataStore.isPersistent)
    precondition(first.webView.url == nil && second.webView.url == nil && first.descriptor.credential == nil)
    let cookie = HTTPCookie(properties: [.domain:"booknook-test.invalid", .path:"/", .name:"synthetic-session", .value:"not-a-real-credential", .secure:"TRUE"])!
    await first.webView.configuration.websiteDataStore.httpCookieStore.setCookie(cookie)
    let otherCookies = await second.webView.configuration.websiteDataStore.httpCookieStore.allCookies()
    precondition(otherCookies.isEmpty)
    // Actual WK content worlds, with synthetic fetch responses in the private
    // world only. No Amazon URL is loaded and no network request is performed.
    let web = first.webView
    web.navigationDelegate = nil
    web.loadHTMLString("<html><body>Synthetic Kindle transport fixture</body></html>", baseURL: nil)
    for _ in 0..<200 { if !web.isLoading && web.url != nil { break }; try await Task.sleep(for: .milliseconds(10)) }
    func evaluate(_ script: String, world: WKContentWorld? = nil, args: [String: Any] = [:]) async throws -> Any? {
        try await web.callAsyncJavaScript(script, arguments: args, in: nil, contentWorld: world ?? .defaultClient)
    }
    let origin = try await evaluate("return location.origin") as! String
    let requested = origin == "null" ? "https://read.amazon.com/kindle-library/search" : origin + "/kindle-library/search"
    // The source guard is tested unchanged before substituting a synthetic origin
    // for transport-only tests. page() independently rejects this blank document.
    try await rejected { _ = try await first.page(KindleWebCatalogReader.pageURL(token:nil)) }
    let args: [String: Any] = ["requestURL": requested, "expectedOrigin": "https://read.amazon.com", "maximumBytes": 128, "timeoutMS": 100]
    let refused = try await evaluate(KindleWebSession.fetchScript, args: args) as! [String: Any]
    precondition(refused["failure"] as? String == "origin")
    // A same-origin synthetic document is loaded from an in-memory HTML string;
    // WebKit gives this document the base URL origin without making that request.
    web.loadHTMLString("<html><body>Synthetic origin only</body></html>", baseURL: URL(string:"https://read.amazon.com/notebook")!)
    for _ in 0..<200 { if !web.isLoading, (try? await evaluate("return location.origin")) as? String == "https://read.amazon.com" { break }; try await Task.sleep(for: .milliseconds(10)) }
    let syntheticOrigin = try await evaluate("return location.origin")
    precondition(syntheticOrigin as? String == "https://read.amazon.com")
    _ = try await evaluate("globalThis.fetch = () => { throw Error('page-world trap'); }; return true", world: .page)
    let install = #"""
    globalThis.fetch = async (url, options) => {
      globalThis.fixtureOptions = options;
      if (mode === 'timeout') return new Promise((resolve, reject) => options.signal.addEventListener('abort', () => reject(Error('aborted'))));
      const bytes = new TextEncoder().encode(body);
      return { status:status, ok:status===200, redirected:false, url:url,
        headers:new Headers({'content-type':type}),
        body:new ReadableStream({start(c){c.enqueue(bytes);c.close();}})};
    }; return true;
    """#
    let requestArgs: [String: Any] = ["requestURL":"https://read.amazon.com/kindle-library/search", "expectedOrigin":"https://read.amazon.com", "maximumBytes":128, "timeoutMS":50]
    for (body,type,status,mode,expected) in [("{}","application/json",200,"normal","body"), ("<html>Login</html>","text/html",200,"normal","response"), ("{}","application/json",401,"normal","auth"), (String(repeating:" ",count:129),"application/json",200,"normal","limit"), ("{}","application/json",200,"timeout","timeout")] {
        _ = try await evaluate(install,args:["body":body,"type":type,"status":status,"mode":mode])
        let value = try await evaluate(KindleWebSession.fetchScript,args:requestArgs) as! [String:Any]
        if expected == "body" { precondition(value["body"] as? String == "{}") } else { precondition(value["failure"] as? String == expected) }
    }
    let optionsSafe = try await evaluate("return fixtureOptions.credentials === 'same-origin' && fixtureOptions.redirect === 'error' && fixtureOptions.cache === 'no-store'")
    precondition(optionsSafe as? Bool == true)
    // Test the actual page() native completion gate even when JavaScript ignores
    // abort entirely. In-memory base URL supplies origin, never a network load.
    first.webView(web, didFinish: nil)
    _ = try await evaluate("globalThis.fetch = () => new Promise(() => {}); return true")
    let deadlineStart = ContinuousClock.now
    do { _ = try await first.page(KindleWebCatalogReader.pageURL(token:nil)); preconditionFailure("Unresponsive fetch escaped native deadline") }
    catch KindleWebError.limit { }
    precondition(deadlineStart.duration(to: .now) < .seconds(2))
    let cancelStart = ContinuousClock.now
    let cancellation = Task { try await first.page(KindleWebCatalogReader.pageURL(token:nil)) }
    try await Task.sleep(for: .milliseconds(10)); first.cancelFetch()
    do { _ = try await cancellation.value; preconditionFailure("Explicit native cancellation ignored") } catch is CancellationError { }
    precondition(cancelStart.duration(to: .now) < .seconds(2))
    first.close(); second.close()
    for _ in 0..<200 { if await first.webView.configuration.websiteDataStore.httpCookieStore.allCookies().isEmpty { break }; try await Task.sleep(for: .milliseconds(10)) }
    let remainingCookies = await first.webView.configuration.websiteDataStore.httpCookieStore.allCookies()
    precondition(remainingCookies.isEmpty)
    precondition(first.closed && !first.canReadCatalog && first.descriptor.status == "reauthorization_required")
    print("Kindle WebKit: actual independent nonpersistent cookie stores and clearing; production origin gate; isolated content-world fetch; metadata response, HTML/auth rejection, streamed byte limit, JS timeout, native unresponsive-fetch deadline and immediate cancellation passed using synthetic in-memory responses. No account, Amazon HTTP request or UI action occurred.")
}

@main struct KindleWebTests {
    @MainActor static func main() {
        let app = NSApplication.shared; app.setActivationPolicy(.prohibited)
        Task { do { try await parserTests(); try await workflowTests(); try await webTests(); print("Kindle web connector tests passed."); exit(0) }
               catch { fputs("Kindle web tests failed: \(error)\n", stderr); exit(1) } }
        app.run()
    }
}
