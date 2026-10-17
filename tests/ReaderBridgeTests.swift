// SPDX-License-Identifier: MPL-2.0
// Headless WKWebView contract test. It never opens a window or uses a user app.
import AppKit
import WebKit

final class ReaderBridgeCheck: NSObject, WKNavigationDelegate, WKScriptMessageHandler {
    var web: WKWebView!
    var measurements = 0
    func run() throws {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        config.defaultWebpagePreferences.allowsContentJavaScript = false
        config.userContentController.add(self, name: "booknook")
        for name in ["reader-geometry", "reader"] {
            let source = try String(contentsOfFile: "apps/apple/Resources/\(name).js", encoding: .utf8)
            config.userContentController.addUserScript(WKUserScript(source: source, injectionTime: .atDocumentEnd, forMainFrameOnly: true))
        }
        web = WKWebView(frame: CGRect(x: 0,y: 0,width: 640,height: 800),configuration: config)
        web.navigationDelegate = self
        let html = "<html><head><meta name='viewport' content='width=device-width, initial-scale=1'></head><body><script>window.untrustedBookScriptRan = true;</script><p>Before 😀 café: the quiet library welcomed Mara with open doors and many stories to discover in the late afternoon light.</p><p>Duplicate passage.</p><p>Duplicate passage.</p><p>Distinct ending.</p>" + String(repeating: "<p>A synthetic paragraph for reading-position restoration.</p>", count: 80) + "<p>Last chapter marker.</p></body></html>"
        web.loadHTMLString(html,baseURL: nil)
        DispatchQueue.main.asyncAfter(deadline: .now()+15) { fputs("Reader bridge timed out\n",stderr); exit(1) }
    }
    func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) { if let body = message.body as? [String: Any], body["type"] as? String == "geometry" { measurements += 1 } }
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        let script = #"""
        (()=>{
          const checks=[]; const check=(ok,name)=>{if(!ok)throw new Error(name);checks.push(name)};
          check(!window.untrustedBookScriptRan,'book scripts are disabled');
          check(document.querySelectorAll('meta[name=viewport]').length===1 && document.querySelector('meta[name=viewport]').content==='width=device-width,initial-scale=1','reader viewport follows pane width without disabling zoom');
          check(getComputedStyle(document.body).backgroundColor==='rgb(247, 242, 229)','consistent paper background');
          const colored=document.querySelector('p');colored.style.backgroundColor='red';colored.style.color='yellow';
          check(getComputedStyle(colored).backgroundColor==='rgba(0, 0, 0, 0)' && getComputedStyle(colored).color==='rgb(41, 63, 56)','source text colors cannot switch the reading surface');
          const unique=booknookResolve({quote:'😀 café',prefix:'Before ',suffix:': the quiet'});
          check(unique.status==='resolved' && unique.range.toString()==='😀 café','Unicode quote and DOM range');
          check(booknookResolve({quote:'Duplicate passage.'}).status==='ambiguous','duplicate quote is never guessed');
          check(booknookResolve({quote:'absent'}).status==='missing','missing quote is retained unresolved');
          const first=document.querySelector('p').getBoundingClientRect();
          const saved=booknookInkAnchor({x:first.x+scrollX,y:first.y+scrollY,width:first.width,height:first.height},'one.xhtml');
          check(saved.status==='resolved' && saved.geometry.text_rects.length>0,'measured ink anchor');
          const same=booknookInkAnchor({x:first.x+scrollX,y:first.y+scrollY,width:first.width,height:first.height},'one.xhtml');
          check(BookNookGeometry.resolveInk(saved.geometry,same.geometry).status==='resolved','unchanged ink placement');
          booknookAnnotations([{id:'portable',anchor:saved.anchor,ink_geometry:saved.geometry,render_portable:true,strokes:[{points:[{x:40,y:50},{x:70,y:60}],color:'#c02080',width:3}]}]);
          const portableLine=document.querySelector('[data-booknook-overlay] polyline');
          check(portableLine?.getAttribute('stroke')==='#c02080' && portableLine?.getAttribute('points')==='40,50 70,60','portable ink and color without native drawing');
          document.body.style.setProperty('max-width','13em','important');
          const changed=document.querySelector('p').getBoundingClientRect();
          const moved=booknookInkAnchor({x:changed.x+scrollX,y:changed.y+scrollY,width:changed.width,height:changed.height},'one.xhtml');
          check(BookNookGeometry.resolveInk(saved.geometry,moved.geometry).status==='changed_layout','reflow retains marker');
          booknookAnnotations([{id:'ink',anchor:saved.anchor,ink_geometry:saved.geometry,render_portable:true,strokes:[{points:[{x:40,y:50},{x:70,y:60}],color:'#4d7050',width:3}]}]);
          check(document.querySelector('[data-booknook-overlay] span')!==null,'ambiguity marker is visible');
          check(booknookResolve(saved.anchor).status==='resolved','overlay text excluded from anchors');
          booknookSetFontSize(25);
          check(getComputedStyle(document.body).fontSize==='25px','native font-size control');
          booknookRestorePosition({quote:'Last chapter marker.'});
          check(scrollY>0,'exact source position restoration');
          const beforeMissing=scrollY;booknookRestorePosition({quote:'No such passage.'});
          check(scrollY===beforeMissing,'missing position does not jump');
          return checks;
        })()
        """#
        webView.evaluateJavaScript(script) { result, error in
            if let error { fputs("Reader bridge failed: \(error)\n",stderr); exit(1) }
            guard let checks = result as? [String], checks.count == 16 else { fputs("Reader bridge returned an unexpected result: \(String(describing: result))\n",stderr); exit(1) }
            print("WKWebView reader bridge: \(checks.joined(separator: "; ")).")
            exit(0)
        }
    }
}
@main struct ReaderBridgeTests {
    static func main() throws {
        let app = NSApplication.shared; app.setActivationPolicy(.prohibited)
        let check = ReaderBridgeCheck(); try check.run(); app.run()
    }
}
