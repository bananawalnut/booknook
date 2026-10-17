# Building and hosting

Book Nook requires no runtime Book Nook server. The native app stores SQLite and immutable content-addressed assets in Application Support. On Unix, the desk directory is owner-only. Local autosave and recovery require no signatures. Network publication and device pairing/transfer use explicitly reviewed Castalia actions.

## Native and SDK builds

`bash scripts/build-apple.sh mac|ipad-sim|ipad-device` generates the public Swift bridge with UniFFI and links Rust statically. XcodeGen creates the project from `apps/apple/project.yml`. Minimum targets are macOS 14 and iPadOS 17. Scripts disable bulky development debug symbols/incremental artifacts to limit disk usage. Builds are unsigned; configure an Apple team for physical installation, signing and notarization. Do not ship synthetic test data or QA environment paths as user state.

`bash scripts/build-wasm.sh web` emits the browser SDK; `nodejs` supports the parity tests. Install wasm-bindgen-cli 0.2.127 to match Cargo.lock. Serve `.wasm` as `application/wasm`. Supply atomic storage, independently pinned issuer configuration and appropriately isolated custody. Browser pages must never receive native signing secrets. Generated schemas/types are checked by `scripts/generate-contracts.py --check`.

## Normal local iPad Release installation

On 2026-10-16, the canonical app was installed and launched on the paired iPad using `BookNookPad`, Release configuration, bundle `org.booknook.ipad`, version 0.1.0 and build 2026101601. It keeps the production secure-storage service `org.booknook.castalia` and a separate app sandbox. The existing QA app is retained; installing the normal app does not migrate QA notes or connection credentials. Use the in-app private backup and restore flow deliberately if transferring a library.

Generate the canonical project with `xcodegen generate --spec apps/apple/project.yml`. Build with an explicit toolchain and task-local derived data, overriding the project's unsigned defaults using `CODE_SIGNING_ALLOWED=YES CODE_SIGN_STYLE=Automatic CODE_SIGN_IDENTITY='Apple Development' DEVELOPMENT_TEAM=<your-team>`. Select `-configuration Release -sdk iphoneos -destination 'id=<paired-device>'`; use `-allowProvisioningUpdates` when Xcode must create a profile for the canonical identifier. Install the resulting `BookNookPad.app` with `devicectl device install app`, then launch `org.booknook.ipad`.

The delivered local build has a device-specific Apple Development profile expiring at 2026-10-24 01:48:21 UTC (October 23 in Vancouver). Renew before expiry to keep launching it. This is a normal local app, not a TestFlight/App Store release or a certificate that all original integration gates passed. [Exact evidence](finger-local-release-evidence.json) and the signed archive's `VERIFICATION.json` record its scope and hashes.

## Native development archives

After successful builds, `python3 scripts/package-apple-apps.py --provenance docs/build-inputs.json` produces three complete app archives in the adjacent `booknook-apps` directory. It requires matching build provenance, checks each bundle’s platform, architecture and dynamic dependencies, independently extracts and compares every file, and writes a verification manifest and SHA-256 checksums. The macOS and Simulator development binaries retain their existing linker ad-hoc signatures; the device app is unsigned and is not an installable IPA. Developer signing/provisioning and physical qualification remain required. Each archive retains its actual debug library, reader resources and offline ontology; no synthetic/user desk or credentials are included.

## Locally consumable SDK packages

The following commands create packages and test independent consumers without publishing or requiring a registry account:

| Command | Output and verification |
| --- | --- |
| `bash scripts/package-rust.sh` | Four `target/package/booknook-*-0.1.0.crate` archives. Unpacks the actual archives into a separate workspace, applies local patches for the unpublished Book Nook siblings, and runs all packaged tests and examples offline. |
| `bash scripts/package-typescript.sh` | `target/packages/booknook-sdk-0.1.0.tgz`. Compiles CJS/ESM plus declarations, builds separate `pkg/web` and `pkg/nodejs` WASM assets, installs the tarball into an isolated consumer, and tests both JS entry points, both WASM initializers, and NodeNext declaration resolution. |
| `bash scripts/package-swift.sh` | `target/packages/booknook-swift-sdk-0.1.0.tar.gz`. Builds all Apple targets, generates a standalone local Swift Package with an XCFramework, and links/runs an independent Swift consumer against a synthetic temporary desk. `--use-built` skips the Apple builds only after matching builds of all three targets. |
| `python3 scripts/stage-sdk-deliverables.py` | Copies the six packages to the adjacent user-facing `booknook-sdk` directory with a README and SHA-256 checksums. |

The Swift package supplies arm64 macOS 14+, iPadOS 17+ and arm64 iOS Simulator slices. It does not claim Intel support or signed distribution. Its generated Swift and binary output are deliberately excluded from the source archive and are reproduced by the script. Handwritten `BookNookServices` and `CastaliaNative` sources are included in both source and binary SDK packages and compiled directly by the reference app. The independent Swift consumer builds first, then uses a fresh public synthetic Castalia credential to exercise a second application and verifies its receipt. Package source includes standard MIT/Apache license texts, dependency inventory boundaries and synthetic test fixtures. `scripts/sync-package-resources.py --check` checks package-local copies against the canonical source files; run it without `--check` after changing those files. No source package needs files outside its archive to compile its tests.

