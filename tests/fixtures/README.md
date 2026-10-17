# Provider consumer fixture

`provider-sample.epub` is an original synthetic three-chapter book authored during
Book Nook's acceptance work. It contains no purchased text, account information or
user notes. Its SHA-256 is
`f1f54598f34360d2df0bcd38342d2466e83cd568a6b52585947d6f94d59907b4`.

The app workflow and independently packaged Swift consumer use these same bytes
to verify replaceable content providers and native EPUB ingestion. Fixture text
is available under `MIT OR Apache-2.0`, like the SDK examples.
