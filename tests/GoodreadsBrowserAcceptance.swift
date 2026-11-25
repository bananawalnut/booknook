// SPDX-License-Identifier: MPL-2.0
import AppKit
import SwiftUI
import WebKit

// Standalone linkage for the sheet's app-level palette aliases.
let nookInk = Color.primary
let nookPaper = Color.secondary
let nookAccent = Color.accentColor

private enum Failure: Error { case check(String) }

@main @MainActor
private struct Acceptance {
    static func wait(_ label: String, until condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(10)
        while !condition() {
            guard ContinuousClock.now < deadline else { throw Failure.check(label) }
            try await Task.sleep(for: .milliseconds(25))
        }
    }

    static func exercise(_ id: UUID) async throws {
        let session = GoodreadsBrowserSession(dataStore: .init(forIdentifier: id))
        defer { session.close() }
        let url = URL(string: "https://fixture.invalid/book")!
        session.webView.loadSimulatedRequest(URLRequest(url: url), responseHTML: "<title>Book fixture</title>")
        try await wait("initial page") { session.webView.title == "Book fixture" && !session.isLoading }
        _ = try await session.webView.evaluateJavaScript("history.pushState({}, '', '/shelf'); 'ok'")
        try await wait("history observation") { session.currentURL?.path == "/shelf" && session.canGoBack }
        session.back()
        try await wait("back") { session.currentURL == url && session.canGoForward }
        session.forward()
        try await wait("forward") { session.currentURL?.path == "/shelf" }
        let unsafe = URL(string: "http://fixture.invalid/?token=private-test-marker")!
        session.webView.load(URLRequest(url: unsafe))
        try await wait("unsafe navigation denial") { session.notice == "Only secure web pages can open here." }
        guard session.currentURL?.path == "/shelf", session.notice?.contains("private-test-marker") == false,
              !GoodreadsBrowserSession.allows(URL(string: "https://user@fixture.invalid/")!) else {
            throw Failure.check("unsafe navigation or notice")
        }
        let cookie = HTTPCookie(properties: [.name: "acceptance", .value: id.uuidString,
                                              .domain: "fixture.invalid", .path: "/",
                                              .expires: Date().addingTimeInterval(3600)])!
        await session.webView.configuration.websiteDataStore.httpCookieStore.setCookie(cookie)
    }

    static func checkReopened(_ id: UUID) async throws {
        let session = GoodreadsBrowserSession(dataStore: .init(forIdentifier: id))
        defer { session.close() }
        let reopened = await session.webView.configuration.websiteDataStore.httpCookieStore.allCookies()
        let separate = await WKWebsiteDataStore.nonPersistent().httpCookieStore.allCookies()
        guard reopened.contains(where: { $0.name == "acceptance" && $0.value == id.uuidString }),
              !separate.contains(where: { $0.value == id.uuidString }) else {
            throw Failure.check("website data isolation or session persistence")
        }
    }

    static func checkPresentation(available: CGSize) async throws {
        let session = GoodreadsBrowserSession(dataStore: .nonPersistent())
        let browser = GoodreadsBrowser(session: session, startAtHome: false, availableSpace: available)
        let hosted = NSHostingView(rootView: browser.environment(\.accessibilityEnabled, true))
        let fit = hosted.fittingSize
        guard fit.width <= available.width, fit.height <= available.height, fit.width > 0, fit.height > 0 else {
            throw Failure.check("sheet cannot fit available space: \(fit) in \(available)")
        }
        let window = NSWindow(contentRect: CGRect(origin: .zero, size: fit), styleMask: [.borderless],
                              backing: .buffered, defer: false)
        window.contentView = hosted
        defer { window.contentView = nil; session.close() }
        hosted.layoutSubtreeIfNeeded()
        let fixture = "<style>*{margin:0}article{width:900px}</style><title>Shelf layout</title><article>Fixture shelf</article>"
        session.webView.loadSimulatedRequest(URLRequest(url: URL(string: "https://fixture.invalid/layout")!), responseHTML: fixture)
        try await wait("sheet content laid out") { session.webView.title == "Shelf layout" && session.webView.bounds.width > 0 }
        let metrics = try await session.webView.evaluateJavaScript("[innerWidth, document.documentElement.scrollWidth]")
        guard let values = metrics as? [NSNumber], values.count == 2 else { throw Failure.check("missing webpage geometry") }
        let viewport = values[0].doubleValue, document = values[1].doubleValue
        guard abs(viewport - session.webView.bounds.width) <= 2,
              available.width >= 900 ? viewport >= 900 && document <= viewport : viewport > 0 && viewport < 900 && document >= 900 else {
            throw Failure.check("web viewport \(viewport), document \(document), view \(session.webView.bounds), available \(available)")
        }

        var seen = Set<ObjectIdentifier>()
        func doneFrame(_ node: NSObject) -> CGRect? {
            guard seen.insert(ObjectIdentifier(node)).inserted else { return nil }
            func attribute(_ key: String) -> Any? {
                node.responds(to: NSSelectorFromString(key)) ? node.value(forKey: key) : nil
            }
            if attribute("accessibilityIdentifier") as? String == "goodreads-done" {
                return (attribute("accessibilityFrame") as? NSValue)?.rectValue
            }
            for child in attribute("accessibilityChildren") as? [NSObject] ?? [] {
                if let found = doneFrame(child) { return found }
            }
            if let view = node as? NSView {
                for child in view.subviews { if let found = doneFrame(child) { return found } }
            }
            return nil
        }
        let bounds = window.convertToScreen(hosted.convert(hosted.bounds, to: nil))
        let done = doneFrame(window) ?? doneFrame(hosted)
        guard let done, !done.isEmpty, bounds.insetBy(dx: -1, dy: -1).contains(done) else {
            throw Failure.check("Done frame \(String(describing: done)), sheet \(bounds)")
        }
        print("Goodreads sheet \(Int(fit.width))×\(Int(fit.height)): web viewport \(Int(viewport)), document \(Int(document)), Done visible")
    }

    static func main() async {
        _ = NSApplication.shared.setActivationPolicy(.prohibited)
        let args = CommandLine.arguments
        guard args.count == 3, let id = UUID(uuidString: args[2]), id != GoodreadsBrowserSession.storeID else { exit(2) }
        do {
            switch args[1] {
            case "exercise":
                try await checkPresentation(available: CGSize(width: 1440, height: 900))
                try await checkPresentation(available: CGSize(width: 620, height: 420))
                try await exercise(id)
            case "reopen": try await checkReopened(id)
            case "cleanup":
                // Initialize WebKit's process pool before its class-level deletion API.
                let bootstrap = GoodreadsBrowserSession(dataStore: .nonPersistent())
                defer { bootstrap.close() }
                _ = await bootstrap.webView.configuration.websiteDataStore.httpCookieStore.allCookies()
                let deadline = ContinuousClock.now + .seconds(10)
                while true {
                    do { try await WKWebsiteDataStore.remove(forIdentifier: id); break }
                    catch {
                        guard ContinuousClock.now < deadline else { throw error }
                        try await Task.sleep(for: .milliseconds(200))
                    }
                }
            default: exit(2)
            }
        } catch {
            FileHandle.standardError.write(Data("Goodreads \(args[1]) failed: \(error)\n".utf8))
            exit(1)
        }
    }
}