Cargo 1.96's built-in workspace package verification produced a temporary-registry `no hash listed` error for unpublished sibling crates in this environment. The Rust script therefore uses `cargo package --no-verify` **only for archive creation**, then independently builds and tests their extracted contents with explicit local patches. This is local source-package qualification, not proof of registry publication. CI uses these same local packaging checks and has no publication command or credential.

The workflow in `.github/workflows/check.yml` runs portable contracts, SDK checks, companion tests and Apple builds/reader tests. This file has been implemented locally; a remote GitHub Actions run is not claimed.

## Static ontology

Run `python3 scripts/serve-ontology.py --bind 127.0.0.1 --port 8787`. The host is restricted to the repository's `ontology` directory, independently of its working directory; `--port 0` selects and reports an available local port. Copy that directory to any static host for public release, publish `discovery.json`, preserve immutable versioned paths and verify manifest hashes. Follow every discovery manifest group, including the nested provider and portable protocol releases. Artifacts are also bundled in both native apps for offline use. `python3 tests/ontology-host.py` starts the actual host on loopback, retrieves every discovered manifest/artifact, checks its digest against the offline bundle, rejects a corrupt nested provider artifact, and verifies that a private working-directory file is not served. The app packaging check follows the same complete discovery graph and compares it to source. No public domain or hosting account is assumed or deployed.

## Optional library companion

`services/library-companion/booknook_companion.py` runs a paired provider host using the Python standard library. Its [README](../services/library-companion/README.md) documents startup, native pairing, token revocation, local Kindle metadata import and the bounded provider-executable protocol. A selected current Kindle-for-Mac SQLite snapshot can be normalized locally using the companion’s export command, then imported as metadata-only JSON. Raw database upload is accepted only on a loopback-bound companion from a loopback peer; Book Nook additionally requires a literal loopback URL and explicitly disables proxy routing for that request. The source is a cache snapshot, not proof of cloud login, ownership, position freshness or complete cloud coverage. Loopback is the default. iPad access to another machine requires properly trusted HTTPS/TLS and explicit pairing; do not expose a plaintext LAN credential endpoint. Browser origins are rejected rather than granted broad access.

The separately installed [optional Audible adapter](../services/optional-audible-adapter/README.md) supplies browser login, MFA handoff, virtual-device registration, completion, renewal, disconnect and source-position lookup through that protocol. Its AGPL-3.0-only source and pinned dependencies are separate from the native/SDK dependency graph; read its notices before installing or distributing it. Book Nook configures the companion and completes the flow in Connections. Uncertain registration has an explicit recovery action after the reader checks the Amazon device list. A locally forgotten pending login does not cancel a remote registration. The native app now also bundles an [experimental US Kindle web catalog connector](KINDLE-WEB-CONNECTOR.md), separate from the companion. It uses an isolated temporary Amazon session, imports metadata only after bounded endpoint enumeration, and retains no credential after closing. File catalog imports remain available. Neither synthetic account responses nor companion pairing qualify real Amazon behavior.

## Optional Castalia issuer

The [reference issuer](../services/castalia-issuer/README.md) provides a small optional HTTP service around the native `booknook-issuer` verifier/signer. Operators supply an owner-private signing seed, matching public pin, bearer-token digest and explicit holder/caller/action grants. No key, trust policy or production grant is provisioned automatically. Configure its endpoint, issuer pin, policy digest and caller in Book Nook Connections. The service receives only the operation digest; its explicit holder-supplied disclosure does not claim payload inspection. Native custody and recipients independently check the actual approved operation. `bash scripts/test-issuer.sh` runs the synthetic HTTP/native/receiver acceptance scenario.

## Recovery and operations

Keep the SQLite database, its WAL/SHM files and assets together while the app is running. Prefer the in-app export for a portable copy. It drains acknowledged edits and includes separate social history without credential/configuration material. Restore into an empty desk; use approved encrypted synchronization to reconcile existing private notebooks. Pairing secrets are device-only Keychain items and are deliberately not portable exports. Re-pair restored devices before transferring private snapshots.

Approved transfer copies remain in the outbox until explicitly removed. A local file export does not prove delivery. Relay outbox entries retain per-recipient acknowledgments. Re-check current trust, revoked transport versions, group epochs and membership before retrying sends.

Generate dependency notices with `python3 scripts/license-inventory.py`. Review the actual native/WASM build closures and retain required notices when distributing. No third-party graph is blanket relicensed. Publishing packages, signing/notarization, public ontology hosting and app-store distribution remain explicit release operations.

## Public preview scope

The current reading desk and Pencil canvas are native Apple interfaces; the WASM SDK does not supply a browser application. A public static preview with sample data, public SDK downloads or the versioned ontology can be hosted over HTTPS. Hosting the native bundle does not make its canvas run in a browser. The optional library companion is a personal paired service with loopback defaults and explicit network controls, not a public multi-tenant backend. Keep account connectors and signing custody behind appropriate private access while their production qualification remains open. No public runtime service or destination is configured by the iPad installation.
