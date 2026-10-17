# Current app screenshots

The main README uses `macos-current.jpg` and `ipad-current.png`, captured on October 16, 2026 (America/Vancouver) from version **0.1.0, build 2026101603**, at source revision `53256f58592648a47794c9ef75d61726f3d203cd`.

| Screenshot | App target | Build | Capture environment |
|---|---|---|---|
| [macOS reading desk](macos-current.jpg) | `BookNookMac` / `org.booknook.desktop` | Release | Native macOS app window |
| [iPad reading desk](ipad-current.png) | `BookNookPad` / `org.booknook.ipad` | Release | iPad Air 13-inch (M3), iPadOS 18.6 Simulator, landscape |

These are captures of the normal app targets, including their current pixel-art mark and inline sidebar controls. No alternate layout, UI test host or image-generated interface was used. Both Release builds succeeded before capture. The build number matches the version installed on the physical iPad; the published iPad image is a Simulator capture.

The isolated libraries were created with [`seed-preview.py`](../../scripts/seed-preview.py). The books, prose and character interpretation are original sample content. No personal library, private note, commercial ebook, connection setting or credential is included.

The macOS image is an unaltered app-window capture. The iPad image comes from `simctl io screenshot`; its portrait framebuffer was turned 90 degrees clockwise to match the observed landscape display. The app interface and text were not retouched or composited. Capture hashes and build identities are recorded in [current.json](current.json).

These images document current rendering. They do not establish physical Pencil input, accessibility acceptance, integration qualification or a qualified production release. Files named `*-preview.png` remain historical evidence linked from the validation and UX records; they are no longer the README's current screenshots.
