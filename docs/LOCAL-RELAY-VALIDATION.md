# Independent local relay acceptance

The native exchange path passed an interoperability test against a separate, unmodified `nostr-rs-relay` process. The test does not use a fake relay or fabricated acknowledgments. Every identity and publication is synthetic; the relay binds only to `127.0.0.1`, and no public relay is contacted.

The independent fixture is [nostr-rs-relay commit b5c1f642e4f4c3b9c54f5d18d66f4c53642076b4](https://github.com/scsibug/nostr-rs-relay/commit/b5c1f642e4f4c3b9c54f5d18d66f4c53642076b4), version 0.10.0, MIT licensed. Its source archive SHA-256 is `5debc36966f86ce012394bcb35d831ea3f792aeef7999780e8a77f9cf37ec1f0`. It is an optional test fixture and is not a Book Nook dependency or required runtime service.

The test passed these checks:

- Two independent native SQLite desks publish explicitly bound, independently verified synthetic Castalia identities.
- Actual relay `OK` responses clear each addressed outbox delivery; a second desk fetches and independently validates the exact approved record.
- Mutual friendship is reconstructed from both signed requests. Repeated fetches leave one logical publication and do not change the received count.
- Captured private relay records have one intended recipient tag, including self-delivery, and contain no plaintext action or Castalia identities.
- Killing the relay retains the approved retry record. Reopening the sender's desk retains it. Restarting the relay with the same database acknowledges the retry and empties the outbox; another refresh sends no duplicate.
- The relay retains its own database across the process restart.

The production application still accepts only `wss://` settings and uses the SDK's default TLS transport. The test injects a real SDK WebSocket implementation that maps its one loopback endpoint to local `ws://`; the shared production exchange, encryption, current policy checks, receipt verification, SQLite commits, and acknowledgment processing are unchanged. This does **not** qualify TLS, remote relay behavior, real issuer provisioning, Keychain lifecycle, or physical multi-device operation. No CA, trust-store, or system network setting is installed.

To reproduce, download the pinned source archive into disposable storage, verify its checksum, and build it using its own `Cargo.lock`:

```sh
curl -fL https://codeload.github.com/scsibug/nostr-rs-relay/tar.gz/b5c1f642e4f4c3b9c54f5d18d66f4c53642076b4 -o source.tar.gz
shasum -a 256 source.tar.gz
tar -xzf source.tar.gz
cd nostr-rs-relay-b5c1f642e4f4c3b9c54f5d18d66f4c53642076b4
CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo build --locked
```

The independent relay build requires Rust/Cargo and `protoc`. From the Book Nook repository, run:

```sh
BOOKNOOK_RELAY_BINARY=/absolute/path/to/nostr-rs-relay bash scripts/test-local-relay.sh
```

The test creates isolated temporary desks and a temporary relay database, starts and kills only its own child relay, and removes its temporary data afterward. It is intentionally ignored by ordinary workspace tests because it requires the independently built executable and permission to open localhost sockets. The explicit run passed with one test, no failures and no ignored tests in 12.55 seconds.

The latest expanded run passed in 12.51 seconds. It additionally exercises three independent native SQLite desks through private invitations, independent acceptances and one membership update, encrypted discussion, duplicate refresh, member removal, exact recipient exclusion, and reopened stale-outbox rejection. The [expanded receipt](private-group-replay-evidence.json) records its source hashes, failing-before evidence and repaired causal replay cases. The earlier 12.55-second result above is historical.

The complementary connector-level test remains in `crates/connectors/tests/private_admission.rs`, with matching native/WASM contracts in `tests/wasm-parity.cjs`. Real public-relay publication, real identities and physical devices remain unqualified.

## Remote default-TLS read-only acceptance

The opt-in [public relay probe](../scripts/probe-public-relay.sh) also passed against `wss://relay.damus.io`, an endpoint documented by the relay operator's [Damus project README](https://github.com/damus-io/notepush#installation). It calls the current `SocialRelayTransport::connect`, `fetch_page` and `disconnect` using the unmodified `DefaultWebsocketTransport`. The delivered script's run at 2026-10-12 20:10:59 UTC observed an actual connection, one outgoing `REQ`, one incoming `EOSE`, and one outgoing `CLOSE` in 243 ms, with no SDK errors or returned events. An empty fetch result alone is insufficient: the probe requires both the connection and protocol completion observations.

```sh
bash scripts/probe-public-relay.sh --read-only-public-relay wss://relay.damus.io
```

The flag and relay URL are required; this is never invoked by ordinary tests or CI. The [standalone test package](../tests/relay-read-probe/Cargo.toml) has its own locked workspace and uses the current connector through a path dependency. Its dependency versions and checksums match the main lockfile. It changes no application, crate or SDK package. Rust/Cargo and public-network access are required; a new build may download locked dependencies.

The network operation is bounded to 25 seconds overall, including an eight-second connection wait and the existing ten-second fetch bound. The query uses only public kind 30078, the `booknook` tag and a one-second time window; it supplies no author, recipient or user identity. No key or signer is created or loaded, so SDK authentication cannot sign or send an `AUTH` response. No publication or `EVENT` call is made. TLS validation, certificates, trust stores and application settings are unchanged. The SDK retains its normal transport controls such as ping/pong.

Output is one JSON summary of connection status, protocol message counts and result count. Raw relay records are neither logged nor saved. Success requires a completed fetch, zero SDK errors, a connection, exactly one outgoing `REQ` and `CLOSE`, and incoming `EOSE`; every other outgoing Nostr message type fails acceptance. An initial scratch run established a connection but its local trace parser missed message types and correctly reported failure. The parser was corrected before these successful delivered-script runs; the initial result is not counted as protocol-completion evidence.

Results describe that endpoint at the time of the probe; availability and policies can change. This evidence qualifies the default TLS/WebSocket connection and read exchange only. Remote publication acknowledgments, encrypted delivery, Castalia authority, account custody and full social operation remain separate acceptance requirements.
