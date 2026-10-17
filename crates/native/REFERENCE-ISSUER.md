# Optional Book Nook credential issuer

This personal, self-hostable reference issuer supplies the endpoint used by Book Nook's existing Connections and native approval flow. It issues the existing `castalia.request-credential.v1` inside `castalia.credential-presentation-request.v2`. It is a new Book Nook policy provider, not a Devgraph/secS adapter, membership issuer, or hosted Book Nook requirement.

The issuer checks explicitly configured clients and grants. It never creates a signing key, bearer token, grant, or trust configuration. Empty grants or clients deny every request. Offline reading and writing do not need this service.

## Build and configure

```sh
cargo build --locked --release -p booknook-native --bin booknook-issuer
```

The Rust binary uses the workspace's existing Ed25519 and canonical JSON implementations. The HTTP transport uses Python 3's standard library. The current private-file implementation supports Unix, including macOS and Linux; other platforms fail closed.

Prepare an operator-owned configuration directory with mode `0700`. The configuration and signing-key files must be regular owner-owned files with mode `0600`, no hardlinks, and no symlink path components. Use canonical absolute paths: macOS `/tmp` and `/var` are symlinks, so paths under them must use their resolved `/private/...` forms. Each file's immediate parent must also be owner-private. The supplied issuer key is **exactly 32 raw seed bytes**, independently provisioned through the operator's existing custody process. It is separate from a reader's Castalia recovery file. The issuer refuses a key that does not match the configured public key. Do not use the repository's public synthetic test keys.

Start from this deliberately nonfunctional example and replace every placeholder. Times are integer Unix seconds. A grant must be current and has an explicit end. The policy's version and entire canonical contents contribute to its digest.

```json
{
  "schema": "booknook.issuer-config.v1",
  "active": true,
  "issuer": "operator-chosen-issuer-name",
  "key_id": "operator-chosen-key-id",
  "public_key": "<issuer public key: 64 lowercase hex characters>",
  "signing_key_file": "/absolute/private/issuer/signer.raw32",
  "clients": [{
    "token_sha256": "<SHA-256 of an independently supplied access token>",
    "holder_public_key": "<reader public key: 64 lowercase hex characters>",
    "caller": {"kind": "terminal", "id": "booknook-native"}
  }],
  "policy": {
    "schema": "booknook.issuer-policy.v1",
    "audience": "booknook",
    "version": 1,
    "disclosure": {
      "mode": "holder_supplied_no_payload_inspection",
      "maximum_statements": 16,
      "maximum_statement_bytes": 512,
      "maximum_title_bytes": 160
    },
    "grants": [{
      "holder_public_key": "<reader public key: 64 lowercase hex characters>",
      "caller": {"kind": "terminal", "id": "booknook-native"},
      "actions": ["social.execute"],
      "disclosure_titles": ["Publish this reading selection"],
      "not_before": 0,
      "expires_at": 0
    }]
  }
}
```

The title above matches the current reading-selection composer, not every social action. Configure the exact title emitted by each Book Nook operation you intend to permit. Titles must match exactly. For example, sync uses `Send a private reading snapshot`, and `nostr.bind` uses `Connect your Nostr transport identity`. Pairing uses `Pair ` followed by the chosen device name. The closed action vocabulary is `sync.send`, `device.enroll`, `social.execute`, `social.publish`, `nostr.bind`, and `group.membership`; including an action here does not create receiver authority or membership. There are no wildcard holders, callers, actions, or disclosure titles. This native reference service accepts terminal callers only.

Current native controls use these exact values. For the two device rows, substitute the actual device name; the angle brackets are not sent. Select only the grants intended by your policy. Custom hosts define and review their own titles.

| Native operation | Action | Exact disclosure title |
|---|---|---|
| Connect or renew Nostr binding | `nostr.bind` | `Connect your Nostr transport identity` |
| Pair/enable a device | `device.enroll` | `Pair <device name>` |
| Disable a device | `device.enroll` | `Disable <device name>` |
| Approve private sync snapshot | `sync.send` | `Send a private reading snapshot` |
| Publish composer review, rating, update, note, or group discussion | `social.execute` | `Publish this reading selection` |
| Publish a selected notebook note | `social.execute` | `Publish only this selected note` |
| Retry a saved publication draft | `social.execute` | `Publish reviewed record` |
| Retract a publication | `social.execute` | `Retract this published selection` |
| Request friendship by identity code | `social.execute` | `Request mutual friendship` |
| Request/accept from a connected identity | `social.execute` | `Approve friendship` |
| End friendship | `social.execute` | `End mutual friendship` |
| Create/update shared list | `social.execute` | `Publish this shared reading list` |
| Delete shared list | `social.execute` | `Delete this shared reading list` |
| Create reading group | `social.execute` | `Create this reading group` |
| Request public group membership | `social.execute` | `Request public group membership` |
| Invite a reader | `social.execute` | `Invite a reader to this group` |
| Accept private invitation | `social.execute` | `Accept this group invitation` |
| Admit/remove member or change moderator | `social.execute` | `Change this group's membership` |
| Close group | `social.execute` | `Close this reading group` |
| Remove a group publication | `social.execute` | `Moderate this group discussion` |
| Ban a group participant | `social.execute` | `Ban this group participant` |

