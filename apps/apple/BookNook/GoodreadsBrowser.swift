// SPDX-License-Identifier: MPL-2.0
import SwiftUI
import WebKit
import Combine

/// A website, not a provider: no notebook bridge, scraping or credential export.
@MainActor final class GoodreadsBrowserSession: NSObject, ObservableObject, WKNavigationDelegate, WKUIDelegate {
    static let home = URL(string: "https://www.goodreads.com/")!
    // Dedicated persistent cookies; never share the reader or Kindle import store.
    static let storeID = UUID(uuidString: "47C8F71F-B13C-4C82-9FB7-478B19109C79")!
    let webView: WKWebView
    @Published private(set) var currentURL: URL?
    @Published private(set) var canGoBack = false
    @Published private(set) var canGoForward = false
    @Published private(set) var isLoading = false
    @Published private(set) var progress = 0.0
    @Published private(set) var notice: String?
    private var observations: [NSKeyValueObservation] = []
    private var started = false
    private var closed = false

    init(dataStore: WKWebsiteDataStore? = nil) {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = dataStore ?? .init(forIdentifier: Self.storeID)
        configuration.preferences.javaScriptCanOpenWindowsAutomatically = false
        webView = WKWebView(frame: .zero, configuration: configuration)
        super.init()
        webView.navigationDelegate = self
        webView.uiDelegate = self
        webView.allowsBackForwardNavigationGestures = true
        webView.isInspectable = false
        #if os(macOS)
        webView.underPageBackgroundColor = NSColor(red: 247.0 / 255, green: 242.0 / 255, blue: 229.0 / 255, alpha: 1)
        #else
        webView.underPageBackgroundColor = UIColor(red: 247.0 / 255, green: 242.0 / 255, blue: 229.0 / 255, alpha: 1)
        #endif
        observations = [
            webView.observe(\.url, options: [.new]) { [weak self] _, _ in Task { @MainActor in self?.refresh() } },
            webView.observe(\.canGoBack, options: [.new]) { [weak self] _, _ in Task { @MainActor in self?.refresh() } },
            webView.observe(\.canGoForward, options: [.new]) { [weak self] _, _ in Task { @MainActor in self?.refresh() } },
            webView.observe(\.isLoading, options: [.new]) { [weak self] _, _ in Task { @MainActor in self?.refresh() } },
            webView.observe(\.estimatedProgress, options: [.new]) { [weak self] _, _ in Task { @MainActor in self?.refresh() } }
        ]
    }

    static func allows(_ url: URL) -> Bool {
        url.scheme?.lowercased() == "https" && url.host?.isEmpty == false &&
            url.user == nil && url.password == nil && (url.port == nil || url.port == 443)
    }
    func start() {
        guard !started, !closed else { return }
        started = true
        webView.load(URLRequest(url: Self.home))
    }
    func back() { if !closed { webView.goBack() } }
    func forward() { if !closed { webView.goForward() } }
    func reload() {
        guard !closed else { return }
        notice = nil
        if webView.url == nil { webView.load(URLRequest(url: Self.home)) }
        else { webView.reload() }
    }
    func close() {
        guard !closed else { return }
        closed = true
        observations.removeAll()
        webView.stopLoading()
        webView.navigationDelegate = nil
        webView.uiDelegate = nil
    }
    private func refresh() {
        guard !closed else { return }
        currentURL = webView.url
        canGoBack = webView.canGoBack
        canGoForward = webView.canGoForward
        isLoading = webView.isLoading
        progress = webView.estimatedProgress
    }
    func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction, decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
        guard !closed, let url = action.request.url else { decisionHandler(.cancel); return }
        // Empty inline frames are used by ordinary websites, including sign-in.
        if action.targetFrame?.isMainFrame == false && ["about:blank", "about:srcdoc"].contains(url.absoluteString) {
            decisionHandler(.allow); return
        }
        guard Self.allows(url) else {
            if action.targetFrame?.isMainFrame != false { notice = "Only secure web pages can open here." }
            decisionHandler(.cancel); return
        }
        if action.shouldPerformDownload {
            notice = "To download a file, open Goodreads in your default browser. You can import a Goodreads CSV from the library menu."
            decisionHandler(.cancel); return
        }
        decisionHandler(.allow)
    }
    func webView(_ webView: WKWebView, decidePolicyFor response: WKNavigationResponse, decisionHandler: @escaping (WKNavigationResponsePolicy) -> Void) {
        guard !closed else { decisionHandler(.cancel); return }
        if response.isForMainFrame, let url = response.response.url, !Self.allows(url) {
            notice = "Only secure web pages can open here."
            decisionHandler(.cancel); return
        }
        if response.isForMainFrame, !response.canShowMIMEType {
            notice = "This file needs your default browser. Goodreads CSV files can then be imported from the library menu."
            decisionHandler(.cancel); return
        }
        decisionHandler(.allow)
    }
    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration, for action: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
        // User-initiated new-window links stay in this sheet, using its own store.
        if !closed, action.targetFrame == nil, let url = action.request.url, Self.allows(url) {
            webView.load(action.request)
        }
        return nil
    }
    func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) { notice = nil; refresh() }
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) { refresh() }
    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) { failed(error) }
    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) { failed(error) }
    private func failed(_ error: Error) {
        guard !closed else { return }
        let nsError = error as NSError
        guard nsError.domain != NSURLErrorDomain || nsError.code != NSURLErrorCancelled else { return }
        // Do not display sign-in URLs, server bodies or redirect query tokens.
        notice = "The page couldn’t load. Try reloading, or open Goodreads in your default browser."
        refresh()
    }
    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
        notice = "The browser paused unexpectedly. Reload to continue."
        refresh()
    }
}

