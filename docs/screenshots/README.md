# Native app screenshots

The main README uses captures from the locally verified native **0.2.1 Release build 2026111603**, made on November 16, 2026 in America/Vancouver.

| Screenshot | App target | Capture environment |
|---|---|---|
| [macOS reading desk](macos-current.jpg) | `BookNookMac` / `org.booknook.desktop` | Native app window on macOS 26.4.1 |
| [iPad reading desk](ipad-current.png) | `BookNookPad` / `org.booknook.ipad` | iPad Air 13-inch (M2), iPadOS 18.6 Simulator, portrait |

Both images show the original synthetic EPUB *The Quiet Library* and two notes in the Margin. The Mac app uses a separate QA library. The dedicated iPad Simulator uses a verified copy of that synthetic desk. The Mac JPEG is the original window capture and the iPad PNG is the original `simctl io screenshot` framebuffer.

[Capture metadata](current.json) records the exact executables, source revisions and image hashes. The [latest delivery evidence](../app-2026111607-public-preview-evidence.json) distinguishes the Mac and iPad development preview at build 2026111607 and the normal physical iPad installation at build 2026111605. The screenshots above retain their original build identities. Physical input and live integration checks remain open.
