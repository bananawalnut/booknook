// SPDX-License-Identifier: MPL-2.0
import SwiftUI
#if os(iOS)
import UIKit
import PencilKit

/// One mounted native paper surface routes the original touch. No gesture needs
/// to finish and no SwiftUI input mode needs to change before PencilKit receives it.
struct NativeNotebookPage: UIViewRepresentable {
    @Environment(\.notebookPaneActive) private var paneActive
    @Binding var text: String
    @Binding var styles: [InlineStyle]
    let formatRequest: FormatRequest?
    @ObservedObject var session: NotebookInkSession
    @ObservedObject var tools: NotebookInkTools
    var minimumHeight: CGFloat = 0
    let changed: (String, [InlineStyle]) -> Void
    var continueHandwriting: (() -> Void)? = nil
    private var textEditor: NativeNotebookEditor {
        NativeNotebookEditor(text: $text, styles: $styles, formatRequest: formatRequest, changed: changed)
    }
    func makeCoordinator() -> NativeNotebookEditor.Coordinator { NativeNotebookEditor.Coordinator(textEditor) }
    func makeUIView(context: Context) -> NotebookPageHost {
        let editor = textEditor.makeEditor(coordinator: context.coordinator, active: paneActive)
        editor.isScrollEnabled = false
        editor.accessibilityIdentifier = "notebook-text-\(session.blockID)"
        let host = NotebookPageHost(editor: editor, ink: NotebookInkHost(session: session, tools: tools))
        context.coordinator.setPaneActive(paneActive, view: editor)
        host.ink.update(editable: true, active: paneActive)
        host.continueHandwriting = continueHandwriting
        host.refreshPlaceholder()
        return host
    }
    func updateUIView(_ view: NotebookPageHost, context: Context) {
        textEditor.updateEditor(view.editor, coordinator: context.coordinator, active: paneActive)
        context.coordinator.setPaneActive(paneActive, view: view.editor)
        view.ink.update(editable: true, active: paneActive)
        view.continueHandwriting = continueHandwriting
        view.refreshPlaceholder()
        view.invalidateIntrinsicContentSize(); view.setNeedsLayout()
    }
    func sizeThatFits(_ proposal: ProposedViewSize, uiView: NotebookPageHost, context: Context) -> CGSize? {
        guard let width = proposal.width, width.isFinite, width > 0 else { return nil }
        return CGSize(width: width, height: max(minimumHeight, uiView.paperHeight(width: width)))
    }
    static func dismantleUIView(_ view: NotebookPageHost, coordinator: NativeNotebookEditor.Coordinator) {
        coordinator.textViewDidChange(view.editor)
        view.ink.session.capturePendingStroke?()
    }
}

