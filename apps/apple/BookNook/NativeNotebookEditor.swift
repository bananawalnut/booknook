// SPDX-License-Identifier: MPL-2.0
import SwiftUI
#if os(macOS)
import AppKit
typealias NotebookFont = NSFont
#else
import UIKit
typealias NotebookFont = UIFont
#endif

struct FormatRequest { let id = UUID(); let style: String }

private struct NotebookPaneActiveKey: EnvironmentKey { static let defaultValue = true }
extension EnvironmentValues {
    var notebookPaneActive: Bool {
        get { self[NotebookPaneActiveKey.self] }
        set { self[NotebookPaneActiveKey.self] = newValue }
    }
}

func inlineStyle(_ attributes: [NSAttributedString.Key: Any], range: NSRange) -> InlineStyle {
    let font = attributes[.font] as? NotebookFont
    #if os(macOS)
    let traits = font.map { NSFontManager.shared.traits(of: $0) } ?? []
    let bold = traits.contains(.boldFontMask), italic = traits.contains(.italicFontMask)
    #else
    let traits = font?.fontDescriptor.symbolicTraits ?? []
    let bold = traits.contains(.traitBold), italic = traits.contains(.traitItalic)
    #endif
    return InlineStyle(startUtf16: range.location, lengthUtf16: range.length, bold: bold, italic: italic, underline: (attributes[.underlineStyle] as? Int ?? 0) != 0)
}
func portableStyles(_ value: NSAttributedString) -> [InlineStyle] {
    var result: [InlineStyle] = []
    value.enumerateAttributes(in: NSRange(location: 0, length: value.length)) { attributes, range, _ in
        let style = inlineStyle(attributes, range: range)
        guard style.bold || style.italic || style.underline else { return }
        if let last = result.last, last.startUtf16 + last.lengthUtf16 == style.startUtf16, last.bold == style.bold, last.italic == style.italic, last.underline == style.underline { result[result.count - 1].lengthUtf16 += style.lengthUtf16 }
        else { result.append(style) }
    }
    return result
}
func styledFont(_ style: InlineStyle, size: CGFloat) -> NotebookFont {
    #if os(macOS)
    var font = NSFont.systemFont(ofSize: size)
    if style.bold { font = NSFontManager.shared.convert(font, toHaveTrait: .boldFontMask) }
    if style.italic { font = NSFontManager.shared.convert(font, toHaveTrait: .italicFontMask) }
    return font
    #else
    let font = UIFont.systemFont(ofSize: size)
    var traits: UIFontDescriptor.SymbolicTraits = []
    if style.bold { traits.insert(.traitBold) }; if style.italic { traits.insert(.traitItalic) }
    return font.fontDescriptor.withSymbolicTraits(traits).map { UIFont(descriptor: $0, size: size) } ?? font
    #endif
}
func notebookBodyFont() -> NotebookFont {
    #if os(macOS)
    NSFont.systemFont(ofSize: 18)
    #else
    UIFont.preferredFont(forTextStyle: .body)
    #endif
}
func attributedNotebook(_ text: String, styles: [InlineStyle]) -> NSAttributedString {
    let font = notebookBodyFont()
    #if os(macOS)
    let color = NSColor(red: 41.0 / 255, green: 63.0 / 255, blue: 56.0 / 255, alpha: 1)
    #else
    let color = UIColor(red: 41.0 / 255, green: 63.0 / 255, blue: 56.0 / 255, alpha: 1)
    #endif
    let value = NSMutableAttributedString(string: text, attributes: [.font: font, .foregroundColor: color])
    for style in styles where style.startUtf16 >= 0 && style.startUtf16 + style.lengthUtf16 <= value.length {
        value.addAttributes([.font: styledFont(style, size: font.pointSize), .underlineStyle: style.underline ? NSUnderlineStyle.single.rawValue : 0], range: NSRange(location: style.startUtf16, length: style.lengthUtf16))
    }
    return value
}

