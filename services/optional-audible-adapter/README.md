# Optional Audible source adapter

This is a separate **AGPL-3.0-only** Python process, explicitly installed by the companion operator. The Apple application, Rust/Swift/WASM SDKs, and their build scripts do not install, embed, import or invoke the upstream Python package directly. Process separation is an architectural boundary, **not a licensing exemption**. Anyone distributing or modifying this adapter must review the AGPL source and network-use obligations, the included notices, and the dependency inventory.

Implemented operations use pinned `audible==0.10.0`: external Amazon browser login with PKCE, completion after browser MFA, virtual-device registration, forced access-token refresh, deregistration of only this device, and authenticated listening-position reads. No password or OTP is submitted to Book Nook. Playback stays in Audible. No content license, DRM removal or audiobook download is implemented.

`qualified` remains **false**: synthetic tests validate code and source contracts; no real Amazon account has been connected in this implementation session. The adapter advertises `audible.positions` because a real request/parser path exists. That capability is distinct from live-account qualification. Kindle login and a live Kindle catalog are **not implemented** and are not advertised. Local Kindle XML/JSON catalog import remains available through the companion.

## Explicit operator installation (macOS first)

Use Python 3.12 with the macOS native Keychain backend. These commands are optional; no app build runs them:

```sh
python3.12 -m venv work/optional-audible-venv
work/optional-audible-venv/bin/python -m pip install --require-hashes -r services/optional-audible-adapter/requirements-macos.lock
python3 services/library-companion/booknook_companion.py --data work/library-companion \
  --provider-command "$PWD/work/optional-audible-venv/bin/python" \
  "$PWD/services/optional-audible-adapter/booknook_audible.py" \
  --data "$PWD/work/optional-audible-state"
```

The lock and [inventory](DEPENDENCY-LICENSES.csv) cover the resolved macOS/Python 3.12 runtime. A Linux Secret Service backend is accepted by the source, but its extra platform dependencies and deployment require a separate lock/inventory and acceptance. Windows is not supported by the process lock implementation. Plaintext and chained/fallback keyrings are rejected. Keep the data directory private (`0700`); it stores only a lock file. Credentials and PKCE sessions live in the selected native credential store, namespaced by the absolute state directory. Moving that directory changes the keyring namespace.

