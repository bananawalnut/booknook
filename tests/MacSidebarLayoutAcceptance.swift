// SPDX-License-Identifier: MPL-2.0
import AppKit
import SwiftUI

@MainActor private final class SidebarLayoutScenario: ObservableObject {
    @Published var sidebarVisible = true
    var detailAppearances = 0
    var detailDisappearances = 0
}

private struct SidebarLayoutFixture: View {
    @ObservedObject var scenario: SidebarLayoutScenario
    var body: some View {
        MacSidebarSplit(isVisible: scenario.sidebarVisible) {
            Text("Book Nook").padding(20)
        } detail: {
            Text("Reading desk")
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .onAppear { scenario.detailAppearances += 1 }
                .onDisappear { scenario.detailDisappearances += 1 }
        }
    }
}

@MainActor func testMacSidebarLayout() async throws {
    let scenario = SidebarLayoutScenario()
    let hosted = NSHostingView(rootView: SidebarLayoutFixture(scenario: scenario))
    let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1320, height: 860),
                          styleMask: [.borderless], backing: .buffered, defer: false)
    window.contentView = hosted
    defer { window.contentView = nil }

    func splitView(in view: NSView) -> NSSplitView? {
        if let split = view as? NSSplitView { return split }
        return view.subviews.lazy.compactMap { splitView(in: $0) }.first
    }

    func verify(_ label: String, predicate: (NSSplitView) -> Bool) async throws -> NSSplitView {
        let deadline = ContinuousClock.now.advanced(by: .seconds(2))
        repeat {
            hosted.layoutSubtreeIfNeeded()
            if let split = splitView(in: hosted), predicate(split) { return split }
            try await Task.sleep(for: .milliseconds(10))
        } while ContinuousClock.now < deadline
        let widths = splitView(in: hosted)?.arrangedSubviews.map(\.frame.width) ?? []
        throw NSError(domain: "Mac sidebar layout", code: 1,
                      userInfo: [NSLocalizedDescriptionKey: "\(label), observed widths: \(widths)"])
    }

    func clampedAndVisible(_ split: NSSplitView) -> Bool {
        let widths = split.arrangedSubviews.map(\.frame.width)
        let sidebarBounds: ClosedRange<CGFloat> = 239.5...320.5
        return widths.count == 2 && sidebarBounds.contains(widths[0]) && widths[1] >= 459.5
    }

    var split = try await verify("Open at the default window width", predicate: clampedAndVisible)
    split.setPosition(150, ofDividerAt: 0)
    split = try await verify("Clamp the released 150-point sidebar") {
        clampedAndVisible($0) && abs($0.arrangedSubviews[0].frame.width - 240) <= 0.5
    }
    split.setPosition(390, ofDividerAt: 0)
    _ = try await verify("Clamp an over-wide sidebar") {
        clampedAndVisible($0) && abs($0.arrangedSubviews[0].frame.width - 320) <= 0.5
    }
    window.setContentSize(NSSize(width: 735, height: 560))
    _ = try await verify("Fit sidebar and detail at a narrow window width") {
        clampedAndVisible($0) && abs($0.frame.width - 735) <= 0.5
    }
    scenario.sidebarVisible = false
    _ = try await verify("Give the whole width to the existing detail when hidden") {
        $0.arrangedSubviews.count == 1 && abs($0.arrangedSubviews[0].frame.width - 735) <= 0.5
    }
    scenario.sidebarVisible = true
    _ = try await verify("Restore the bounded sidebar", predicate: clampedAndVisible)
    precondition(scenario.detailAppearances == 1 && scenario.detailDisappearances == 0,
                 "Showing or hiding the sidebar must preserve the detail view’s identity")
    print("Mac sidebar runtime layout passed: width bounds, narrow window, hide/reopen, and detail identity")
    try await testShortSidebar()
}

