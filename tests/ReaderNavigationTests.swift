// SPDX-License-Identifier: MPL-2.0
// Actual contained WKWebView navigation with the production coordinator.
import AppKit
import WebKit
struct JSONValue { let any: Any; var object: [String: Any]? { any as? [String: Any] } }
struct Annotation { let id: String; let anchor: JSONValue; let inkGeometry: JSONValue?; let nativeDrawing: String?; let strokes: [JSONValue] }

@MainActor final class ReaderNavigationHarness: NSObject, WKNavigationDelegate {
    var root: URL!
    var web: WKWebView!
    var coordinator: ReaderCoordinator!
    var current = "one.xhtml"
    var selectionResource = ""
    var status = ""
    var navigationMessages: [String] = []
    var cancelledProgrammaticBlank = false
    let spine = ["one.xhtml", "two.xhtml", "three.xhtml"]
    func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction, decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
        coordinator.webView(webView, decidePolicyFor: action) { policy in
            if action.request.url?.absoluteString == "about:blank", action.navigationType == .other, policy == .cancel {
                self.cancelledProgrammaticBlank = true
            }
            decisionHandler(policy)
        }
    }
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        coordinator.webView(webView, didFinish: navigation)
    }
    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        coordinator.webView(webView, didFailProvisionalNavigation: navigation, withError: error)
    }
    func load(_ resource: String, destination: ReaderDestination? = nil) {
        current = resource
        updateWeb(web, coordinator: coordinator, root: root, resource: resource, annotations: [], fontSize: 19, restorePosition: nil, spine: spine, destination: destination)
    }
    func evaluate(_ code: String) async throws -> Any? {
        try await withCheckedThrowingContinuation { continuation in
            web.evaluateJavaScript(code) { value, error in
                if let error { continuation.resume(throwing: error) } else { continuation.resume(returning: value) }
            }
        }
    }
    func wait(_ name: String, _ condition: () async throws -> Bool) async throws {
        for _ in 0..<160 { if try await condition() { return }; try await Task.sleep(for: .milliseconds(25)) }
        do { print("DOM", String(describing: try await evaluate("document.body.outerHTML.slice(0,300)"))) } catch { print("DOM ERROR", error) }
        print("Timeout state", current, web.url?.path ?? "nil", coordinator.loaded?.path ?? "nil", coordinator.networkRulesReady, web.isLoading, status)
        throw NSError(domain: "ReaderNavigationTests", code: 1, userInfo: [NSLocalizedDescriptionKey: "Timed out: \(name)"])
    }
    func at(_ resource: String) async throws -> Bool {
        guard current == resource, !web.isLoading, web.url?.lastPathComponent == resource else { return false }
        return (try? await evaluate("document.body.dataset.chapter")) as? String == resource
    }
    func run() async throws {
        root = FileManager.default.temporaryDirectory.appendingPathComponent("BookNookNavigation-\(UUID())")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        for (index, name) in spine.enumerated() {
            let next = index < 2 ? "<a id='next' href='\(spine[index + 1])#target'>Next chapter</a>" : ""
            let html = "<html xmlns='http://www.w3.org/1999/xhtml'><body data-chapter='\(name)'>\(next)<a id='external' href='https://example.invalid/'>External</a><a id='escape' href='../outside.xhtml'>Outside asset</a><a id='unknown' href='not-in-spine.xhtml'>Unlisted file</a><a id='missing' href='#missing-target'>Missing fragment</a>" + String(repeating: "<p>A long synthetic paragraph gives this chapter a real scroll position.</p>", count: 45) + "<p id='target'>Unique passage in \(name)</p></body></html>"
            try html.write(to: root.appendingPathComponent(name), atomically: true, encoding: .utf8)
        }
        coordinator = ReaderCoordinator { [weak self] message in
            guard let self else { return }
            switch message["type"] as? String {
            case "navigate":
                guard let resource = message["resource"] as? String else { return }
                DispatchQueue.main.async { self.load(resource, destination: ReaderDestination(resource: resource, fragment: message["fragment"] as? String, anchor: nil)) }
            case "error": self.status = message["message"] as? String ?? ""
            case "navigation_status": self.status = message["message"] as? String ?? ""; self.navigationMessages.append(self.status)
            case "selection": self.selectionResource = self.current
            default: break
            }
        }
        let scripts = try ["reader-geometry", "reader"].map { try String(contentsOfFile: "apps/apple/Resources/\($0).js", encoding: .utf8) }
        web = makeWebView(coordinator, scriptSources: scripts); web.frame = CGRect(x: 0, y: 0, width: 640, height: 480)
        // Observe real WebKit requests, forwarding every decision to the actual
        // production coordinator rather than substituting a policy in the test.
        web.navigationDelegate = self
        load(spine[0]); try await wait("first chapter") { try await self.at(self.spine[0]) }
        precondition(navigationMessages.isEmpty, "Initial native chapter load must not claim the reader followed an unsupported link")
        web.load(URLRequest(url: URL(string: "about:blank")!))
        try await wait("programmatic blank rejected by production navigation policy") { self.cancelledProgrammaticBlank }
        try await wait("original chapter retained after rejected programmatic navigation") { try await self.at(self.spine[0]) }
        precondition(navigationMessages.isEmpty, "A blocked programmatic request must not claim the reader followed an unsupported link")
        print("Reader navigation: native chapter loaded without link warning; actual programmatic about:blank was cancelled without a link warning and original document retained.")
        _ = try await evaluate("document.getElementById('next').click(); true")
        try await wait("second chapter and fragment") { try await self.at(self.spine[1]) }
        try await wait("fragment scroll position") { (try await self.evaluate("scrollY") as? Double ?? 0) > 0 }
        _ = try await evaluate("(()=>{const found=booknookResolve({quote:'Unique passage in two.xhtml'});const s=getSelection();s.removeAllRanges();s.addRange(found.range);return true})()")
        try await wait("selection attributed to native chapter") { self.selectionResource == self.spine[1] }
        _ = try await evaluate("document.getElementById('next').click(); true")
        try await wait("third chapter") { try await self.at(self.spine[2]) }
        for link in ["external", "escape", "unknown"] {
            status = ""; _ = try await evaluate("document.getElementById('\(link)').click(); true")
            try await wait("visible unsupported-link state") { !self.status.isEmpty }
            precondition(current == spine[2] && web.url?.lastPathComponent == spine[2] && web.url?.isFileURL == true)
        }
        status = ""; _ = try await evaluate("document.getElementById('missing').click(); true")
        try await wait("missing fragment feedback") { self.status.contains("could not be found") }
        load(spine[0], destination: ReaderDestination(resource: spine[0], fragment: nil, anchor: ["quote": "Unique passage in one.xhtml"]))
        try await wait("annotation jump to earlier chapter") { try await self.at(self.spine[0]) }
        try await wait("annotation scroll position") { (try await self.evaluate("scrollY") as? Double ?? 0) > 0 }
        let originalReceiver = coordinator.onMessage
        var obsoleteWarnings = 0
        var replacementWarnings = 0
        coordinator.onMessage = { _ in obsoleteWarnings += 1 }
        coordinator.destination = ReaderDestination(resource: spine[0], fragment: "missing-before-switch", anchor: nil)
        coordinator.applyDestination(in: web)
        // Change native context before WebKit can return the missing-target result.
        coordinator.destination = nil
        coordinator.onMessage = { _ in replacementWarnings += 1 }
        _ = try await evaluate("true")
        precondition(obsoleteWarnings == 0 && replacementWarnings == 0, "Superseded navigation must not emit a warning into either context")
        coordinator.onMessage = { _ in obsoleteWarnings += 1 }
        coordinator.destination = ReaderDestination(resource: spine[0], fragment: "missing-same-context", anchor: nil)
        coordinator.applyDestination(in: web)
        coordinator.onMessage = { _ in replacementWarnings += 1 }
        try await wait("destination callback uses captured receiver") { obsoleteWarnings == 1 }
        precondition(replacementWarnings == 0, "An asynchronous result must retain its original receiver")
        coordinator.onMessage = originalReceiver
        coordinator.destination = nil
        let one = ReaderHighlightRequest(anchor: ["quote": "café 日本語", "kind": "text", "edition_id": "edition", "resource": "one.xhtml"], geometry: nil, context: "café 日本語")
        let reordered = ReaderHighlightRequest(anchor: ["resource": "one.xhtml", "edition_id": "edition", "kind": "text", "quote": "café 日本語"], geometry: nil, context: "café 日本語")
        let another = ReaderHighlightRequest(anchor: ["quote": "café 日本語", "kind": "text", "edition_id": "edition", "resource": "two.xhtml"], geometry: nil, context: "café 日本語")
        precondition(one.key == reordered.key && one.key != another.key)
        print("Reader navigation: actual three-chapter EPUB links/fragments, native selection attribution, annotation return, explicit external/escape/non-spine rejection feedback, missing-target feedback, and stable highlight deduplication keys passed.")
    }
}
@main struct ReaderNavigationTests {
    @MainActor static func main() {
        let app = NSApplication.shared; app.setActivationPolicy(.prohibited)
        let harness = ReaderNavigationHarness()
        Task { do { try await harness.run(); exit(0) } catch { fputs("Reader navigation failed: \(error)\n", stderr); exit(1) } }
        app.run()
    }
}
