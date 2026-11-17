// SPDX-License-Identifier: MPL-2.0
// Headless WKWebView contract test. It never opens a window or uses a user app.
import AppKit
import WebKit

final class ReaderBridgeCheck: NSObject, WKNavigationDelegate, WKScriptMessageHandler {
    var web: WKWebView!
    var measurements = 0
    private var highlightSnapshots: [[String: String]] = []
    private var checkpoints: [String: Int] = [:]
    private var domChecks: [String]?
    private var latestGeometry: [String: Any]?
    private var checkingResume = false
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
        let html = "<html><head><meta name='viewport' content='width=device-width, initial-scale=1'></head><body><script>window.untrustedBookScriptRan = true;</script><p>Before 😀 café: the quiet library welcomed Mara with open doors and many stories to discover in the late afternoon light.</p><p>Duplicate passage.</p><p>Duplicate passage.</p><p hidden>Unique hidden quote.</p><p>Distinct ending.</p>" + String(repeating: "<p>A synthetic paragraph for reading-position restoration.</p>", count: 80) + "<p>Last chapter marker.</p>" +
            "<p id='long-run'>" + (0..<440).map { "Cairn-\($0): 😀 café   stones rest beneath the window, 日本語." }.joined(separator: " ") +
            "</p><p id='repeated-run'>" + String(repeating: "Répété 😀 sous le même ciel. ", count: 2200) +
            "</p></body></html>"
        web.loadHTMLString(html,baseURL: nil)
        DispatchQueue.main.asyncAfter(deadline: .now()+15) { fputs("Reader bridge timed out\n",stderr); exit(1) }
    }
    func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
        guard let body = message.body as? [String: Any] else { return }
        switch body["type"] as? String {
        case "geometry":
            measurements += 1
            latestGeometry = body
        case "highlight_placements":
            guard let placements = body["placements"] as? [String: String] else {
                fputs("Expected a dictionary of highlight placement statuses\n", stderr); exit(1)
            }
            highlightSnapshots.append(placements)
        case "bridge_test_checkpoint":
            guard let name = body["name"] as? String else { return }
            checkpoints[name] = highlightSnapshots.count
            finishIfReady()
        default: break
        }
    }

    private func finishIfReady() {
        guard let domChecks, checkpoints["cleared"] != nil, !checkingResume else { return }
        let expected = ["opening": "resolved", "ending": "resolved", "ambiguous": "ambiguous",
                        "missing": "missing", "hidden": "not_visible"]
        let expectedCheckpoints = ["registry": 1, "fallback": 3, "reflow": 3,
                                   "reordered": 3, "scrolled": 3, "cleared": 4]
        guard highlightSnapshots == [expected, [:], expected, [:]], checkpoints == expectedCheckpoints else {
            fputs("Unexpected highlight placement snapshots: \(highlightSnapshots); checkpoints: \(checkpoints)\n", stderr)
            exit(1)
        }
        print("WKWebView reader bridge: \(domChecks.joined(separator: "; ")).")
        print("Native bridge received exact per-ID placement dictionaries for both render paths, no repeats after reflow/reorder/scroll, and one empty snapshot on removal.")
        checkingResume = true
        scrollAndCapture("Cairn-100:") { first in
            self.scrollAndCapture("Cairn-280:") { second in
                self.verifyResume(first, second)
            }
        }
    }

    private func scrollAndCapture(_ token: String, receive: @escaping ([String: Any]) -> Void) {
        web.callAsyncJavaScript("""
            const rect = booknookResolve({quote: token}).rects[0];
            if (!rect) throw new Error("Missing fixture token: " + token);
            scrollTo({top: rect.y - 80, behavior: "instant"});
            dispatchEvent(new Event("scroll"));
            return scrollY;
            """, arguments: ["token": token], in: nil, in: .page) { result in
            guard case .success(let value) = result, let target = value as? Double else {
                fputs("Could not scroll to fixture token \(token): \(result)\n", stderr); exit(1)
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.4) {
                guard let y = (self.latestGeometry?["viewport"] as? [String: Any])?["y"] as? Double,
                      abs(y - target) < 2,
                      let anchor = self.latestGeometry?["position"] as? [String: Any] else {
                    fputs("Expected native position for fixture token \(token) at \(target)\n", stderr); exit(1)
                }
                receive(["anchor": anchor, "scroll": target])
            }
        }
    }

    private func verifyResume(_ first: [String: Any], _ second: [String: Any]) {
        let script = #"""
        const check = (ok, name) => { if (!ok) throw new Error(name); };
        const validUTF16 = text => Array.from(text).every(char => {
          const cp = char.codePointAt(0);
          return cp < 0xd800 || cp > 0xdfff;
        });
        check(first.anchor.quote !== second.anchor.quote, 'two interior viewports must produce distinct quotes');
        for (const sample of [first, second]) {
          const {anchor, scroll} = sample;
          check(anchor.quote.length <= 72 && anchor.quote.trim().length > 0 &&
            [anchor.quote,anchor.prefix,anchor.suffix].every(validUTF16),
            'passive position is a bounded valid Unicode quote with exact context');
          const match = booknookResolve(anchor);
          check(match.status === 'resolved' && Math.abs(match.rects[0].y - scroll) < 120,
            'the quote starts at the measured viewport of the original long text node');
          check(match.range.startContainer.parentElement.id === 'long-run', 'both positions are in the same text node');
          scrollTo(0, 0);
          check(booknookRestorePosition(anchor).status === 'resolved' && Math.abs(scrollY - scroll) < 80,
            'reopening returns to the captured interior line');
        }
        booknookSetFontSize(29);
        const beforeReflowRestore = booknookResolve(second.anchor);
        check(beforeReflowRestore.status === 'resolved', 'the saved quote survives font reflow');
        scrollTo(0, 0);
        check(booknookRestorePosition(second.anchor).status === 'resolved' &&
          Math.abs(beforeReflowRestore.rects[0].y - scrollY) < 120, 'reopening after reflow returns to the saved text');
        const fullRun = document.querySelector('#long-run').textContent;
        scrollTo(0, 0);
        check(booknookRestorePosition({quote:fullRun}).status === 'resolved' &&
          Math.abs(booknookResolve({quote:fullRun}).rects[0].y - scrollY) < 120,
          'legacy full-node positions still restore');
        const repeated = document.querySelector('#repeated-run').getBoundingClientRect();
        scrollTo({top:scrollY + repeated.top + repeated.height/2,behavior:'instant'});
        dispatchEvent(new Event('scroll'));
        return scrollY;
        """#
        web.callAsyncJavaScript(script, arguments: ["first": first, "second": second], in: nil, in: .page) { result in
            guard case .success(let value) = result, let target = value as? Double else {
                fputs("Reading resume checks failed: \(result)\n", stderr); exit(1)
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.4) {
                guard let y = (self.latestGeometry?["viewport"] as? [String: Any])?["y"] as? Double,
                      abs(y - target) < 2, self.latestGeometry?["position"] is NSNull else {
                    fputs("Repeated text must not replace the last useful saved position\n", stderr); exit(1)
                }
                print("WKWebView reading resume: two distinct positions within one Unicode text node, exact context, reflow, legacy anchor restore and ambiguous position rejection passed.")
                exit(0)
            }
        }
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        let script = #"""
        return await (async()=>{
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
          document.body.style.setProperty('max-width','42em','important');
          booknookSetFontSize(19);
          scrollTo(0,0);
          const opening = {quote:'the quiet library welcomed Mara with open doors and many stories to discover in the late afternoon light.'};
          const ending = {quote:'Last chapter marker.'};
          const hidden = {quote:'Unique hidden quote.'};
          const hiddenMatch = booknookResolve(hidden);
          check(hiddenMatch.range?.toString()===hidden.quote && hiddenMatch.rects.length===0 &&
            booknookRestorePosition(hidden).status==='not_visible' && scrollY===0,
            'uniquely identified text without rectangles cannot report a successful jump');
          const highlights = [
            {id:'opening',anchor:opening},{id:'ending',anchor:ending},
            {id:'ambiguous',anchor:{quote:'Duplicate passage.'}},
            {id:'missing',anchor:{quote:'No annotation match.'}},
            {id:'hidden',anchor:hidden}
          ];
          const checkpoint = name => window.webkit.messageHandlers.booknook.postMessage({type:'bridge_test_checkpoint',name});
          const hasCustomHighlights = Boolean(window.CSS?.highlights && window.Highlight);
          booknookAnnotations(highlights);
          check(!hasCustomHighlights || window.CSS.highlights.get('booknook')?.size===2,
            hasCustomHighlights ? 'native custom highlight paints only two resolved ranges' : 'native highlight registry unavailable on this host');
          checkpoint('registry');
          booknookAnnotations([]);
          Object.defineProperty(window,'Highlight',{value:undefined,configurable:true});
          const originalHeight = document.documentElement.scrollHeight;
          booknookAnnotations(highlights);
          checkpoint('fallback');
          const overlay = () => document.querySelector('[data-booknook-highlight-fallback]');
          const aligned = anchor => {
            const layer = overlay();
            if (!layer) return false;
            const expected = Array.from(booknookResolve(anchor).range.getClientRects());
            const painted = Array.from(layer.children, rect=>rect.getBoundingClientRect());
            return expected.length>0 && painted.length===expected.length && expected.every((rect,index)=>
              ['left','top','width','height'].every(key=>Math.abs(rect[key]-painted[index][key]) < 0.75));
          };
          const waitFor = async condition => {
            for (let attempt=0; attempt<60; attempt++) {
              if (condition()) return true;
              await new Promise(resolve=>setTimeout(resolve,25));
            }
            return false;
          };
          check(aligned(opening),'fallback uses only the visible, uniquely resolved highlight rectangles');
          check(overlay().getAttribute('aria-hidden')==='true' && getComputedStyle(overlay()).pointerEvents==='none' &&
            document.documentElement.scrollHeight===originalHeight,'fallback leaves text accessibility, hit testing and document layout intact');
          const beforeReflow = booknookResolve(opening).rects;
          booknookSetFontSize(34);
          check(await waitFor(()=>aligned(opening)) &&
            JSON.stringify(beforeReflow)!==JSON.stringify(booknookResolve(opening).rects),'fallback follows the anchored text through font reflow');
          checkpoint('reflow');
          booknookAnnotations([...highlights].reverse());
          checkpoint('reordered');
          booknookRestorePosition(ending);
          check(await waitFor(()=>aligned(ending)) && scrollY>0,'fallback follows an offscreen highlight after restoring its passage');
          checkpoint('scrolled');
          booknookAnnotations([]);
          check(!overlay()?.children.length,'removing saved highlights clears the fallback');
          checkpoint('cleared');
          return checks;
        })()
        """#
        webView.callAsyncJavaScript(script, arguments: [:], in: nil, in: .page) { outcome in
            let result: Any
            switch outcome {
            case .success(let value): result = value
            case .failure(let error): fputs("Reader bridge failed: \(error)\n",stderr); exit(1)
            }
            guard let checks = result as? [String], checks.count == 23 else { fputs("Reader bridge returned an unexpected result: \(String(describing: result))\n",stderr); exit(1) }
            self.domChecks = checks
            self.finishIfReady()
        }
    }
}
@main struct ReaderBridgeTests {
    static func main() throws {
        let app = NSApplication.shared; app.setActivationPolicy(.prohibited)
        let check = ReaderBridgeCheck(); try check.run(); app.run()
    }
}
