# Book Nook Swift SDK

This standalone Swift Package exposes three products: `BookNookNative` for the UniFFI bridge, `BookNookServices` for injectable source workflows, and `CastaliaNative` for reusable custody, secure storage and consent. It contains generated Swift, handwritten public facades, a static XCFramework, SDK license texts and the dependency inventory. It does not include the MPL reference application's SwiftUI views. It supports macOS 14+, iPadOS 17+, and the arm64 Mac, iOS device and iOS Simulator slices built by this workspace. Intel Mac/simulator slices are not supplied.

Generate it from source with `bash scripts/package-swift.sh`. Full Xcode, Rust Apple targets and XcodeGen are prerequisites. The script builds all three targets, generates the package, runs an independent Swift consumer against a temporary desk, and creates `target/packages/booknook-swift-sdk-0.1.0.tar.gz`. After running all three matching Apple builds already, `--use-built` skips rebuilding. Do not use that option with stale native libraries. Nothing is signed or remotely published.

Extract the archive and use Xcode's local package option, or add:

```swift
.package(path: "/path/to/booknook-swift-sdk-0.1.0")
```

Depend on `.product(name: "BookNookNative", package: "booknook-swift-sdk-0.1.0")`, then:

```swift
import BookNookNative
let desk = try Desk(directory: localDeskDirectory)
let snapshotJSON = try desk.snapshot()
```

Run persistence, imports and networking off the UI actor. The public JSON commands are identical to the Rust/WASM contracts; pass complete UTF-16 formatting spans when staging text edits. Native readers can use `stageTextEdit`, `flushTextEdits`, `exportArchive` and the same explicit-consent custody/synchronization APIs consumed by the reference app. Providers and credentials remain configured by the embedding host.

`validateNativeAuthorityProfile` and `validateProviderConnections` validate the shared Castalia public contracts from original JSON. `previewCastaliaProfile`, `installCastaliaProfile` and `exportCastaliaProfile` separate public trust review from holder custody; hosts must obtain explicit user review before installation. Use `saveConnectionSettingsIfCurrent` with the settings snapshot the user edited to prevent stale windows overwriting trust. Source-bound Kindle/audio APIs preserve optional provider connection references without exporting credentials. No separate Book Nook membership is required, and a wallet-managed descriptor does not provide a wallet transport implementation.

For source composition, depend on `BookNookServices`, implement its `BookNookProvider` protocol, and construct `BookNookServices(directory:providers:)`. Providers supply immutable descriptor JSON and asynchronous responses; Rust validates both before committing. `importCatalog`, `importContent`, `sourcePositions`/`attachPosition`, `identity`, `recommendations` and `exchange` use the registered implementation. Catalog, content and position capabilities remain separate. Cancellation is checked between asynchronous reads and synchronous commits; registration or connection changes invalidate pending reads. The reference app uses this same facade.

For a different Castalia app, depend on `CastaliaNative` and provide storage, current trust policy, issuer request and consent adapters. No Book Nook desk or account is required. See [Castalia component construction and migration](CASTALIA.md). Native keys stay inside custody; generic verification does not grant application permission or consume replay keys.

For standalone notebook state with a replaceable store, use the `BookNookServices` product's `BookNookNotebook` facade:

```swift
import BookNookServices
let local = try BookNookNotebook(localDirectory: isolatedNotebookDirectory)
// Your host initializes valid State and implements atomic, durable compare-and-save.
let alternate = try BookNookNotebook(storage: applicationNotebookStore)
let snapshotJSON = try alternate.snapshotJSON()
let responseJSON = try alternate.execute(commandJSON: commandJSON)
```

`BookNookNotebookStorage` supplies synchronous, thread-safe `descriptorJSON()`, `loadJSON()` and `compareAndSave(expectedRevision:stateJSON:)`. Its descriptor uses the existing core shape, for example `{"id":"example.notebooks","capabilities":["notebook_storage"],"available":true,"reason":null}`. A save returns `true` only after committing the complete state; `false` means a revision conflict and throwing means I/O failure. Neither failure may partially replace state. The host must serialize all writers, including other processes, at this boundary. Rust rereads availability and state for each call and rejects a changed descriptor before saving. There is no retry or cached successful projection after a conflict. No Rust mutex spans host callbacks; the host is responsible for safe callback locking. Use bounded local I/O and run it off the UI actor.

This store receives private notebook state. It is separate from the asynchronous catalog/content provider interface, which does not receive the notebook. It also does not replace `Desk`'s composed local settings, social state, assets or write-ahead text journal. The reference app continues using that journaled `Desk` path. Use a separate directory for standalone local notebooks; do not share it with an active `Desk`. Existing native methods, archives and commands are unchanged. The underlying UniFFI types are `NotebookEngine` and `NotebookStorageHost` in `BookNookNative`.

The package script builds its independent consumer first, then generates a fresh public synthetic Castalia fixture. It exercises real SQLite/provider persistence and a second application's approval, and independently verifies the resulting receipt. These checks use synthetic providers and recovery material; they do not qualify a live account, issuer or Keychain installation.

The checked-in source workspace deliberately omits generated binaries and duplicate Swift output. Run the package script before adding this directory as a local Swift dependency. A generated archive is self-contained. Review `LICENSE-BOUNDARIES.md` and the actual dependency closure before distributing a binary release.
