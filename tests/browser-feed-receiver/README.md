# Disposable receiving-host fixture

This is test code, not the future browser app. It uses the actual packaged Book Nook WASM and TypeScript review objects, preserves original incoming bytes, and saves only after a separate local review. Its actual IndexedDB transaction compares the exact saved row before a write. Imports never install a fetch provider or activate a source.

Prepare an unused output directory from an explicitly supplied tested npm archive:

```sh
python3 scripts/prepare-feed-browser-receiver.py --package /absolute/path/booknook-sdk-0.1.0.tgz --output /absolute/unused/fixture-directory
```

A disposable loopback test server serves `index.html` at `/receiver`, and the allowlisted files under `/booknook/`. The explicit request button calls the optional, capability-negotiated `window.castaliaWallet.selectFeeds()`; it does not request settings on page load. Use `#confirm-import` only after Wallet consent and receiving-host review. All untrusted previews and conflicts render through `textContent`.

`window.feedReceiverTest` exposes **fixture-only** ready/state/read, receive/resolve/confirm/cancel, validated seeding and an asynchronous pre-CAS barrier for deterministic competing-tab tests. It must not be included in a production app. No test hook bypasses the actual WASM parser, opaque review or IndexedDB comparison.

This configuration transaction is separate from the synchronous `FeedCacheStoreV1` cache-commit contract. Browser fixture results do not qualify native UI, physical devices, account access, production capability enablement or a released browser client.
