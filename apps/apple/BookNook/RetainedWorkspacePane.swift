// SPDX-License-Identifier: MPL-2.0
import SwiftUI
#if os(iOS)
import UIKit
#endif

/// Keeps each pane's native views and SwiftUI state mounted. On iPad a real
/// UIKit ancestor owns visibility: hiding only SwiftUI layout metadata left
/// invisible reader controls in the device's accessibility hierarchy.
struct RetainedWorkspacePane<Content: View>: View {
    let visible: Bool
    let content: Content
    init(visible: Bool, @ViewBuilder content: () -> Content) {
        self.visible = visible; self.content = content()
    }
    var body: some View {
        #if os(iOS)
        NativeWorkspacePane(visible: visible, content: content)
        #else
        content.opacity(visible ? 1 : 0).allowsHitTesting(visible)
            .accessibilityElement(children: visible ? .contain : .ignore)
            .accessibilityHidden(!visible)
        #endif
    }
}

#if os(iOS)
private struct HostedWorkspaceContent<Content: View>: View {
    let content: Content
    let environment: EnvironmentValues
    let model: DeskModel
    var body: some View { content.environment(\.self, environment).environmentObject(model) }
}

private final class WorkspacePaneController<Content: View>: UIViewController {
    let host: UIHostingController<HostedWorkspaceContent<Content>>
    // SwiftUI manages the representable's root view. This inner container is
    // owned here, so its visibility cannot be reset by that outer layout pass.
    let boundary = UIView()
    init(root: HostedWorkspaceContent<Content>) {
        host = UIHostingController(rootView: root)
        super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) unavailable") }
    override func loadView() {
        view = UIView(); view.backgroundColor = .clear
        boundary.backgroundColor = .clear; boundary.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(boundary)
        addChild(host); host.view.backgroundColor = .clear; host.view.translatesAutoresizingMaskIntoConstraints = false
        boundary.addSubview(host.view); host.didMove(toParent: self)
        NSLayoutConstraint.activate([
            boundary.leadingAnchor.constraint(equalTo: view.leadingAnchor), boundary.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            boundary.topAnchor.constraint(equalTo: view.topAnchor), boundary.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            host.view.leadingAnchor.constraint(equalTo: boundary.leadingAnchor), host.view.trailingAnchor.constraint(equalTo: boundary.trailingAnchor),
            host.view.topAnchor.constraint(equalTo: boundary.topAnchor), host.view.bottomAnchor.constraint(equalTo: boundary.bottomAnchor)
        ])
    }
    func setVisible(_ visible: Bool) {
        loadViewIfNeeded()
        boundary.accessibilityElementsHidden = !visible
        boundary.isUserInteractionEnabled = visible
        boundary.isHidden = !visible
    }
}

private struct NativeWorkspacePane<Content: View>: UIViewControllerRepresentable {
    @EnvironmentObject private var model: DeskModel
    let visible: Bool
    let content: Content
    func makeUIViewController(context: Context) -> WorkspacePaneController<Content> {
        let controller = WorkspacePaneController(root: root(context))
        controller.setVisible(visible)
        return controller
    }
    func updateUIViewController(_ controller: WorkspacePaneController<Content>, context: Context) {
        controller.host.rootView = root(context)
        controller.setVisible(visible)
    }
    func sizeThatFits(_ proposal: ProposedViewSize, uiViewController: WorkspacePaneController<Content>, context: Context) -> CGSize? {
        proposal.replacingUnspecifiedDimensions()
    }
    private func root(_ context: Context) -> HostedWorkspaceContent<Content> {
        HostedWorkspaceContent(content: content, environment: context.environment, model: model)
    }
}
#endif
