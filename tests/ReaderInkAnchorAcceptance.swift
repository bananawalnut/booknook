// SPDX-License-Identifier: MPL-2.0
import AppKit
import WebKit

/// Separate event-loop mode: no window, activation, pointer or keyboard input.
/// Exercises the exact production anchor resolver against real WKWebView DOM.
@MainActor final class ReaderInkAnchorHarness: NSObject, WKNavigationDelegate {
    var web: WKWebView!
    var source: URL!
    var phase = 0
    func run() throws {
        source = FileManager.default.temporaryDirectory.appendingPathComponent("booknook-reader-anchor-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: source, withIntermediateDirectories: true)
        let chapter = source.appendingPathComponent("one.xhtml")
        try "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>Original synthetic chapter</title></head><body><p>The original café passage remains in this source.</p><p>Another paragraph follows the original.</p></body></html>".write(to: chapter, atomically: true, encoding: .utf8)
        let config = WKWebViewConfiguration(); config.websiteDataStore = .nonPersistent()
        config.defaultWebpagePreferences.allowsContentJavaScript = false
        for name in ["reader-geometry", "reader"] {
            let script = try String(contentsOfFile: "apps/apple/Resources/\(name).js", encoding: .utf8)
            config.userContentController.addUserScript(WKUserScript(source: script, injectionTime: .atDocumentEnd, forMainFrameOnly: true))
        }
        web = WKWebView(frame: CGRect(x: 0, y: 0, width: 640, height: 800), configuration: config)
        web.navigationDelegate = self
        fputs("Reader ink anchor: starting isolated file load\n", stderr)
        web.loadFileURL(chapter, allowingReadAccessTo: source)
        DispatchQueue.global().asyncAfter(deadline: .now() + 25) { fputs("Reader ink anchor acceptance timed out\n", stderr); exit(1) }
    }
    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) { fputs("Reader ink file navigation failed: \(error)\n", stderr); exit(1) }
    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) { fputs("Reader ink navigation failed: \(error)\n", stderr); exit(1) }
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        guard phase == 0 else { return }; phase = 1
        fputs("Reader ink anchor: didFinish; resolving actual DOM\n", stderr)
        Task { @MainActor in
            do {
                let value = try await webView.evaluateJavaScript("""
                (()=>{const r=document.querySelector('p').getBoundingClientRect();const b={x:r.x+scrollX,y:r.y+scrollY,width:r.width,height:r.height};
                const a=booknookInkAnchor(b,'one.xhtml');return {bounds:b,url:location.href,font:parseFloat(getComputedStyle(document.body).fontSize),fingerprint:a.geometry.content_fingerprint,
                layout:[location.pathname,innerWidth,innerHeight,getComputedStyle(document.body).fontSize,document.documentElement.scrollHeight,a.geometry.content_fingerprint].join('|')};})()
                """) as! [String: Any]
                let b = value["bounds"] as! [String: Double]
                let context = ReaderInkContext(resource: "one.xhtml", documentURL: value["url"] as? String,
                    contentFingerprint: value["fingerprint"] as? String, layoutFingerprint: value["layout"] as? String)
                let bounds = CGRect(x: b["x"]!, y: b["y"]!, width: b["width"]!, height: b["height"]!)
                let draft = ReaderInkDraft(id: "probe", context: context, bounds: bounds, strokes: [], nativeDrawing: "AQID")
                let resolved = try await ReaderInkAnchorResolver.resolve(draft, in: webView)
                precondition(resolved.anchor["resource"] as? String == "one.xhtml" && resolved.originalContext.contains("original café"))
                let wrong = ReaderInkContext(resource: "one.xhtml", documentURL: context.documentURL! + ".different", contentFingerprint: context.contentFingerprint, layoutFingerprint: context.layoutFingerprint)
                do {
                    _ = try await ReaderInkAnchorResolver.resolve(ReaderInkDraft(id: "wrong", context: wrong, bounds: bounds, strokes: [], nativeDrawing: "AQID"), in: webView)
                    preconditionFailure("A different source URL must never receive this captured ink")
                } catch { precondition(bookNookErrorMessage(error).contains("original passage and layout")) }
                let originalFont = (value["font"] as! NSNumber).doubleValue
                _ = try await webView.evaluateJavaScript("booknookSetFontSize(\(originalFont + 9)); true")
                do { _ = try await ReaderInkAnchorResolver.resolve(draft, in: webView); preconditionFailure("Changed layout must reject old coordinates") }
                catch { precondition(bookNookErrorMessage(error).contains("original passage and layout")) }
                _ = try await webView.evaluateJavaScript("booknookSetFontSize(\(originalFont)); true")
                let retry = try await ReaderInkAnchorResolver.resolve(draft, in: webView)
                precondition(retry.originalContext == resolved.originalContext)
                // Missing/unplaced locations are not passed into the text renderer.
                _ = try await webView.evaluateJavaScript("booknookAnnotations([{id:'unplaced',anchor:{kind:'provider',provider:'booknook.reader.unplaced',source_id:'asset',value:'{}'},legacy_ink:true,strokes:[]}]);true")
                let overlays = try await webView.evaluateJavaScript("document.querySelectorAll('[data-booknook-overlay] polyline,[data-booknook-overlay] span').length") as? Int
                precondition(overlays == 0)
                try? FileManager.default.removeItem(at: source)
                print("Reader ink actual WKWebView: original source/layout resolves; wrong source and changed font layout reject; original layout retry resolves; unplaced provider markup never acquires a text placement. No window or UI input.")
                exit(0)
            } catch { fputs("Reader ink anchor failed: \(error)\n", stderr); exit(1) }
        }
    }
}
