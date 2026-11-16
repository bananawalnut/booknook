# Current app screenshots

The main README uses captures from the locally verified native **0.2.1 Release build 2026111603**, made on November 16, 2026 in America/Vancouver.

| Screenshot | App target | Capture environment |
|---|---|---|
| [macOS reading desk](macos-current.jpg) | `BookNookMac` / `org.booknook.desktop` | Native app window on macOS 26.4.1 |
| [iPad reading desk](ipad-current.png) | `BookNookPad` / `org.booknook.ipad` | iPad Air 13-inch (M2), iPadOS 18.6 Simulator, portrait |

Both images show the original synthetic EPUB *The Quiet Library* and two notes in the Margin. The Mac app uses a separate QA library. The dedicated iPad Simulator uses a verified copy of that synthetic desk. The Mac JPEG is the original window capture and the iPad PNG is the original `simctl io screenshot` framebuffer.

[Capture metadata](current.json) records the exact executables, source revisions and image hashes. The [normal iPad installation](../ipad-2026111601-launch-evidence.json) and [public preview](../app-2026111601-public-preview-evidence.json) are build 2026111601. Physical input and live integration checks remain open.
