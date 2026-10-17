// SPDX-License-Identifier: MPL-2.0
import SwiftUI

/// Reorders placement, not view identity: swapping sides keeps the native
/// notebook and reader instances, focus, scroll positions and selection intact.
struct NotebookWorkspaceLayout: Layout {
    var notebookOnRight: Bool
    var notebookFraction: Double = 0.51
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize { proposal.replacingUnspecifiedDimensions() }
    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        guard subviews.count == 3 else { return }
        let frames = NotebookPaneFrames(size: bounds.size, notebookOnRight: notebookOnRight, notebookFraction: notebookFraction)
        let placements = [frames.notebook, frames.divider, frames.reader]
        for (view, frame) in zip(subviews, placements) {
            view.place(at: CGPoint(x: bounds.minX + frame.minX, y: bounds.minY + frame.minY), anchor: .topLeading, proposal: ProposedViewSize(frame.size))
        }
    }
}

