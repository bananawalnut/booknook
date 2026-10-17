// SPDX-License-Identifier: MPL-2.0
import Foundation
import CoreGraphics

/// A page keeps its logical coordinates; the native view, never its strokes,
/// scales to fit the notebook pane after rotation or a pane-side change.
struct NotebookCanvasGeometry {
    static let defaultSize = CGSize(width: 768, height: 960)
    let logicalSize: CGSize
    func scale(in available: CGSize) -> CGFloat {
        guard logicalSize.width > 0, logicalSize.height > 0,
              available.width > 0, available.height > 0 else { return 0 }
        return min(available.width / logicalSize.width, available.height / logicalSize.height)
    }
}

struct NotebookPaneFrames {
    let notebook: CGRect
    let divider: CGRect
    let reader: CGRect
    init(size: CGSize, notebookOnRight: Bool, notebookFraction: Double = 0.51, separatorWidth: CGFloat = 18) {
        let width = max(0, size.width), height = max(0, size.height)
        let separator = min(max(0, separatorWidth), width)
        let available = width - separator
        // Both contexts stay visible even in a narrow window. The minimum adapts
        // to available space instead of collapsing either pane into a tab.
        let minimum = min(220, available * 0.3)
        let fraction = notebookFraction.isFinite ? notebookFraction : 0.51
        let notebookWidth = min(max(minimum, available * fraction), max(minimum, available - minimum))
        let readerWidth = width - separator - notebookWidth
        notebook = CGRect(x: notebookOnRight ? readerWidth + separator : 0, y: 0, width: notebookWidth, height: height)
        divider = CGRect(x: notebookOnRight ? readerWidth : notebookWidth, y: 0, width: separator, height: height)
        reader = CGRect(x: notebookOnRight ? 0 : notebookWidth + separator, y: 0, width: readerWidth, height: height)
    }
}
