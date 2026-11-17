// SPDX-License-Identifier: MPL-2.0
import AppKit
import PDFKit
@main struct PDFReaderTests {
    static func main() throws {
        NSApplication.shared.setActivationPolicy(.prohibited)
        let bytes = NSMutableData(), pageBox = CGRect(x: 0,y: 0,width: 600,height: 800)
        let consumer = CGDataConsumer(data: bytes)!
        var mediaBox = pageBox
        let context = CGContext(consumer: consumer,mediaBox: &mediaBox,nil)!
        context.beginPDFPage(nil)
        NSGraphicsContext.saveGraphicsState(); NSGraphicsContext.current = NSGraphicsContext(cgContext: context,flipped: false)
        NSAttributedString(string: "A quiet library welcomes its reader.",attributes: [.font: NSFont.systemFont(ofSize: 18)]).draw(at: CGPoint(x: 40,y: 700))
        NSGraphicsContext.restoreGraphicsState(); context.endPDFPage(); context.closePDF()
        let url = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("BookNook-\(UUID()).pdf")
        try bytes.write(to: url); defer { try? FileManager.default.removeItem(at: url) }
        let view = PDFView(frame: CGRect(x: 0,y: 0,width: 600,height: 800))
        var selected: [[String: Any]] = []
        var placementReports: [[String: String]] = []
        let reader = PDFReaderCoordinator(onMessage: { message in
            switch message["type"] as? String {
            case "pdf_selection": selected = message["highlights"] as? [[String: Any]] ?? []
            case "pdf_placements":
                precondition(message["assetFingerprint"] as? String == "fixture-sha256")
                placementReports.append(message["placements"] as! [String: String])
            default: break
            }
        }, onInk: { _, acknowledged in acknowledged(true) })
        func drainMessages() {
            var received = false
            DispatchQueue.main.async { received = true }
            let deadline = Date(timeIntervalSinceNow: 2)
            while !received && Date() < deadline {
                RunLoop.main.run(until: Date(timeIntervalSinceNow: 0.01))
            }
            precondition(received, "PDF placement message queue did not drain")
        }
        reader.fingerprint = "fixture-sha256"; reader.connect(view); reader.load(url)
        let document = view.document!, page = document.page(at: 0)!
        let selection = document.findString("quiet library",withOptions: []).first!
        view.currentSelection = selection; reader.selection(); RunLoop.main.run(until: Date(timeIntervalSinceNow: 0.02))
        precondition(selected.count == 1 && selected[0]["page"] as? Int == 1)
        let geometry = selected[0]["geometry"] as! [String: Any]
        precondition(!(geometry["text_rects"] as! [[String: Any]]).isEmpty)
        let mark = Annotation(id: "highlight",inkGeometry: JSONValue(object: geometry),nativeDrawing: nil,originalContext: "quiet library",strokes: [])
        reader.annotations = [mark]; reader.render(); precondition(page.annotations.count == 1)
        precondition(page.annotations[0].contents == "quiet library")
        drainMessages()
        precondition(placementReports == [["highlight": "resolved"]], "A drawn PDF highlight must report resolved placement once")
        reader.render(); precondition(page.annotations.count == 1)
        drainMessages()
        precondition(placementReports.count == 1, "Unchanged PDF placements must not emit again")
        reader.annotations = []; reader.render(); precondition(page.annotations.isEmpty)
        drainMessages()
        precondition(placementReports.last == [:], "Removed PDF annotations must clear their placement statuses")
        var inkGeometry = geometry; inkGeometry["text_rects"] = []
        let stroke = JSONValue(object: ["points": [["x": 40.0,"y": 690.0],["x": 100.0,"y": 690.0]],"width": 3.0,"color": "#c02080"])
        reader.annotations = [Annotation(id: "ink",inkGeometry: JSONValue(object: inkGeometry),nativeDrawing: nil,originalContext: "PDF ink",strokes: [stroke])]
        reader.render(); precondition(page.annotations.count == 1 && page.annotations[0].type == "Ink")
        drainMessages()
        precondition(placementReports.last == ["ink": "resolved"], "Matching PDF ink must also report resolved placement")
        let inkColor = page.annotations[0].color.usingColorSpace(.deviceRGB)!
        precondition(abs(inkColor.redComponent - 192.0 / 255) < 0.001 && abs(inkColor.blueComponent - 128.0 / 255) < 0.001)
        reader.annotations = []; reader.render(); precondition(page.annotations.isEmpty)
        var mismatched = geometry
        mismatched["content_fingerprint"] = "another-file"
        let wrongFile = Annotation(id: "wrong-file", inkGeometry: JSONValue(object: mismatched),
                                   nativeDrawing: nil, originalContext: "Original highlight quote", strokes: [])
        var missingPage = inkGeometry
        missingPage["page"] = document.pageCount + 1
        let wrongPage = Annotation(id: "wrong-page", inkGeometry: JSONValue(object: missingPage),
                                   nativeDrawing: nil, originalContext: "Original ink context", strokes: [stroke])
        reader.annotations = [wrongFile, wrongPage]
        reader.render(); precondition(page.annotations.isEmpty)
        drainMessages()
        let unresolved = ["wrong-file": "wrong_asset", "wrong-page": "missing_page"]
        precondition(placementReports.last == unresolved, "Each unplaced PDF annotation must report its own reason")
        precondition(reader.annotations.map(\.originalContext) == ["Original highlight quote", "Original ink context"])
        let unresolvedReportCount = placementReports.count
        reader.annotations.reverse()
        reader.render()
        drainMessages()
        precondition(placementReports.count == unresolvedReportCount, "Row order must not duplicate the placement snapshot")
        reader.annotations = [mark, wrongPage]
        reader.render()
        drainMessages()
        precondition(placementReports.last == ["highlight": "resolved", "wrong-page": "missing_page"])
        precondition(page.annotations.count == 1 && page.annotations[0].contents == "quiet library")
        let unchanged = try Data(contentsOf: url); precondition(unchanged == bytes as Data)
        print("PDFKit: accessible selection, exact highlight/ink rendering, per-annotation wrong-file/missing-page status, no misplaced overlays, deduplication, and unchanged PDF bytes passed.")
    }
}