/// Exercise the real sidebar without ordering a window on screen or changing system accessibility settings.
@MainActor private func testShortSidebar() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("booknook-short-sidebar-\(UUID())")
    defer { try? FileManager.default.removeItem(at: root) }
    let model = DeskModel(repository: DeskRepository(directory: root))
    await model.open()
    var bookID = ""
    for index in 1...12 {
        let result = try await model.perform(["op": "add_book", "title": "Sidebar fixture \(index)", "author": "Fixture", "at": index])
        guard let id = result?.object?["book_id"] as? String else { throw CocoaError(.coderReadCorrupt) }
        bookID = id
    }
    try await model.selectBook(bookID)
    guard let notebook = model.notebook, let block = notebook.blocks.first else { throw CocoaError(.coderReadCorrupt) }

    func descendant<T: NSView>(_ type: T.Type, in view: NSView) -> T? {
        if let found = view as? T { return found }
        for child in view.subviews {
            if let found = descendant(type, in: child) { return found }
        }
        return nil
    }
    func axFrames(in view: NSView, scroll: NSScrollView, sampling viewport: NSRect, roles: inout Set<String>) -> [String: NSRect] {
        var frames: [String: NSRect] = [:]
        var visited = Set<ObjectIdentifier>()
        func visit(_ node: NSObject) {
            // SwiftUI exposes both native views and virtual AppKit accessibility elements.
            func attribute(_ key: String) -> Any? {
                node.responds(to: NSSelectorFromString(key)) ? node.value(forKey: key) : nil
            }
            if let id = attribute("accessibilityIdentifier") as? String,
               let frame = attribute("accessibilityFrame") as? NSValue {
                frames[id] = frame.rectValue
            }
            guard visited.insert(ObjectIdentifier(node)).inserted else { return }
            let role = attribute("accessibilityRole") as? String ?? "-"
            let label = attribute("accessibilityLabel") as? String ?? ""
            roles.insert("\(type(of: node))|\(role)|\(label)")
            // Scroll areas may expose the document via AXContents and the
            // visible subset separately. Walk only forward containment edges.
            for key in ["accessibilityChildren", "accessibilityContents", "accessibilityVisibleChildren",
                        "accessibilityChildrenInNavigationOrder"] {
                for child in attribute(key) as? [NSObject] ?? [] { visit(child) }
            }
            if let native = node as? NSView { native.subviews.forEach(visit) }
        }
        visit(view)
        // Hidden SwiftUI scroll views virtualize child enumeration. Native hit-testing
        // still returns the actual semantic control at a visible viewport point.
        // This reads accessibility only; it neither clicks nor changes focus.
        // HSplitView embeds each arranged pane in its own hosting view. Query
        // the scroll view's ancestor chain without depending on private class names.
        var owners: [NSView] = []
        var ancestor: NSView? = scroll
        while let owner = ancestor {
            owners.append(owner)
            if owner === view { break }
            ancestor = owner.superview
        }
        for x in stride(from: viewport.minX + 3, to: viewport.maxX - 3, by: CGFloat(12)) {
            for y in stride(from: viewport.minY + 3, to: viewport.maxY - 3, by: CGFloat(6)) {
                for owner in owners {
                    if let hit = owner.accessibilityHitTest(NSPoint(x: x, y: y)) as? NSObject { visit(hit) }
                }
            }
        }
        return frames
    }
    for size: DynamicTypeSize in [.large, .accessibility5] {
        let hosted = NSHostingView(rootView: RootView().environmentObject(model).dynamicTypeSize(size)
            .environment(\.accessibilityEnabled, true))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 820, height: 400),
                              styleMask: [.borderless], backing: .buffered, defer: false)
        window.contentView = hosted
        defer { window.contentView = nil }
        func settle() async throws {
            hosted.layoutSubtreeIfNeeded()
            try await Task.sleep(for: .milliseconds(60))
            hosted.layoutSubtreeIfNeeded()
        }
        try await settle()
        guard let sidebar = descendant(NSSplitView.self, in: hosted)?.arrangedSubviews.first,
              let scroll = descendant(NSScrollView.self, in: sidebar), let doc = scroll.documentView else {
            throw NSError(domain: "Short sidebar missing its scroll view", code: 1)
        }
        let clip = scroll.contentView
        let viewport = window.convertToScreen(clip.convert(clip.bounds, to: nil)).insetBy(dx: -1, dy: -1)
        let windowBounds = window.convertToScreen(hosted.convert(hosted.bounds, to: nil)).insetBy(dx: -1, dy: -1)
        let limit = doc.bounds.height - clip.bounds.height
        guard limit > 0, clip.bounds.height > 0, hosted.bounds.height <= 401, windowBounds.contains(viewport) else {
            throw NSError(domain: "Short sidebar has no usable constrained viewport", code: 2)
        }
        var reached = Set<String>()
        let targets: Set<String> = ["sidebar-goodreads", "sidebar-add-book", "sidebar-book-\(bookID)"]
        var observed = Set<String>()
        var observedRoles = Set<String>()
        var frameSamples = [String]()
        let steps = Int(ceil(limit / max(1, clip.bounds.height / 3)))
        for index in 0...steps {
            clip.scroll(to: NSPoint(x: clip.bounds.minX, y: doc.bounds.minY + limit * CGFloat(index) / CGFloat(steps)))
            scroll.reflectScrolledClipView(clip)
            try await settle()
            // SwiftUI owns virtual AX controls at the hosting root; the split's
            // arranged NSView owns only the physical sidebar viewport.
            let frames = axFrames(in: hosted, scroll: scroll, sampling: viewport, roles: &observedRoles)
            observed.formUnion(frames.keys.filter { $0.hasPrefix("sidebar") })
            if index == 0 || index == steps {
                let positions = targets.sorted().map { "\($0)=\(String(describing: frames[$0]))" }
                frameSamples.append("clip \(clip.bounds): \(positions.joined(separator: ", "))")
            }
            for id in targets {
                if let frame = frames[id], !frame.isEmpty, viewport.contains(frame) { reached.insert(id) }
            }
        }
        guard reached == targets else {
            throw NSError(domain: "Short sidebar \(size) unreachable controls: \(targets.subtracting(reached).sorted()); viewport \(viewport); observed \(observed.sorted()); \(frameSamples); roles \(observedRoles.sorted().prefix(80))", code: 3)
        }
        guard model.section == "desk", model.selectedBookId == bookID, model.notebook?.id == notebook.id,
              model.notebook?.blocks.contains(where: { $0.id == block.id }) == true else {
            throw NSError(domain: "Scrolling sidebar changed reading context", code: 4)
        }
    }
    print("Mac 400-point sidebar passed at standard and accessibility text sizes with reading context retained")
}
