// SPDX-License-Identifier: MPL-2.0
// Exercises the actual TextKit editor class in an isolated, hidden test window.
import AppKit
import SwiftUI
struct InlineStyle: Equatable { var startUtf16: Int; var lengthUtf16: Int; var bold: Bool; var italic: Bool; var underline: Bool }
@main struct NotebookEditorTests {
    static func main() {
        let app = NSApplication.shared; app.setActivationPolicy(.prohibited)
        let window = NSWindow(contentRect: CGRect(x: 0,y: 0,width: 600,height: 500),styleMask: [.titled],backing: .buffered,defer: false)
        let editor = NotebookTextView(usingTextLayoutManager: true)
        editor.isRichText = true; editor.allowsUndo = true; window.contentView = editor; window.makeFirstResponder(editor)
        // Exercise the actual initial-content path: an empty attributed string
        // alone used to leave first typing in Helvetica 12pt, then reopen at 18pt.
        installNotebookContent("", styles: [], in: editor)
        editor.setSelectedRange(NSRange(location: 0, length: 0))
        editor.insertText("Red", replacementRange: NSRange(location: NSNotFound, length: 0))
        let firstFont = editor.attributedString().attribute(.font, at: 0, effectiveRange: nil) as! NSFont
        let reopenedFont = attributedNotebook("Red", styles: []).attribute(.font, at: 0, effectiveRange: nil) as! NSFont
        precondition(firstFont.fontName == reopenedFont.fontName && firstFont.pointSize == reopenedFont.pointSize,
                     "First typing and reopened writing must use the same body font")
        precondition((editor.attributedString().attribute(.foregroundColor, at: 0, effectiveRange: nil) as? NSColor) == NSColor(red: 41.0 / 255, green: 63.0 / 255, blue: 56.0 / 255, alpha: 1))
        let mixed = [InlineStyle(startUtf16: 0, lengthUtf16: 3, bold: true, italic: false, underline: true)]
        installNotebookContent("Red blue", styles: mixed, in: editor)
        precondition(portableStyles(editor.attributedString()) == mixed, "Installing nonempty content must retain mixed rich styles")
        let emptyParent = NativeNotebookEditor(text: .constant(""), styles: .constant([]), formatRequest: nil, changed: { _, _ in })
        let emptyCoordinator = NativeNotebookEditor.Coordinator(emptyParent)
        emptyCoordinator.lastText = "Red blue"; emptyCoordinator.lastStyles = mixed
        emptyCoordinator.applySnapshot(to: editor, text: "", styles: [])
        editor.insertText("Blue", replacementRange: NSRange(location: NSNotFound, length: 0))
        let externalEmptyFont = editor.attributedString().attribute(.font, at: 0, effectiveRange: nil) as! NSFont
        precondition(externalEmptyFont.fontName == reopenedFont.fontName && externalEmptyFont.pointSize == reopenedFont.pointSize)
        precondition(portableStyles(editor.attributedString()).isEmpty, "A newly installed empty snapshot starts in the plain body style")
        editor.undoManager?.removeAllActions()
        let original = "😀 café — 日本語\n" + String(repeating: "A character's meaning changes with each reading.\n",count: 2400)
        editor.textStorage?.setAttributedString(attributedNotebook(original,styles: []))
        editor.setSelectedRange(NSRange(location: 0,length: 2))
        editor.toggleStyle("bold")
        precondition(editor.string == original)
        precondition(portableStyles(editor.attributedString()).first?.bold == true)
        precondition(editor.selectedRange() == NSRange(location: 0,length: 2))
        editor.undoManager?.undo()
        precondition(portableStyles(editor.attributedString()).isEmpty)
        editor.undoManager?.redo()
        precondition(portableStyles(editor.attributedString()).first?.bold == true)
        let at = (editor.string as NSString).length
        editor.setSelectedRange(NSRange(location: at,length: 0))
        editor.setMarkedText("に",selectedRange: NSRange(location: 1,length: 0),replacementRange: NSRange(location: NSNotFound,length: 0))
        precondition(editor.hasMarkedText())
        let before = portableStyles(editor.attributedString())
        editor.toggleStyle("underline")
        precondition(portableStyles(editor.attributedString()) == before)
        editor.unmarkText()
        precondition(!editor.hasMarkedText() && editor.string.hasSuffix("に"))
        let value = attributedNotebook("A😀B",styles: [InlineStyle(startUtf16: 1,lengthUtf16: 2,bold: true,italic: true,underline: true)])
        let styles = portableStyles(value)
        precondition(styles == [InlineStyle(startUtf16: 1,lengthUtf16: 2,bold: true,italic: true,underline: true)])
        // Regression: stale SwiftUI snapshots interleaved with native typing used
        // to replace newer content and move subsequent input backwards.
        var boundText = "Seed", boundStyles: [InlineStyle] = []
        let parent = NativeNotebookEditor(text: Binding(get: { boundText }, set: { boundText = $0 }), styles: Binding(get: { boundStyles }, set: { boundStyles = $0 }), formatRequest: nil, changed: { _, _ in })
        let coordinator = NativeNotebookEditor.Coordinator(parent)
        editor.textStorage?.setAttributedString(attributedNotebook(boundText, styles: [])); coordinator.lastText = boundText; coordinator.lastStyles = []; editor.delegate = coordinator
        window.makeFirstResponder(editor); editor.setSelectedRange(NSRange(location: 4,length: 0))
        var expected = "Seed"
        for character in " abcdefghijklmnopqrstuvwxyz0123456789 café 🦊 日本語" {
            let stale = expected
            editor.insertText(String(character), replacementRange: NSRange(location: NSNotFound,length: 0)); coordinator.capture(editor)
            expected += String(character)
            coordinator.applySnapshot(to: editor, text: stale, styles: [])
            precondition(editor.string == expected, "A stale autosave echo rewrote active typing")
            precondition(editor.selectedRange().location == (expected as NSString).length, "A stale echo moved the caret")
        }
        precondition(boundText == expected)
        window.makeFirstResponder(nil)
        coordinator.applySnapshot(to: editor, text: "Reviewed external revision", styles: [])
        precondition(editor.string == "Reviewed external revision")

        // Automatic compact layout changes retain the editor view. Hiding that
        // view must commit its native input before any stale binding is applied.
        let staleBeforeHide = editor.string
        window.makeFirstResponder(editor)
        editor.setSelectedRange(NSRange(location: (editor.string as NSString).length, length: 0))
        editor.setMarkedText("日本語", selectedRange: NSRange(location: 3, length: 0), replacementRange: NSRange(location: NSNotFound, length: 0))
        precondition(editor.hasMarkedText() && window.firstResponder === editor)
        let committedOnHide = editor.string
        coordinator.setPaneActive(false, editor: editor)
        precondition(!editor.hasMarkedText() && window.firstResponder !== editor, "An invisible notebook must release its own focus")
        precondition(boundText == committedOnHide && editor.string == committedOnHide, "Hiding must capture native marked text without discarding it")
        coordinator.applySnapshot(to: editor, text: staleBeforeHide, styles: [])
        precondition(editor.string == committedOnHide, "Resignation must not allow a stale SwiftUI snapshot to overwrite the captured text")
        precondition(!editor.becomeFirstResponder() && !editor.acceptsFirstResponder, "A hidden notebook cannot regain native focus")
        editor.setSelectedRange(NSRange(location: 0, length: 3))
        let hiddenStyles = portableStyles(editor.attributedString())
        editor.toggleStyle("underline")
        coordinator.requestFormat(FormatRequest(style: "bold"), editor: editor)
        precondition(portableStyles(editor.attributedString()) == hiddenStyles)
        coordinator.setPaneActive(true, editor: editor)
        coordinator.applySnapshot(to: editor, text: staleBeforeHide, styles: [])
        precondition(editor.string == committedOnHide, "Revealing must wait for the captured binding rather than restore an old snapshot")
        coordinator.applySnapshot(to: editor, text: committedOnHide, styles: hiddenStyles)
        coordinator.applySnapshot(to: editor, text: "Acknowledged external revision", styles: [])
        precondition(editor.string == "Acknowledged external revision", "External updates remain available after the captured binding is acknowledged")

        // A toolbar callback queued before hiding must not steal focus after
        // either a hide or a rapid hide/reveal cycle.
        editor.setSelectedRange(NSRange(location: 0, length: 3))
        coordinator.requestFormat(FormatRequest(style: "italic"), editor: editor)
        coordinator.setPaneActive(false, editor: editor)
        coordinator.setPaneActive(true, editor: editor)
        RunLoop.main.run(until: Date(timeIntervalSinceNow: 0.05))
        precondition(window.firstResponder !== editor && portableStyles(editor.attributedString()).isEmpty, "Stale formatting must not reactivate a notebook")

        // Ordinary form fields (including a sheet's field editor) are not owned
        // by this notebook, so a layout update must leave their focus untouched.
        let field = NSTextField(frame: NSRect(x: 10, y: 10, width: 200, height: 30))
        editor.addSubview(field)
        precondition(window.makeFirstResponder(field))
        guard let fieldEditor = field.currentEditor() else { preconditionFailure("The actual form field did not obtain its TextKit editor") }
        precondition(window.firstResponder === fieldEditor)
        coordinator.setPaneActive(false, editor: editor)
        precondition(window.firstResponder === fieldEditor, "Hiding the notebook must not resign an unrelated form field")
        coordinator.requestFormat(FormatRequest(style: "underline"), editor: editor)
        RunLoop.main.run(until: Date(timeIntervalSinceNow: 0.05))
        precondition(window.firstResponder === fieldEditor && portableStyles(editor.attributedString()).isEmpty)
        coordinator.setPaneActive(true, editor: editor)
        precondition(window.firstResponder === fieldEditor, "Revealing the notebook must not automatically steal focus")
        precondition(window.makeFirstResponder(editor), "The visible notebook can be focused explicitly again")
        print("TextKit editor: initial/external empty-snapshot first typing matches reopened body font/color, mixed rich styles preserved; 115k-character Unicode document, bold formatting, selection preservation, undo/redo, IME marked-text protection and portable style round-trip, rapid ASCII/Unicode typing with stale autosave echoes, caret stability and external refresh after focus loss; hidden-pane native focus/IME capture, stale-snapshot protection, queued-format cancellation, and unrelated form-field focus passed.")
    }
}