struct GoodreadsBrowser: View {
    @Environment(\.dismiss) private var dismiss
    @StateObject private var session = GoodreadsBrowserSession()
    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("Goodreads").font(.system(.title3, design: .serif)).bold()
                Spacer()
                Button("Done") { dismiss() }.keyboardShortcut(.cancelAction)
                    .accessibilityIdentifier("goodreads-done")
            }.padding(.horizontal, 18).padding(.top, 12)
            HStack(spacing: 4) {
                browserButton("Back", icon: "chevron.left", action: session.back).disabled(!session.canGoBack)
                browserButton("Forward", icon: "chevron.right", action: session.forward).disabled(!session.canGoForward)
                browserButton("Reload page", icon: "arrow.clockwise", action: session.reload)
                Text(session.currentURL?.host ?? "www.goodreads.com")
                    .font(.caption).lineLimit(1).truncationMode(.middle).frame(maxWidth: .infinity)
                    .accessibilityLabel("Website: \(session.currentURL?.host ?? "www.goodreads.com")")
                    .accessibilityIdentifier("goodreads-host")
                // A fixed homepage never forwards sign-in callback tokens to another app.
                Link(destination: GoodreadsBrowserSession.home) {
                    Image(systemName: "arrow.up.right.square").frame(width: 44, height: 44)
                }.accessibilityLabel("Open Goodreads in default browser")
                    .help("Open Goodreads in your default browser")
            }.buttonStyle(.borderless).padding(.horizontal, 8)
            ProgressView(value: session.progress).progressViewStyle(.linear)
                .opacity(session.isLoading ? 1 : 0).frame(height: 3).accessibilityHidden(true)
            if let notice = session.notice {
                Text(notice).font(.callout).frame(maxWidth: .infinity, alignment: .leading)
                    .padding(12).accessibilityIdentifier("goodreads-browser-notice")
            }
            GoodreadsWebSurface(web: session.webView)
                .accessibilityIdentifier("goodreads-web-content")
        }.foregroundStyle(nookInk).background(nookPaper).tint(nookAccent)
        #if os(macOS)
        .frame(minWidth: 520, idealWidth: 780, minHeight: 420, idealHeight: 620)
        #else
        .frame(minWidth: 280, minHeight: 380)
        .presentationDetents([.large])
        #endif
        .task { session.start() }
        .onDisappear { session.close() }
    }
    private func browserButton(_ title: String, icon: String, action: @escaping () -> Void) -> some View {
        Button(action: action) { Image(systemName: icon).frame(width: 44, height: 44) }
            .accessibilityLabel(title).help(title)
    }
}

#if os(macOS)
private struct GoodreadsWebSurface: NSViewRepresentable {
    let web: WKWebView
    func makeNSView(context: Context) -> WKWebView { web }
    func updateNSView(_ view: WKWebView, context: Context) {}
}
#else
private struct GoodreadsWebSurface: UIViewRepresentable {
    let web: WKWebView
    func makeUIView(context: Context) -> WKWebView { web }
    func updateUIView(_ view: WKWebView, context: Context) {}
}
#endif
