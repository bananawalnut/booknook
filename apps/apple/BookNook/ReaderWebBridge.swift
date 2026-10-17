// SPDX-License-Identifier: MPL-2.0
import WebKit

final class ReaderCoordinator: NSObject, WKScriptMessageHandler, WKNavigationDelegate {
    var onMessage: ([String: Any]) -> Void
    var loaded: URL?
    var root: URL?
    var networkRulesReady = false
    var pendingLoad: (() -> Void)?
    var annotationPayload = ""
    var submittedPayload = ""
    var fontSize = 19
    var submittedFontSize = 0
    var restorePosition = "null"
    var spine: [String] = []
    var destination: ReaderDestination?
    var appliedDestination: UUID?
    init(_ onMessage: @escaping ([String: Any]) -> Void) { self.onMessage = onMessage }
    func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
        guard let body = message.body as? [String: Any], message.frameInfo.isMainFrame,
              message.frameInfo.request.url?.standardizedFileURL.path == loaded?.standardizedFileURL.path else { return }
        if body["type"] as? String == "reader_link", let href = body["href"] as? String, let url = URL(string: href) { navigateLink(url) }
        else { onMessage(body) }
    }
    func navigateLink(_ url: URL) {
        guard let root else { return }
        let rootPath = root.standardizedFileURL.resolvingSymlinksInPath().path
        guard url.isFileURL, url.host == nil || url.host == "" || url.host == "localhost",
              url.standardizedFileURL.resolvingSymlinksInPath().path.hasPrefix(rootPath + "/") else {
            onMessage(["type": "navigation_status", "message": "This link is outside the attached book and cannot be opened in the reader."]); return
        }
        guard let resource = spine.first(where: { root.appendingPathComponent($0).standardizedFileURL.path == url.standardizedFileURL.path }) else {
            onMessage(["type": "navigation_status", "message": "This link points to a resource outside this book's reading order."]); return
        }
        onMessage(["type": "navigate", "resource": resource, "fragment": url.fragment?.removingPercentEncoding as Any? ?? NSNull()])
    }
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        guard webView.url?.standardizedFileURL.path == loaded?.standardizedFileURL.path else { return }
        webView.evaluateJavaScript("window.booknookAnnotations?.(\(annotationPayload.isEmpty ? "[]" : annotationPayload))", completionHandler: nil); submittedPayload = annotationPayload
        webView.evaluateJavaScript("window.booknookSetFontSize?.(\(fontSize))", completionHandler: nil); submittedFontSize = fontSize
        if destination != nil { applyDestination(in: webView) }
        else { webView.evaluateJavaScript("window.booknookRestorePosition?.(\(restorePosition))", completionHandler: nil) }
    }
    func applyDestination(in web: WKWebView) {
        guard let destination, destination.id != appliedDestination, let root,
              root.appendingPathComponent(destination.resource).standardizedFileURL.path == loaded?.standardizedFileURL.path else { return }
        let argument: Any = destination.anchor as Any? ?? (destination.fragment ?? "")
        guard let data = try? JSONSerialization.data(withJSONObject: [argument]), let json = String(data: data, encoding: .utf8) else { return }
        appliedDestination = destination.id
        let receiver = onMessage
        let source = loaded
        let function = destination.anchor == nil ? "booknookNavigateFragment" : "booknookRestorePosition"
        web.evaluateJavaScript("window.\(function)?.(...\(json))") { [weak self] result, error in
            guard let self, self.loaded == source,
                  self.destination?.id == destination.id,
                  self.appliedDestination == destination.id else { return }
            if error != nil || (result as? [String: Any])?["status"] as? String != "resolved" {
                receiver(["type": "navigation_status", "message": "The chapter is open, but this saved passage or link target could not be found uniquely."])
            }
        }
    }
    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) { onMessage(["type": "error", "message": "Unable to open book content: \(error.localizedDescription)"]) }
    func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction, decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
        guard let url = action.request.url, let root, action.targetFrame?.isMainFrame == true else { decisionHandler(.cancel); return }
        let rootPath = root.standardizedFileURL.resolvingSymlinksInPath().path
        guard url.isFileURL, url.host == nil || url.host == "" || url.host == "localhost",
              url.standardizedFileURL.resolvingSymlinksInPath().path.hasPrefix(rootPath + "/") else {
            decisionHandler(.cancel)
            // Keep internal/programmatic requests contained without claiming the
            // reader followed a link. Explicit links still explain the rejection.
            if action.navigationType == .linkActivated {
                onMessage(["type": "navigation_status", "message": "This link is outside the attached book and cannot be opened in the reader."])
            }
            return
        }
        if action.navigationType == .linkActivated { decisionHandler(.cancel); navigateLink(url); return }
        // Initial loads are native-controlled; book redirects cannot silently
        // change the resource assigned to future selections and positions.
        decisionHandler(url.standardizedFileURL.path == loaded?.standardizedFileURL.path ? .allow : .cancel)
    }
}
func makeWebView(_ coordinator: ReaderCoordinator, scriptSources: [String]? = nil) -> WKWebView {
    let configuration = WKWebViewConfiguration()
    configuration.websiteDataStore = .nonPersistent()
    configuration.defaultWebpagePreferences.allowsContentJavaScript = false
    configuration.userContentController.add(coordinator, name: "booknook")
    let scripts = scriptSources ?? ["reader-geometry", "reader"].compactMap { name in
        Bundle.main.url(forResource: name, withExtension: "js").flatMap { try? String(contentsOf: $0, encoding: .utf8) }
    }
    for script in scripts { configuration.userContentController.addUserScript(WKUserScript(source: script, injectionTime: .atDocumentEnd, forMainFrameOnly: true)) }
    let web = WKWebView(frame: .zero, configuration: configuration); web.navigationDelegate = coordinator
    #if os(macOS)
    web.underPageBackgroundColor = NSColor(red: 247.0 / 255, green: 242.0 / 255, blue: 229.0 / 255, alpha: 1)
    #else
    let paper = UIColor(red: 247.0 / 255, green: 242.0 / 255, blue: 229.0 / 255, alpha: 1)
    web.isOpaque = false; web.backgroundColor = paper; web.scrollView.backgroundColor = paper
    web.underPageBackgroundColor = paper
    #endif
    let rules = "[{\"trigger\":{\"url-filter\":\".*\"},\"action\":{\"type\":\"block\"}},{\"trigger\":{\"url-filter\":\"^file:\"},\"action\":{\"type\":\"ignore-previous-rules\"}},{\"trigger\":{\"url-filter\":\"^data:\"},\"action\":{\"type\":\"ignore-previous-rules\"}}]"
    WKContentRuleListStore.default().compileContentRuleList(forIdentifier: "BookNookNoNetwork", encodedContentRuleList: rules) { [weak web, weak coordinator] list, error in
        guard let web, let coordinator else { return }
        guard let list else { coordinator.onMessage(["type": "error", "message": "Reader isolation could not be initialized: \(error?.localizedDescription ?? "unknown error")"]); return }
        web.configuration.userContentController.add(list); coordinator.networkRulesReady = true
        coordinator.pendingLoad?(); coordinator.pendingLoad = nil
    }
    return web
}
func updateWeb(_ web: WKWebView, coordinator: ReaderCoordinator, root: URL, resource: String, annotations: [Annotation], fontSize: Int, restorePosition: [String: Any]?, spine: [String], destination: ReaderDestination?) {
    let url = root.appendingPathComponent(resource)
    coordinator.root = root; coordinator.fontSize = fontSize; coordinator.spine = spine; coordinator.destination = destination
    #if os(macOS)
    let renderPortableInk = true
    #else
    let renderPortableInk = false
    #endif
    let items: [[String: Any]] = annotations.filter { ($0.anchor.object?["resource"] as? String) == resource }.map { annotation in
        ["id": annotation.id, "anchor": annotation.anchor.any, "ink_geometry": annotation.inkGeometry?.any ?? NSNull(), "legacy_ink": (annotation.nativeDrawing != nil || !annotation.strokes.isEmpty) && annotation.inkGeometry == nil, "strokes": annotation.strokes.map(\.any), "render_portable": renderPortableInk || annotation.nativeDrawing == nil]
    }
    let data = try? JSONSerialization.data(withJSONObject: items, options: .sortedKeys)
    coordinator.annotationPayload = data.flatMap { String(data: $0, encoding: .utf8) } ?? "[]"
    if coordinator.loaded != url {
        coordinator.loaded = url; coordinator.submittedPayload = ""
        if let restorePosition, restorePosition["resource"] as? String == resource, let data = try? JSONSerialization.data(withJSONObject: restorePosition), let json = String(data: data, encoding: .utf8) { coordinator.restorePosition = json } else { coordinator.restorePosition = "null" }
        let load = { [weak web] in web?.loadFileURL(url, allowingReadAccessTo: root); return () }
        if coordinator.networkRulesReady { load() } else { coordinator.pendingLoad = load }
        return
    }
    if destination?.id != coordinator.appliedDestination { coordinator.applyDestination(in: web) }
    if coordinator.fontSize != coordinator.submittedFontSize { web.evaluateJavaScript("window.booknookSetFontSize?.(\(fontSize))", completionHandler: nil); coordinator.submittedFontSize = fontSize }
    if coordinator.annotationPayload != coordinator.submittedPayload {
        web.evaluateJavaScript("window.booknookAnnotations?.(\(coordinator.annotationPayload))", completionHandler: nil)
        coordinator.submittedPayload = coordinator.annotationPayload
    }

}