On macOS, replacement updates only the existing item's value with Apple's [`SecItemUpdate`](https://developer.apple.com/documentation/security/secitemupdate(_:_:)); a missing item uses [`SecItemAdd`](https://developer.apple.com/documentation/security/secitemadd(_:_:)), with one update retry if another writer created it first. Replacement never deletes the old item. This avoids the delete-before-add behavior in [pinned keyring 25.6.0](https://github.com/jaraco/keyring/blob/v25.6.0/keyring/backends/macOS/api.py), which can lose working credentials or an unfinished-registration marker if adding the replacement fails. Queries match the exact generic-password service and account. The JSON format and namespace are unchanged, so existing records remain readable through the pinned backend. The default Keychain selection and existing access controls remain in effect; this adds no custom keychain-path or access policy. The pinned backend itself ignores `KEYCHAIN_PATH`. Production interaction behavior is unchanged. See Apple's [updating and deleting keychain items](https://developer.apple.com/documentation/security/updating-and-deleting-keychain-items) guidance.

In Book Nook → Connections:

1. Pair the companion with its one-use code; choose the Audible marketplace.
2. Choose **Connect Audible account in companion**. Complete Amazon login and MFA in the browser.
3. Copy the full final Amazon `/ap/maplanding` URL into the masked field under Library companion. It contains a secret authorization code; do not paste it into notes or send it to someone else.
4. Read the virtual-device disclosure and explicitly select **Register a virtual Audible device in my Amazon account**, then **Complete registration**. The upstream registration presents itself as Audible for iPhone and appears in Amazon’s device list. This is an unofficial client, not an Amazon-supported Book Nook integration.
5. Enter an audiobook ASIN and choose **Read position through companion**. Review the position’s source time and ebook-edition match before saving. **Renew Audible account access** makes a real forced token refresh; opening a login URL alone never reports connected access.
6. **Disconnect companion Audible account** deregisters only that paired device’s virtual registration and removes its stored credentials. Do this before unpairing. Unpairing the companion only revokes the companion token; it does not remove an Amazon device.

Sessions expire after ten minutes, are bound to the paired-device digest and marketplace, and are consumed before registration starts. The app saves only the pending session descriptor to Keychain, so it can complete or cancel after restarting. The final returned URL stays in memory and is never persisted. Cancel discards the session; expired or already consumed sessions cannot register. A private lock serializes concurrent subprocesses, so two completion requests cannot register twice.

If registration reaches Amazon but fails before credentials can be saved (including process termination), the adapter keeps an uncertainty marker and refuses another registration. Check Amazon’s device list and remove the newly created virtual Audible device if present. In Book Nook, choose **Resolve unfinished Audible registration**, read the disclosure, and confirm that review. This only clears this paired device’s local marker for the selected marketplace. It refuses to clear a valid account’s credentials, another paired device, or a state without an unfinished registration. It does not contact Amazon, claim remote removal, or automatically retry registration. You can then start login again.

## Position and metadata evidence

`provider.connections.list` accepts a paired device ID and marketplace and reports a shared `castalia.provider-connection.v1` descriptor only when that exact secure-store account exists. It returns a nonsecret reference, `secure_local` storage and `reauthorization_required` portability. Listing does not call Amazon, renew tokens or export them. IDs remain stable across restarts and renewals at the same store. A successful new registration rotates the reference, preserving the distinction between account replacements. Existing stores without a generation keep a stable legacy reference until re-registration. A different device, marketplace or store has a separate reference. Availability is a local storage observation, not live qualification. Native reading provenance can retain the reference without carrying the account credential.

The [Python upstream API documentation](https://audible.readthedocs.io/en/latest/misc/external_api.html) establishes `GET /1.0/annotations/lastpositions`. The more specific response schema and UTC interpretation come from [audible-api-ts at commit a88fc1cf589184a3ca44c4e095c838745627c4a8](https://github.com/moifort/audible-api-ts/tree/a88fc1cf589184a3ca44c4e095c838745627c4a8), especially `src/schemas.ts`, `src/library.ts`, `src/library.unit.test.ts` and the separate credential-gated integration test. This is published unofficial-client evidence, not an Amazon API guarantee or a live observation by Book Nook. Its MIT notice is included.

The original parser accepts `asin_last_position_heard_annots` records with matching requested ASINs, `Exists` status, an explicit nonnegative integer `position_ms`, and `last_updated` in the documented UTC format. `DoesNotExist` and missing position/time evidence produce no observation. Unexpected statuses, malformed timestamps, future timestamps, duplicate or unrequested ASINs fail closed. Source timestamps remain stale when stale; `fetched_at` is separate. Requests use at most 25 ASINs per source batch and 100 unique inputs per companion request.

A separate bounded catalog read may add source title, authors and language for the edition matcher. Its failure leaves the valid position without metadata. No audiobook ASIN or ISBN is relabeled as an ebook identifier, and no percentage, duration or ebook position is guessed. Read-only probes discard their raw JSON. Responses and errors never expose stored credentials, the callback URL, raw provider errors, or profile details.

## Verification

```sh
# Pure state/parser tests also run without optional dependencies (upstream tests skip).
python3 -m unittest discover -s tests/amazon_adapter -v
# All 23 tests on macOS, including pinned upstream serializers and vault failure paths:
work/optional-audible-venv/bin/python -m unittest discover -s tests/amazon_adapter -v
# Real loopback HTTP and real adapter subprocess contract, synthetic credentials only:
python3 -m unittest discover -s tests/companion -v
```

All 23 adapter tests passed in the pinned macOS environment. Seven replacement regressions use real CoreFoundation encoding with intercepted native mutation calls: denied updates preserve existing credentials and unfinished-registration markers, Unicode round-trips, missing records use Add, and duplicate-add races retry Update without deleting the competing record. The separate companion suite covers loopback HTTP and adapter subprocess contracts. The upstream network transport is intercepted only in synthetic unit tests; production uses the real upstream client.

An additional opt-in acceptance check performs real Keychain create/read/reopen/update/empty-state/delete operations on one newly generated QA service/account. It never looks up production namespaces, calls Amazon, or instantiates an Audible client. Only this test's bounded worker disables optional UI, then restores its prior process setting. The output directory must not already exist:

```sh
work/optional-audible-venv/bin/python tests/amazon_adapter/keychain_acceptance.py \
  --allow-synthetic-keychain-record --output-dir work/audible-keychain-acceptance
```

This check passed on macOS on 2026-10-12, including exact synthetic JSON replacement, existing-namespace readability, cleanup with absence verification, and restored interaction settings. Ordinary tests and CI never invoke it. This establishes native storage behavior for synthetic data; real Amazon login/MFA, registration, refresh, OS-keyring prompts, observed position accuracy, and in-app browser completion remain live acceptance work. No account credentials or public network publications were used for these tests, and `qualified` remains false.
