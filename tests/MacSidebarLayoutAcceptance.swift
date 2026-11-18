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
}
