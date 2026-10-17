# macOS Keychain adapter acceptance

The current `CustodySecureStorage` passed an actual macOS Security framework create/read/update/delete test with synthetic opaque bytes. The exact test record was removed and its absence verified. Production application code was unchanged.

This qualifies byte preservation and these storage operations on the tested Mac. It does not qualify app prompt behavior, denial under actual signing identities, the complete Castalia custody/recovery/authorization flow, real provider credentials, or physical iPad Keychain behavior. The values are opaque test bytes, **not validated encrypted custody envelopes**.

## Reproduce explicitly

On a local Mac with Xcode, from the repository root:

```sh
python3 scripts/test-keychain.py --allow-synthetic-keychain-record
```

The acknowledgement flag is required. The script rejects CI and non-macOS execution, and is not called by normal test scripts or CI jobs. It creates a new evidence directory under `target/keychain-acceptance/<UUID>`. `--output-dir /path/to/new-directory` selects a different location; an existing directory is rejected.

The runner stages the exact current [adapter](../apps/apple/BookNook/SecureStorage.swift) and [storage protocol source](../apps/apple/BookNook/RecoveryImport.swift). The **sole adapter change** is replacing its production service literal with a newly generated `org.booknook.qa.keychain.<UUID>` value. A separate random account identifies the test item. The production service is never queried. The [harness](../tests/KeychainAcceptance.swift) tests:

1. The exact random service/account is absent before any write.
2. Creating 45 bytes and reading identical bytes, including NUL, non-ASCII and UTF-8 data.
3. Updating to 280 different bytes, including every byte value, and reading identical bytes.
4. Removing the exact record, verifying absence, and repeating removal successfully.

The standalone harness disables optional Keychain interaction using `SecKeychainSetUserInteractionAllowed(false)`, verifies that setting, then restores its previous process state. This is a **test-harness difference from normal app behavior**. The API is deprecated and produces expected compiler warnings. Apple's published [SecKeychain implementation](https://github.com/apple-oss-distributions/Security/blob/main/OSX/libsecurity_keychain/lib/SecKeychain.cpp) and [framework Globals implementation](https://github.com/apple-oss-distributions/Security/blob/main/OSX/libsecurity_keychain/lib/Globals.cpp) keep this interaction flag within the framework's process state. No Keychain lock, trust, access-control, or machine preference is changed. The test never interacts with prompts or attempts to unlock a Keychain.

Execution is bounded to 20 seconds after compilation. Normal failures attempt prompt-disabled cleanup of only the exact owned record. After a timeout or terminated process, the runner allows one additional 10-second cleanup attempt only when durable checkpoints prove the exact random item was absent before the attempted write. It never deletes by service alone. A blocked cleanup is reported; the script does not broaden permissions, enumerate credentials, or change settings to force cleanup.

If the execution environment denies Keychain access, keep the failure receipt. Where authorized, run through the environment's normal permission review. Do not treat access denial as permission to bypass UI, trust, or account protections.

## Recorded result

The delivered script was run on macOS 26.4.1, arm64, with permission-reviewed access outside the filesystem sandbox. It passed in 0.349 seconds, exited 0, restored the process interaction setting with status 0, removed its exact test item, and verified absence. The original adapter and protocol hashes remained unchanged afterward.

| Evidence | SHA-256 |
| --- | --- |
| Production `SecureStorage.swift` | `0e90391950fd7d89fde1244fb9099e83b634e3a1f891a3ececb64cb372dae2d6` |
| Production `RecoveryImport.swift` | `27916bb7dda95e9151707a73c1e22a239b819e0ff6623efdd18a61b39f1bf8f2` |
| Delivered harness | `983712f7aae169bae8aa36cbc423dba02d12100d02f3dbce1a53aedbaaabf652` |
| Delivered runner | `fa963572c5bdc38edaaee0d96e6aa2fb5a5c8fe021ed84f15a9eaece97acab1b` |
| Delivered-script acceptance manifest | `258b61bd290e30514b91e16a86fcdbc74d51fdf258be4cff383a7d481ca32a20` |

The [delivered structured receipt](keychain-acceptance-evidence.json) includes the exact manifest, result and execution record. Full development receipts are in the workspace’s `work/opt-in-keychain-acceptance`: `manifest.json`, `result.json`, `lifecycle.execution.json`, build output and operation logs. Each run writes its own source, staged-source, harness, executable and evidence hashes; synthetic service/account identifiers are retained for cleanup audit.

The earlier exploratory sandboxed run returned OSStatus `-50` on its initial read, **before any write**. The same exploratory binary passed after permission-reviewed access. That sandbox failure is neither an application regression nor a successful storage check. Both exploratory and delivered-script successful runs verified deletion of their own distinct synthetic records. No real credentials were used.