Use an access token from a secret manager with at least 32 random bytes of entropy. The service configuration stores only its SHA-256 digest. Each digest is bound to one configured holder and caller; possession of that token cannot obtain a credential for another holder. The token is sent only in the Authorization header and the signer's stdin, never command arguments, logs, exports, or credential output.

Print only the public values needed for Book Nook:

```sh
/absolute/path/booknook-issuer --config /absolute/private/issuer/config.json describe
```

That command verifies the supplied key and emits issuer, key ID, public key, audience, current policy digest, and active state. You can enter those values manually, or export the shared public profile for review in Book Nook:

```sh
/absolute/path/booknook-issuer --config /absolute/private/issuer/config.json profile \
  --endpoint http://127.0.0.1:8779/v1/credentials \
  --caller booknook-native --name "My Castalia authority" > authority-profile.json
```

The output is `castalia.native-authority-profile.v1`, using Castalia's shared schema and the same current policy digest as `describe`. The command checks the real supplied key and requires the chosen terminal caller to exist in a configured client. It accepts canonical HTTPS endpoints, or HTTP on exact loopback only; URLs with credentials, query, fragment, wildcard/trailing-dot hosts, port zero or noncanonical spellings are rejected. No endpoint is contacted. The profile contains no holder, access token, signing material, grants, or account membership. It remains exportable when the issuer is inactive or its grants are empty so configuration can be reviewed before activation; export does not demonstrate that any action is permitted.

In Book Nook → Connections, import the authority profile and explicitly review its issuer key, audience/caller, policy and endpoint before applying it. Select your existing Castalia identity independently; the profile cannot replace that identity. Enter the issuer access token separately in the existing secret field. Book Nook saves that token in Keychain under the exact endpoint. Neither export nor import automatically installs trust, recovery material, or action approval. Subsequent operations still need a current grant, fresh issuer credential, explicit reader approval and receiver validation. The same shared identity does not require a separate Book Nook account or membership credential.

## Serve

```sh
python3 services/castalia-issuer/booknook_issuer.py \
  --issuer-bin /absolute/path/booknook-issuer \
  --config /absolute/private/issuer/config.json \
  --host 127.0.0.1 --port 8779
```

Use `http://127.0.0.1:8779/v1/credentials` on the same Mac. For iPad or another host, bind an appropriate interface and provide `--cert /path/to/certificate.pem --key /path/to/tls-key.pem`; Book Nook requires a trusted HTTPS certificate away from exact loopback. The TLS certificate key is separate from the Castalia issuer key. Do not expose this personal reference HTTP server as a public multi-tenant service; deploy behind appropriate network access controls and request-rate limits.

Only `POST /v1/credentials` with one bearer header and a bounded JSON body is accepted. Browser origins, cookies, transfer encodings and duplicated critical headers are rejected. The wrapper never follows redirects or forwards to a remote signer. It starts only the operator-selected executable, without a shell, and bounds request/output size, simultaneous signing processes and runtime. Failures contain fixed machine codes, never raw signer errors or request material.

The request and response are the documented native endpoint contracts (source workspace: `services/library-companion/README.md#issuer-endpoint-contract`). No app or SDK contract change is needed. Configuration and key files are re-read for every request. Removing clients/grants or setting `active: false` prevents subsequent issuance. After a policy change, explicitly review and update the new digest in each receiver's Connections settings; already-issued credentials remain usable only under the receiver's current policy pin and their short expiry.

## What issuance means

A successful credential attests that the configured holder/caller/action and disclosure title met this issuer's current grants and validity limits, and binds the exact request and disclosure digests. The service receives the **payload digest**, not the notebook or social payload. It cannot attest that the disclosure accurately describes unseen content or that a group/list mutation is legal. The explicit `holder_supplied_no_payload_inspection` policy mode records that limit. The native app verifies its own canonical payload and displays the bound disclosure; fresh reader approval and the receiving application's resource, membership, replay and policy checks remain mandatory.

Credentials last at most 120 seconds and never outlive the selected grant. Each issuance has fresh randomness. Issuing or retrying a credential does not execute or reserve a mutation. Native custody consumes each prepared approval once; the receiving application owns persistent replay/idempotency checks. The issuer does not falsely claim a publication or synchronization occurred.

## Validation and licensing

```sh
bash scripts/test-issuer.sh
```

The regression suite uses synthetic issuer material and the existing published Castalia recovery vector. It tests real binary issuance, wrong client/caller/action/policy, expired/missing grants, bounded disclosure, configuration replacement, key mismatch and unsafe private files. The separate loopback scenario starts this actual HTTP server, obtains a real issuer signature, uses the native custody prepare/approve methods, and verifies the result with the independent core receiver. It also rejects wrong payloads, duplicate approvals, browser requests and revoked configuration. These tests do not establish real operator trust, native consent UI interaction, live Keychain or production deployment qualification.

New Rust issuer code is `MIT OR Apache-2.0`; the reference HTTP service is `MPL-2.0`. No external issuer code or new cryptographic package was copied or bundled. The additional direct `libc` declaration uses the already inventoried dependency to open private files without following links.
