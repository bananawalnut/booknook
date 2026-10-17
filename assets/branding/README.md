# Book Nook icon

The original Book Nook artwork shows an ivory open book and amber ribbon on deep green. It was created for this project using OpenAI image generation. The source artwork and its Apple app-icon renditions are available under **MIT OR Apache-2.0**, like the project's original documentation and build assets.

`booknook-icon.png` is the high-resolution source image. On macOS, regenerate the complete Apple asset catalog from the repository root:

```sh
swift scripts/generate-app-icons.swift .
```

iPad renditions are opaque, full-bleed squares for the system mask. macOS renditions include transparent padding and a rounded tile for macOS 14 and later. The artwork contains no embedded text or third-party logo.