#if os(macOS)
func installNotebookContent(_ text: String, styles: [InlineStyle], in view: NSTextView) {
    view.textStorage?.setAttributedString(attributedNotebook(text, styles: styles))
    // An empty attributed string has no runs from which TextKit can inherit
    // typing attributes. Seed only empty snapshots; preserve nonempty rich text.
    if text.isEmpty {
        let font = notebookBodyFont()
        view.font = font; view.textColor = NSColor(red: 41.0 / 255, green: 63.0 / 255, blue: 56.0 / 255, alpha: 1)
        view.typingAttributes = [.font: font, .foregroundColor: NSColor(red: 41.0 / 255, green: 63.0 / 255, blue: 56.0 / 255, alpha: 1)]
    }
}
final class NotebookTextView: NSTextView {
    var changed: (() -> Void)?
    var notebookPaneActive = true
    override var acceptsFirstResponder: Bool { notebookPaneActive && super.acceptsFirstResponder }
    override func becomeFirstResponder() -> Bool { notebookPaneActive && super.becomeFirstResponder() }
    override func keyDown(with event: NSEvent) {
        guard notebookPaneActive else { return }
        if event.modifierFlags.contains(.command), let key = event.charactersIgnoringModifiers, ["b", "i", "u"].contains(key) {
            toggleStyle(["b": "bold", "i": "italic", "u": "underline"][key]!); return
        }
        super.keyDown(with: event)
    }
    func restore(_ value: NSAttributedString) {
        let before = NSAttributedString(attributedString: attributedString())
        undoManager?.registerUndo(withTarget: self) { $0.restore(before) }
        textStorage?.setAttributedString(value); changed?()
    }
    func toggleStyle(_ name: String) {
        guard notebookPaneActive, !hasMarkedText() else { return }
        let range = selectedRange()
        if range.length == 0 {
            var style = inlineStyle(typingAttributes, range: range)
            switch name { case "bold": style.bold.toggle(); case "italic": style.italic.toggle(); default: style.underline.toggle() }
            typingAttributes[.font] = styledFont(style, size: font?.pointSize ?? 18)
            typingAttributes[.underlineStyle] = style.underline ? NSUnderlineStyle.single.rawValue : 0
            window?.makeFirstResponder(self); return
        }
        let before = NSAttributedString(attributedString: attributedString())
        let value = NSMutableAttributedString(attributedString: before)
        var allEnabled = true
        before.enumerateAttributes(in: range) { attrs, r, _ in let s = inlineStyle(attrs, range: r); allEnabled = allEnabled && (name == "bold" ? s.bold : name == "italic" ? s.italic : s.underline) }
        before.enumerateAttributes(in: range) { attrs, r, _ in
            var style = inlineStyle(attrs, range: r)
            switch name { case "bold": style.bold = !allEnabled; case "italic": style.italic = !allEnabled; default: style.underline = !allEnabled }
            value.addAttributes([.font: styledFont(style, size: (attrs[.font] as? NSFont)?.pointSize ?? 18), .underlineStyle: style.underline ? NSUnderlineStyle.single.rawValue : 0], range: r)
        }
        undoManager?.registerUndo(withTarget: self) { $0.restore(before) }
        textStorage?.setAttributedString(value); setSelectedRange(range); window?.makeFirstResponder(self); changed?()
    }
}
struct NativeNotebookEditor: NSViewRepresentable {
    @Environment(\.notebookPaneActive) private var notebookPaneActive
    @Binding var text: String
    @Binding var styles: [InlineStyle]
    let formatRequest: FormatRequest?
    var changed: (String, [InlineStyle]) -> Void
    func makeCoordinator() -> Coordinator { Coordinator(self) }
    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSScrollView(); scroll.hasVerticalScroller = true; scroll.drawsBackground = false
        let view = NotebookTextView(usingTextLayoutManager: true)
        view.isRichText = true; view.importsGraphics = false; view.allowsUndo = true
        view.isVerticallyResizable = true; view.isHorizontallyResizable = false; view.autoresizingMask = [.width]
        view.textContainer?.widthTracksTextView = true; view.textContainerInset = NSSize(width: 4, height: 10)
        view.drawsBackground = false; view.setAccessibilityLabel("Notebook text"); view.delegate = context.coordinator
        installNotebookContent(text, styles: styles, in: view); context.coordinator.lastText = text; context.coordinator.lastStyles = styles
        view.changed = { [weak view, weak coordinator = context.coordinator] in if let view { coordinator?.capture(view) } }
        scroll.documentView = view
        context.coordinator.setPaneActive(notebookPaneActive, editor: view)
        return scroll
    }
    func updateNSView(_ view: NSScrollView, context: Context) {
        context.coordinator.parent = self
        guard let editor = view.documentView as? NotebookTextView else { return }
        context.coordinator.setPaneActive(notebookPaneActive, editor: editor)
        context.coordinator.applySnapshot(to: editor, text: text, styles: styles)
        if let request = formatRequest { context.coordinator.requestFormat(request, editor: editor) }
    }
    static func dismantleNSView(_ view: NSScrollView, coordinator: Coordinator) { if let editor = view.documentView as? NSTextView { coordinator.capture(editor) } }
    class Coordinator: NSObject, NSTextViewDelegate {
        var parent: NativeNotebookEditor
        var lastFormat: UUID?
        var lastText = ""
        var lastStyles: [InlineStyle] = []
        var hasLocalEdits = false
        private(set) var paneActive = true
        private var visibilityGeneration = 0
        private var awaitingVisibilityCapture = false
        init(_ parent: NativeNotebookEditor) { self.parent = parent }
        func setPaneActive(_ active: Bool, editor: NotebookTextView) {
            editor.notebookPaneActive = active
            guard paneActive != active else { return }
            paneActive = active; visibilityGeneration += 1
            guard !active else { return }
            // Commit the native composition before resigning. The same SwiftUI
            // update may still carry the snapshot from before this last input.
            if editor.hasMarkedText() { editor.unmarkText() }
            capture(editor)
            if editor.window?.firstResponder === editor { editor.window?.makeFirstResponder(nil) }
            capture(editor)
            awaitingVisibilityCapture = hasLocalEdits
        }
        func requestFormat(_ request: FormatRequest, editor: NotebookTextView) {
            guard lastFormat != request.id else { return }
            lastFormat = request.id
            guard paneActive else { return }
            let generation = visibilityGeneration
            DispatchQueue.main.async { [weak self, weak editor] in
                guard let self, let editor, self.paneActive, self.visibilityGeneration == generation else { return }
                editor.toggleStyle(request.style)
            }
        }
        func applySnapshot(to editor: NSTextView, text: String, styles: [InlineStyle]) {
            // SwiftUI and autosave can deliver an older binding snapshot after the
            // next native key event. The active TextKit selection and text own truth.
            // Explicit history/sync restoration drains and remounts the editor.
            guard paneActive, !editor.hasMarkedText() else { return }
            if awaitingVisibilityCapture {
                guard lastText == text, lastStyles == styles else { return }
                awaitingVisibilityCapture = false
            }
            guard !(hasLocalEdits && editor.window?.firstResponder === editor) else { return }
            guard lastText != text || lastStyles != styles else { return }
            let range = editor.selectedRange(), length = (text as NSString).length
            installNotebookContent(text, styles: styles, in: editor)
            editor.setSelectedRange(NSRange(location: min(range.location, length), length: min(range.length, max(0, length-range.location))))
            lastText = text; lastStyles = styles; hasLocalEdits = false
        }
        func capture(_ view: NSTextView) { guard !view.hasMarkedText() else { return }; let styles = portableStyles(view.attributedString()); guard view.string != lastText || styles != lastStyles else { return }; hasLocalEdits = true; lastText = view.string; lastStyles = styles; parent.text = view.string; parent.styles = styles; parent.changed(view.string, styles) }
        func textDidChange(_ notification: Notification) { if let view = notification.object as? NSTextView { capture(view) } }
        func textDidEndEditing(_ notification: Notification) { if let view = notification.object as? NSTextView { capture(view) } }
    }
}
#else
final class NotebookTextView: UITextView {
    var notebookPaneActive = true
    override var canBecomeFirstResponder: Bool { notebookPaneActive && super.canBecomeFirstResponder }
}
func installNotebookContent(_ text: String, styles: [InlineStyle], in view: UITextView) {
    view.attributedText = attributedNotebook(text, styles: styles)
    // Seed the empty editor's font as well as typing attributes: UIKit can reset
    // typing attributes when the selection changes before the first insertion.
    if text.isEmpty {
        let font = notebookBodyFont()
        view.font = font; view.textColor = UIColor(red: 41.0 / 255, green: 63.0 / 255, blue: 56.0 / 255, alpha: 1)
        view.typingAttributes = [.font: font, .foregroundColor: UIColor(red: 41.0 / 255, green: 63.0 / 255, blue: 56.0 / 255, alpha: 1)]
    }
}
func revealNotebookCaret(_ view: UITextView) {
    guard view.isFirstResponder, view.window != nil, let selection = view.selectedTextRange else { return }
    // The text view can reveal its own caret while still being outside its
    // enclosing SwiftUI scroll viewport. Forward the measured caret outward;
    // SwiftUI remains responsible for all keyboard/safe-area content insets.
    var ancestor: UIView? = view
    while let current = ancestor {
        if let scroll = current as? UIScrollView, scroll.isScrollEnabled {
            let padding = view.textContainerInset
            let editingRect = view.caretRect(for: selection.end).inset(by: UIEdgeInsets(
                top: -max(0, padding.top), left: -max(0, padding.left),
                bottom: -max(0, padding.bottom), right: -max(0, padding.right)))
            let caret = view.convert(editingRect, to: scroll)
            let inset = scroll.adjustedContentInset
            let visible = scroll.bounds.inset(by: inset)
            if !caret.isNull, caret.width > 0, caret.height > 0, visible.width > 0, visible.height > 0 {
                let dx = caret.minX < visible.minX ? caret.minX - visible.minX : max(0, caret.maxX - visible.maxX)
                let dy = caret.minY < visible.minY ? caret.minY - visible.minY : max(0, caret.maxY - visible.maxY)
                let minimum = CGPoint(x: -inset.left, y: -inset.top)
                let maximum = CGPoint(x: max(minimum.x, scroll.contentSize.width - scroll.bounds.width + inset.right),
                                      y: max(minimum.y, scroll.contentSize.height - scroll.bounds.height + inset.bottom))
                let offset = CGPoint(x: min(maximum.x, max(minimum.x, scroll.contentOffset.x + dx)),
                                     y: min(maximum.y, max(minimum.y, scroll.contentOffset.y + dy)))
                if offset.x.isFinite, offset.y.isFinite, offset != scroll.contentOffset {
                    scroll.setContentOffset(offset, animated: false)
                }
            }
        }
        ancestor = current.superview
    }
}
struct NativeNotebookEditor: UIViewRepresentable {
    @Environment(\.notebookPaneActive) private var notebookPaneActive
    @Binding var text: String
    @Binding var styles: [InlineStyle]
    let formatRequest: FormatRequest?
    var changed: (String, [InlineStyle]) -> Void
    func makeCoordinator() -> Coordinator { Coordinator(self) }
    func makeUIView(context: Context) -> UITextView { makeEditor(coordinator: context.coordinator) }
    func makeEditor(coordinator: Coordinator, active: Bool? = nil) -> UITextView {
        let view = NotebookTextView(usingTextLayoutManager: true); view.delegate = coordinator
        view.allowsEditingTextAttributes = true
        installNotebookContent(text, styles: styles, in: view); coordinator.lastText = text; coordinator.lastStyles = styles
        view.adjustsFontForContentSizeCategory = true; view.backgroundColor = .clear; view.accessibilityLabel = "Notebook text"
        coordinator.setPaneActive(active ?? notebookPaneActive, view: view)
        return view
    }
    func updateUIView(_ view: UITextView, context: Context) { updateEditor(view, coordinator: context.coordinator) }
    func updateEditor(_ view: UITextView, coordinator: Coordinator, active: Bool? = nil) {
        coordinator.parent = self
        coordinator.setPaneActive(active ?? notebookPaneActive, view: view)
        coordinator.applySnapshot(to: view, text: text, styles: styles)
        if let request = formatRequest { coordinator.requestFormat(request, view: view) }
    }
    static func dismantleUIView(_ view: UITextView, coordinator: Coordinator) { coordinator.textViewDidChange(view) }
    class Coordinator: NSObject, UITextViewDelegate {
        var parent: NativeNotebookEditor
        var lastFormat: UUID?
        var lastText = ""
        var lastStyles: [InlineStyle] = []
        var hasLocalEdits = false
        private(set) var paneActive = true
        private var visibilityGeneration = 0
        private var awaitingVisibilityCapture = false
        weak var focusedView: UITextView?
        var caretRevealScheduled = false
        init(_ parent: NativeNotebookEditor) {
            self.parent = parent
            super.init()
            NotificationCenter.default.addObserver(self, selector: #selector(keyboardFrameDidChange), name: UIResponder.keyboardDidChangeFrameNotification, object: nil)
        }
        deinit { NotificationCenter.default.removeObserver(self) }
        func setPaneActive(_ active: Bool, view: UITextView) {
            (view as? NotebookTextView)?.notebookPaneActive = active
            guard paneActive != active else { return }
            paneActive = active; visibilityGeneration += 1
            guard !active else { return }
            if view.markedTextRange != nil { view.unmarkText() }
            textViewDidChange(view)
            if view.isFirstResponder { view.resignFirstResponder() }
            textViewDidChange(view)
            awaitingVisibilityCapture = hasLocalEdits
        }
        func requestFormat(_ request: FormatRequest, view: UITextView) {
            guard lastFormat != request.id else { return }
            lastFormat = request.id
            guard paneActive else { return }
            let generation = visibilityGeneration
            DispatchQueue.main.async { [weak self, weak view] in
                guard let self, let view, self.paneActive, self.visibilityGeneration == generation, view.markedTextRange == nil else { return }
                view.becomeFirstResponder()
                switch request.style { case "bold": view.toggleBoldface(nil); case "italic": view.toggleItalics(nil); default: view.toggleUnderline(nil) }
                self.textViewDidChange(view)
            }
        }
        @objc private func keyboardFrameDidChange(_ notification: Notification) {
            if let view = focusedView { scheduleCaretReveal(view) }
        }
        private func scheduleCaretReveal(_ view: UITextView) {
            guard paneActive, view.isFirstResponder else { return }
            focusedView = view
            guard !caretRevealScheduled else { return }
            caretRevealScheduled = true
            DispatchQueue.main.async { [weak self, weak view] in
                self?.caretRevealScheduled = false
                guard self?.paneActive == true, let view, view.isFirstResponder else { return }
                view.window?.layoutIfNeeded()
                revealNotebookCaret(view)
            }
        }
        func applySnapshot(to view: UITextView, text: String, styles: [InlineStyle]) {
            guard paneActive, view.markedTextRange == nil else { return }
            if awaitingVisibilityCapture {
                guard lastText == text, lastStyles == styles else { return }
                awaitingVisibilityCapture = false
            }
            guard !(hasLocalEdits && view.isFirstResponder) else { return }
            guard lastText != text || lastStyles != styles else { return }
            let range = view.selectedRange, length = (text as NSString).length
            installNotebookContent(text, styles: styles, in: view)
            view.selectedRange = NSRange(location: min(range.location, length), length: min(range.length, max(0, length-range.location)))
            lastText = text; lastStyles = styles; hasLocalEdits = false
        }
        func textViewShouldBeginEditing(_ view: UITextView) -> Bool { paneActive }
        func textViewDidBeginEditing(_ view: UITextView) { scheduleCaretReveal(view) }
        func textViewDidChangeSelection(_ view: UITextView) { scheduleCaretReveal(view) }
        func textViewDidEndEditing(_ view: UITextView) { if focusedView === view { focusedView = nil }; textViewDidChange(view) }
        func textViewDidChange(_ view: UITextView) { guard view.markedTextRange == nil else { return }; let styles = portableStyles(view.attributedText); guard view.text != lastText || styles != lastStyles else { return }; hasLocalEdits = true; lastText = view.text; lastStyles = styles; parent.text = view.text; parent.styles = styles; parent.changed(view.text, styles) }
    }
}
#endif
