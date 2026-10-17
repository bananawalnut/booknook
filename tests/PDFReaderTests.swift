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
        let reader = PDFReaderCoordinator(onMessage: { if $0["type"] as? String == "pdf_selection" { selected = $0["highlights"] as? [[String: Any]] ?? [] } },onInk: { _, acknowledged in acknowledged(true) })
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
        reader.render(); precondition(page.annotations.count == 1)
        reader.annotations = []; reader.render(); precondition(page.annotations.isEmpty)
        var inkGeometry = geometry; inkGeometry["text_rects"] = []
        let stroke = JSONValue(object: ["points": [["x": 40.0,"y": 690.0],["x": 100.0,"y": 690.0]],"width": 3.0,"color": "#c02080"])
        reader.annotations = [Annotation(id: "ink",inkGeometry: JSONValue(object: inkGeometry),nativeDrawing: nil,originalContext: "PDF ink",strokes: [stroke])]
        reader.render(); precondition(page.annotations.count == 1 && page.annotations[0].type == "Ink")
        let inkColor = page.annotations[0].color.usingColorSpace(.deviceRGB)!
        precondition(abs(inkColor.redComponent - 192.0 / 255) < 0.001 && abs(inkColor.blueComponent - 128.0 / 255) < 0.001)
        reader.annotations = []; reader.render(); precondition(page.annotations.isEmpty)
        var mismatched = geometry; mismatched["content_fingerprint"] = "another-file"
        reader.annotations = [Annotation(id: "wrong-file",inkGeometry: JSONValue(object: mismatched),nativeDrawing: nil,originalContext: "Do not place",strokes: [])]
        reader.render(); precondition(page.annotations.isEmpty)
        let unchanged = try Data(contentsOf: url); precondition(unchanged == bytes as Data)
        print("PDFKit: accessible text selection, portable page rectangles, highlight rendering/deduplication, pass isolation, portable-only ink and color without PKDrawing, asset isolation and unchanged source bytes passed.")
    }
}
