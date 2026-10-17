// SPDX-License-Identifier: MPL-2.0
import Foundation
import WebKit
import Combine

@MainActor private final class KindleWebCall {
    private var continuation: CheckedContinuation<Any, Error>?
    var timeout: Task<Void, Never>?
    init(_ continuation: CheckedContinuation<Any, Error>) { self.continuation = continuation }
    func finish(_ result: Result<Any, Error>) {
        guard let continuation else { return }
        self.continuation = nil; timeout?.cancel(); timeout = nil
        continuation.resume(with: result)
    }
}

/// One explicit, temporary US Amazon session. No cookie enumeration, credential
/// extraction, persistent website store, script-message handler or automatic login.
@MainActor final class KindleWebSession: NSObject, ObservableObject, WKNavigationDelegate, WKUIDelegate {
    @Published private(set) var canReadCatalog = false
    @Published private(set) var status = "Sign in on Amazon, then choose Import catalog."
    @Published private(set) var closed = false
    let webView: WKWebView
    let connectionID = "kindle.web.us." + UUID().uuidString.lowercased()
    private var started = false
    private var generation = UUID()
    private var fetching = false
    private var pending: KindleWebCall?
    private let requestTimeout: Duration
    static let loginURL = URL(string: "https://www.amazon.com/ap/signin?openid.pape.max_auth_age=3600&openid.return_to=https%3A%2F%2Fread.amazon.com%2Fnotebook&openid.identity=http%3A%2F%2Fspecs.openid.net%2Fauth%2F2.0%2Fidentifier_select&openid.assoc_handle=amzn_kindle_ynhv2_us&openid.mode=checkid_setup&language=en_US&openid.claimed_id=http%3A%2F%2Fspecs.openid.net%2Fauth%2F2.0%2Fidentifier_select&pageId=amzn_kindle_ynhv2_us&openid.ns=http%3A%2F%2Fspecs.openid.net%2Fauth%2F2.0")!
    init(requestTimeout: Duration = .seconds(15)) {
        self.requestTimeout = requestTimeout
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        configuration.preferences.javaScriptCanOpenWindowsAutomatically = false
        webView = WKWebView(frame: .zero, configuration: configuration)
        super.init()
        webView.navigationDelegate = self; webView.uiDelegate = self
        webView.allowsBackForwardNavigationGestures = false
        webView.isInspectable = false
    }
    var descriptor: ProviderConnection {
        ProviderConnection(schema: "castalia.provider-connection.v1", connectionId: connectionID,
            providerId: "com.amazon.kindle", displayName: "Kindle US · temporary web session",
            capabilities: ["kindle.catalog"], status: closed ? "reauthorization_required" : (canReadCatalog ? "available" : "reauthorization_required"), credential: nil)
    }
    static func allowedNavigation(_ url: URL) -> Bool {
        url.scheme == "https" && ["www.amazon.com", "amazon.com", "read.amazon.com"].contains(url.host?.lowercased() ?? "") &&
            url.user == nil && url.password == nil && (url.port == nil || url.port == 443)
    }
    static func catalogOrigin(_ url: URL?) -> Bool {
        guard let url else { return false }
        return allowedNavigation(url) && url.host?.lowercased() == "read.amazon.com"
    }
    func start() {
        guard !started, !closed else { return }
        started = true
        webView.load(URLRequest(url: Self.loginURL, timeoutInterval: 60))
    }
    func cancelFetch() {
        generation = UUID()
        pending?.finish(.failure(CancellationError())); pending = nil
        // The private content world is not accessible to Amazon page scripts.
        webView.evaluateJavaScript("globalThis.__booknookKindleAbort?.abort()", in: nil, in: .defaultClient) { _ in }
    }
    func close() {
        guard !closed else { return }
        closed = true; canReadCatalog = false; cancelFetch(); webView.stopLoading()
        webView.navigationDelegate = nil; webView.uiDelegate = nil
        webView.configuration.websiteDataStore.removeData(ofTypes: WKWebsiteDataStore.allWebsiteDataTypes(), modifiedSince: .distantPast) { }
        // Releasing the sheet releases this nonpersistent data store. Nothing is
        // copied to UserDefaults, Keychain, the companion or a portable archive.
    }
    func page(_ url: URL) async throws -> Data {
        guard !closed else { throw KindleWebError.closed }
        guard !fetching, canReadCatalog, Self.catalogOrigin(webView.url),
              url.scheme == "https", url.host == "read.amazon.com", url.path == "/kindle-library/search",
              url.user == nil, url.password == nil, url.port == nil, url.fragment == nil else { throw KindleWebError.signIn }
        try Task.checkCancellation()
        let requestGeneration = generation
        fetching = true
        defer { fetching = false }
        let raw: Any
        do {
            raw = try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Any, Error>) in
                let call = KindleWebCall(continuation); pending = call
                // WebKit runs JavaScript out of process. A native deadline also
                // completes when that process is unresponsive or ignores abort.
                call.timeout = Task { [weak self] in
                    do { try await Task.sleep(for: self?.requestTimeout ?? .seconds(15)) } catch { return }
                    call.finish(.failure(KindleWebError.limit))
                    self?.webView.evaluateJavaScript("globalThis.__booknookKindleAbort?.abort()", in: nil, in: .defaultClient) { _ in }
                }
                webView.callAsyncJavaScript(Self.fetchScript,
                    arguments: ["requestURL": url.absoluteString, "expectedOrigin": KindleWebCatalogReader.origin,
                                "maximumBytes": 2 * 1024 * 1024, "timeoutMS": 15_000], in: nil, in: .defaultClient) { result in
                    call.finish(result)
                }
            }
            pending = nil
        } catch {
            try Task.checkCancellation()
            pending = nil
            guard !closed, requestGeneration == generation else { throw CancellationError() }
            if case KindleWebError.limit = error { throw KindleWebError.limit }
            throw KindleWebError.unavailable // Never surface WebKit URLs or server bodies.
        }
        try Task.checkCancellation()
        guard !closed, requestGeneration == generation, Self.catalogOrigin(webView.url) else { throw CancellationError() }
        guard let envelope = raw as? [String: Any] else { throw KindleWebError.unavailable }
        if let failure = envelope["failure"] as? String {
            switch failure {
            case "origin", "auth": throw KindleWebError.signIn
            case "limit", "timeout": throw KindleWebError.limit
            case "cancelled": throw CancellationError()
            default: throw KindleWebError.unavailable
            }
        }
        guard let body = envelope["body"] as? String, body.utf8.count <= 2 * 1024 * 1024 else { throw KindleWebError.unavailable }
        return Data(body.utf8)
    }
    // Read a bounded same-origin response without forwarding cookies or headers
    // to native code. Redirects fail closed instead of returning a login page.
    static let fetchScript = #"""
    if (location.origin !== expectedOrigin) return {failure:'origin'};
    const requested = new URL(requestURL);
    if (requested.origin !== expectedOrigin || requested.pathname !== '/kindle-library/search') return {failure:'origin'};
    const controller = new AbortController();
    globalThis.__booknookKindleAbort = controller;
    let expired = false;
    const timer = setTimeout(() => { expired = true; controller.abort(); }, timeoutMS);
    let reader;
    try {
      const response = await fetch(requested.href, {credentials:'same-origin', redirect:'error', cache:'no-store', signal:controller.signal, headers:{'Accept':'application/json'}});
      if (response.status === 401 || response.status === 403) return {failure:'auth'};
      if (!response.ok || response.redirected || response.url !== requested.href) return {failure:'response'};
      const type = response.headers.get('content-type') || '';
      if (!/^application\/json(?:\s*;|$)/i.test(type)) return {failure:'response'};
      const length = response.headers.get('content-length');
      if (length !== null && (!/^\d+$/.test(length) || Number(length) > maximumBytes)) return {failure:'limit'};
      if (!response.body) return {failure:'response'};
      reader = response.body.getReader();
      const chunks = []; let bytes = 0;
      while (true) {
        const part = await reader.read();
        if (part.done) break;
        bytes += part.value.byteLength;
        if (bytes > maximumBytes) { controller.abort(); return {failure:'limit'}; }
        chunks.push(part.value);
      }
      const data = new Uint8Array(bytes); let offset = 0;
      for (const chunk of chunks) { data.set(chunk, offset); offset += chunk.byteLength; }
      return {body:new TextDecoder('utf-8', {fatal:true}).decode(data)};
    } catch (_) {
      return {failure:controller.signal.aborted ? (expired ? 'timeout' : 'cancelled') : 'response'};
    } finally {
      clearTimeout(timer);
      if (reader) { try { await reader.cancel(); } catch (_) {} }
      if (globalThis.__booknookKindleAbort === controller) delete globalThis.__booknookKindleAbort;
    }
    """#
    func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction, decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
        guard !closed, let url = action.request.url, Self.allowedNavigation(url), action.targetFrame != nil else {
            if action.targetFrame?.isMainFrame != false { status = "This sign-in stays on Amazon’s US websites. External links and new windows are not opened." }
            decisionHandler(.cancel); return
        }
        if action.targetFrame?.isMainFrame == true {
            canReadCatalog = false
            if fetching { cancelFetch() }
        }
        decisionHandler(.allow)
    }
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        guard !closed else { return }
        canReadCatalog = Self.catalogOrigin(webView.url)
        status = canReadCatalog ? "Amazon’s US page is ready. Choose Import catalog to check access; signing in alone does not prove catalog availability." : "Complete Amazon sign-in and any verification on this page."
    }
    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) { failed(error) }
    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) { failed(error) }
    private func failed(_ error: Error) {
        guard !closed, (error as NSError).code != NSURLErrorCancelled else { return }
        canReadCatalog = false; status = "Amazon’s sign-in page could not load. Close this session and try again, or use a catalog file."
    }
    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
        canReadCatalog = false; cancelFetch(); status = "The temporary web session ended. Close it and sign in again; your library is unchanged."
    }
}