final class NotebookPageHost: UIView {
    let editor: UITextView
    let ink: NotebookInkHost
    private let placeholder = UILabel()
    private(set) var inkBoundaryButton = UIButton(type: .system)
    var continueHandwriting: (() -> Void)?
    private var inkBoundary: CGFloat?
    private var boundaryHeight: CGFloat = 0
    init(editor: UITextView, ink: NotebookInkHost) {
        self.editor = editor; self.ink = ink
        super.init(frame: .zero)
        backgroundColor = .clear
        addSubview(editor); addSubview(placeholder); addSubview(ink)
        placeholder.text = "Start a note…"
        placeholder.font = notebookBodyFont(); placeholder.textColor = UIColor.label.withAlphaComponent(0.35)
        placeholder.isUserInteractionEnabled = false; placeholder.isAccessibilityElement = false
        inkBoundaryButton.setTitle("Continue handwriting in a new section", for: .normal)
        inkBoundaryButton.titleLabel?.font = .preferredFont(forTextStyle: .caption1)
        inkBoundaryButton.titleLabel?.numberOfLines = 0
        inkBoundaryButton.backgroundColor = UIColor(red: 0.984, green: 0.980, blue: 0.969, alpha: 1)
        inkBoundaryButton.accessibilityHint = "This section has reached its handwriting height. Typed text continues below."
        inkBoundaryButton.addTarget(self, action: #selector(continueInk), for: .touchUpInside)
        inkBoundaryButton.isHidden = true; addSubview(inkBoundaryButton)
        clipsToBounds = true
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) unavailable") }
    @objc private func continueInk() { continueHandwriting?() }
    func refreshPlaceholder() { placeholder.isHidden = !editor.text.isEmpty || !ink.session.versions.isEmpty }
    func paperHeight(width: CGFloat) -> CGFloat {
        // An incoming head is deliberately not applied during a stroke. Measure
        // the canvas the reader can see, not that newer model's dimensions.
        let logical = ink.logicalCanvasSize
        let scale = width / logical.width
        // Reserve a measured native button-sized gap only when prose exceeds
        // the portable canvas limit. The boundary cannot obscure typed text.
        editor.textContainer.exclusionPaths = []
        var prose = editor.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude)).height
        let limit = NotebookInkHost.maximumCanvasHeight * scale
        inkBoundary = prose > limit ? limit : nil
        inkBoundaryButton.isHidden = inkBoundary == nil
        if let boundary = inkBoundary {
            let inset = editor.textContainerInset
            boundaryHeight = max(inkBoundaryButton.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude)).height,
                                 inkBoundaryButton.titleLabel!.font.lineHeight) + inset.top + inset.bottom
            editor.textContainer.exclusionPaths = [UIBezierPath(rect: CGRect(x: 0, y: max(0, boundary - inset.top), width: width, height: boundaryHeight))]
            prose = editor.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude)).height
        }
        return max(logical.height * scale, prose)
    }
    override func layoutSubviews() {
        super.layoutSubviews()
        guard bounds.width > 0 else { return }
        editor.frame = bounds
        // Text grows the section rather than creating a second scroll viewport.
        // Height extension never moves/reflows existing logical stroke points.
        let logicalWidth = ink.logicalCanvasSize.width
        let scale = bounds.width / logicalWidth
        ink.extendPaper(to: bounds.height / scale)
        ink.frame = CGRect(x: 0, y: 0, width: bounds.width, height: ink.logicalCanvasSize.height * scale)
        ink.setNeedsLayout(); ink.layoutIfNeeded()
        if let inkBoundary { inkBoundaryButton.frame = CGRect(x: 0, y: inkBoundary, width: bounds.width, height: boundaryHeight) }
        let inset = editor.textContainerInset
        let padding = editor.textContainer.lineFragmentPadding
        placeholder.frame = CGRect(x: inset.left + padding, y: inset.top,
                                   width: max(0, bounds.width - inset.left - inset.right - 2 * padding), height: notebookBodyFont().lineHeight)
    }
    /// Testable destination boundary used by hitTest, operating on UIKit touch
    /// types. A Pencil contact bypasses TextKit's Scribble recognizers immediately.
    func target(for point: CGPoint, touchType: UITouch.TouchType, event: UIEvent? = nil) -> UIView? {
        guard bounds.contains(point), !isHidden, isUserInteractionEnabled else { return nil }
        if touchType == .pencil { return ink.hitTest(convert(point, to: ink), with: event) }
        if !inkBoundaryButton.isHidden, inkBoundaryButton.frame.contains(point) { return inkBoundaryButton.hitTest(convert(point, to: inkBoundaryButton), with: event) }
        return editor.hitTest(convert(point, to: editor), with: event)
    }
    override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? {
        if let touches = event?.allTouches, let touch = touches.first {
            let kind: UITouch.TouchType = touches.contains { $0.type == .pencil } ? .pencil : touch.type
            return target(for: point, touchType: kind, event: event)
        }
        // Accessibility/programmatic hit testing has no physical touch. Keep the
        // ordinary editable text target; genuine Pencil events take the route above.
        return target(for: point, touchType: .direct, event: event)
    }
}
#endif
