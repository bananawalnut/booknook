// SPDX-License-Identifier: MIT OR Apache-2.0
//! Real optional issuer binary/HTTP tests using only published or generated synthetic keys.
#![cfg(unix)]
use booknook_core::{authority::*, canonical, digest};
use booknook_native::Desk;
use ed25519_dalek::SigningKey;
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
const TOKEN: &str = "synthetic-issuer-access-only-never-production-00000001";
struct Fixture {
    root: PathBuf,
    config: Value,
    request: Value,
    payload: Vec<u8>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("booknook-issuer-{}", booknook_core::new_id()));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let root = root.canonicalize().unwrap();
        let vector: Value =
            serde_json::from_str(include_str!("fixtures/castalia-custody-vector.json")).unwrap();
        let public = hex::encode(
            SigningKey::from_bytes(&[0x43; 32])
                .verifying_key()
                .as_bytes(),
        );
        let now = now();
        let caller = json!({"kind":"terminal","id":"booknook-native"});
        let holder = &vector["public_key"];
        let policy = json!({"schema":"booknook.issuer-policy.v1","audience":"booknook","version":1,
            "disclosure":{"mode":"holder_supplied_no_payload_inspection","maximum_statements":8,"maximum_statement_bytes":512,"maximum_title_bytes":160},
            "grants":[{"holder_public_key":holder,"caller":caller,"actions":["social.publish"],"disclosure_titles":["Publish selected note"],"not_before":now-10,"expires_at":now+3600}]});
        let policy_digest = digest(&canonical(&policy).unwrap());
        let config = json!({"schema":"booknook.issuer-config.v1","active":true,"issuer":"synthetic-booknook-issuer","key_id":"synthetic-key",
            "public_key":public,"signing_key_file":root.join("signer.raw32"),"policy":policy,
            "clients":[{"token_sha256":digest(TOKEN.as_bytes()),"holder_public_key":holder,"caller":caller}]});
        let payload = canonical(&json!({"selected_note":"Reading café 🌿"})).unwrap();
        let request = json!({"schema":"booknook.credential-request.v1","action_request":{"schema":"booknook.authorized-action.v1","action":"social.publish",
            "actor":holder,"payload_digest":digest(&payload),"resource_version":0,"idempotency_key":"synthetic-request"},
            "holder_public_key":holder,"caller":caller,"audience":"booknook","policy_digest":policy_digest,
            "disclosure":{"title":"Publish selected note","statements":["Publish only the selected synthetic note."]}});
        let value = Self {
            root,
            config,
            request,
            payload,
        };
        value.write("signer.raw32", &[0x43; 32]);
        value.save();
        value
    }
    fn write(&self, name: &str, bytes: &[u8]) {
        fs::write(self.root.join(name), bytes).unwrap();
        fs::set_permissions(self.root.join(name), fs::Permissions::from_mode(0o600)).unwrap();
    }
    fn save(&self) {
        self.write("issuer.json", &canonical(&self.config).unwrap());
    }
    fn call(&self, token: &str, request: &Value) -> std::process::Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_booknook-issuer"))
            .args([
                "--config",
                self.root.join("issuer.json").to_str().unwrap(),
                "issue",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&canonical(&json!({"schema":"booknook.issuer-call.v1","authorization":format!("Bearer {token}"),"request":request})).unwrap()).unwrap();
        child.wait_with_output().unwrap()
    }
    fn policy(&self) -> AuthorityPolicy {
        AuthorityPolicy {
            pins: vec![TrustPin {
                issuer: "synthetic-booknook-issuer".into(),
                key_id: "synthetic-key".into(),
                public_key: self.config["public_key"].as_str().unwrap().into(),
                audience: "booknook".into(),
                callers: vec![Caller {
                    kind: "terminal".into(),
                    id: "booknook-native".into(),
                }],
            }],
            expected_caller: Caller {
                kind: "terminal".into(),
                id: "booknook-native".into(),
            },
            audience: "booknook".into(),
            actor: self.request["holder_public_key"].as_str().unwrap().into(),
            action: "social.publish".into(),
            policy_digest: self.request["policy_digest"].as_str().unwrap().into(),
        }
    }
    fn profile(&self, endpoint: &str, caller: &str, name: &str) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_booknook-issuer"))
            .args([
                "--config",
                self.root.join("issuer.json").to_str().unwrap(),
                "profile",
                "--endpoint",
                endpoint,
                "--caller",
                caller,
                "--name",
                name,
            ])
            .output()
            .unwrap()
    }
    fn desk(&self, endpoint: &str) -> Desk {
        use booknook_core::settings::*;
        let p = self.policy();
        let pin = &p.pins[0];
        let desk = Desk::new(self.root.join("desk").to_string_lossy().into()).unwrap();
        desk.save_connection_settings(
            serde_json::to_string(&ConnectionSettings {
                castalia: Some(CastaliaSettings {
                    issuer: pin.issuer.clone(),
                    key_id: pin.key_id.clone(),
                    issuer_public_key: pin.public_key.clone(),
                    audience: pin.audience.clone(),
                    terminal_caller: p.expected_caller.id,
                    holder_public_key: p.actor,
                    policy_digest: p.policy_digest,
                    credential_endpoint: endpoint.into(),
                }),
                ..Default::default()
            })
            .unwrap(),
        )
        .unwrap();
        let vector: Value =
            serde_json::from_str(include_str!("fixtures/castalia-custody-vector.json")).unwrap();
        desk.custody_unlock(
            vector["container"].to_string(),
            vector["passphrase"].as_str().unwrap().into(),
        )
        .unwrap();
        desk
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[test]
fn issuer_exports_a_shared_public_profile_from_actual_configuration() {
    use booknook_core::castalia_connections::NativeAuthorityProfile;
    let mut f = Fixture::new();
    let endpoint = "http://127.0.0.1:8779/v1/credentials";
    let output = f.profile(endpoint, "booknook-native", "Personal Castalia authority");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let profile = NativeAuthorityProfile::parse(&output.stdout).unwrap();
    assert_eq!(profile.issuer, f.config["issuer"]);
    assert_eq!(profile.key_id, f.config["key_id"]);
    assert_eq!(profile.issuer_public_key, f.config["public_key"]);
    assert_eq!(profile.audience, f.config["policy"]["audience"]);
    assert_eq!(profile.policy_digest_sha256, f.request["policy_digest"]);
    assert_eq!(profile.caller.id, "booknook-native");
    assert_eq!(profile.credential_endpoint, endpoint);
    let encoded = String::from_utf8(output.stdout).unwrap();
    for private in [
        TOKEN,
        f.config["signing_key_file"].as_str().unwrap(),
        f.request["holder_public_key"].as_str().unwrap(),
        f.config["clients"][0]["token_sha256"].as_str().unwrap(),
        "grants",
        "clients",
        "active",
    ] {
        assert!(
            !encoded.contains(private),
            "profile must contain only public issuer configuration"
        );
    }
    // Export is not permission or availability: inactive/deny-all policy remains
    // exportable for reviewed setup, and subsequent issue still denies the action.
    f.config["active"] = false.into();
    f.config["policy"]["grants"] = json!([]);
    f.save();
    let output = f.profile(endpoint, "booknook-native", "Personal Castalia authority");
    let changed = NativeAuthorityProfile::parse(&output.stdout).unwrap();
    assert_ne!(profile.policy_digest_sha256, changed.policy_digest_sha256);
    assert_eq!(
        changed.policy_digest_sha256,
        digest(&canonical(&f.config["policy"]).unwrap())
    );
    assert_eq!(f.call(TOKEN, &f.request).status.code(), Some(3));
}

#[test]
fn issuer_profile_rejects_unconfigured_callers_unsafe_destinations_and_key_substitution() {
    let f = Fixture::new();
    assert_eq!(
        f.profile(
            "https://issuer.example/v1/credentials",
            "other-app",
            "Issuer"
        )
        .status
        .code(),
        Some(3)
    );
    for endpoint in [
        "http://remote.example/v1/credentials",
        "http://127.1/v1/credentials",
        "https://issuer.example",
        "https://issuer.example/v1/credentials?token=synthetic",
        "https://user@issuer.example/v1/credentials",
        "https://issuer.example:0/v1/credentials",
    ] {
        let output = f.profile(endpoint, "booknook-native", "Issuer");
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
    }
    assert_eq!(
        f.profile(
            "https://issuer.example/v1/credentials",
            "booknook-native",
            "Issuer\u{202e}"
        )
        .status
        .code(),
        Some(2)
    );
    f.write("signer.raw32", &[0x44; 32]);
    assert_eq!(
        f.profile(
            "https://issuer.example/v1/credentials",
            "booknook-native",
            "Issuer"
        )
        .status
        .code(),
        Some(4)
    );
}

#[test]
fn issuer_uses_exact_generic_contract_and_current_configuration() {
    let mut f = Fixture::new();
    let output = f.call(TOKEN, &f.request);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let (request, _) = validate_request(&output.stdout, &f.policy(), &f.payload, 0, now()).unwrap();
    assert_eq!(request.disclosure.title, "Publish selected note");
    assert!(request.credential.claims.expires_at <= now() + 120);
    let again: Value = serde_json::from_slice(&f.call(TOKEN, &f.request).stdout).unwrap();
    assert_ne!(
        again["credential"]["claims"]["nonce"],
        request.credential.claims.nonce
    );
    let description = Command::new(env!("CARGO_BIN_EXE_booknook-issuer"))
        .args([
            "--config",
            f.root.join("issuer.json").to_str().unwrap(),
            "describe",
        ])
        .output()
        .unwrap();
    let description: Value = serde_json::from_slice(&description.stdout).unwrap();
    assert_eq!(description["policy_digest"], f.request["policy_digest"]);
    assert!(!description.to_string().contains(TOKEN));
    assert!(description.get("signing_key_file").is_none());
    f.config["active"] = false.into();
    f.save();
    assert_eq!(f.call(TOKEN, &f.request).status.code(), Some(3));
}
#[test]
fn issuer_denies_forged_context_expired_or_missing_grants_and_disclosures() {
    let mut f = Fixture::new();
    assert_eq!(
        f.call("wrong-token-but-long-enough-synthetic", &f.request)
            .status
            .code(),
        Some(3)
    );
    for (field, value) in [
        ("audience", json!("other")),
        ("policy_digest", json!("ab".repeat(32))),
        ("caller", json!({"kind":"terminal","id":"other"})),
    ] {
        let mut request = f.request.clone();
        request[field] = value;
        assert!(!f.call(TOKEN, &request).status.success(), "{field}");
    }
    let mut request = f.request.clone();
    request["action_request"]["action"] = "sync.send".into();
    assert_eq!(f.call(TOKEN, &request).status.code(), Some(3));
    request = f.request.clone();
    request["disclosure"]["title"] = "Unreviewed title".into();
    assert_eq!(f.call(TOKEN, &request).status.code(), Some(3));
    request = f.request.clone();
    request["disclosure"]["statements"] = json!(["bad\u{202e}display"]);
    assert_eq!(f.call(TOKEN, &request).status.code(), Some(3));
    request = f.request.clone();
    request["disclosure"]["statements"] = json!(vec!["statement"; 9]);
    assert_eq!(f.call(TOKEN, &request).status.code(), Some(3));
    request = f.request.clone();
    request["unsigned_override"] = true.into();
    assert_eq!(f.call(TOKEN, &request).status.code(), Some(2));
    f.config["policy"]["grants"][0]["expires_at"] = (now() - 1).into();
    f.request["policy_digest"] = digest(&canonical(&f.config["policy"]).unwrap()).into();
    f.save();
    assert_eq!(f.call(TOKEN, &f.request).status.code(), Some(3));
    f.config["policy"]["grants"] = json!([]);
    f.request["policy_digest"] = digest(&canonical(&f.config["policy"]).unwrap()).into();
    f.save();
    assert_eq!(f.call(TOKEN, &f.request).status.code(), Some(3));
}
#[test]
fn issuer_refuses_unsafe_or_substituted_signing_material() {
    let f = Fixture::new();
    fs::set_permissions(
        f.root.join("signer.raw32"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert_eq!(f.call(TOKEN, &f.request).status.code(), Some(4));
    f.write("other.raw32", &[0x43; 32]);
    fs::remove_file(f.root.join("signer.raw32")).unwrap();
    symlink(f.root.join("other.raw32"), f.root.join("signer.raw32")).unwrap();
    assert_eq!(f.call(TOKEN, &f.request).status.code(), Some(4));
    fs::remove_file(f.root.join("signer.raw32")).unwrap();
    fs::hard_link(f.root.join("other.raw32"), f.root.join("signer.raw32")).unwrap();
    assert_eq!(f.call(TOKEN, &f.request).status.code(), Some(4));
    fs::remove_file(f.root.join("signer.raw32")).unwrap();
    f.write("signer.raw32", &[0x44; 32]);
    assert_eq!(f.call(TOKEN, &f.request).status.code(), Some(4));
    f.write("signer.raw32", &[0x43; 32]);
    fs::set_permissions(&f.root, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(f.call(TOKEN, &f.request).status.code(), Some(4));
}
struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn post(port: u16, token: &str, body: &[u8], extra: &str) -> (u16, Vec<u8>) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    write!(stream,"POST /v1/credentials HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra}\r\n",body.len()).unwrap();
    stream.write_all(body).unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).unwrap();
    let boundary = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let status = std::str::from_utf8(&raw[..boundary])
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    (status, raw[boundary + 4..].to_vec())
}
#[test]
#[ignore = "Starts an actual local HTTP issuer; run scripts/test-issuer.sh in a loopback-enabled environment"]
fn actual_http_issuer_roundtrips_native_consent_and_receiver() {
    let mut f = Fixture::new();
    let script = PathBuf::from(std::env::var_os("BOOKNOOK_ISSUER_HTTP_SERVICE")
        .expect("Set BOOKNOOK_ISSUER_HTTP_SERVICE to the reference HTTP script; scripts/test-issuer.sh does this"));
    let mut server = Server(
        Command::new("python3")
            .args([
                script.to_str().unwrap(),
                "--issuer-bin",
                env!("CARGO_BIN_EXE_booknook-issuer"),
                "--config",
                f.root.join("issuer.json").to_str().unwrap(),
                "--port",
                "0",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut line = String::new();
    BufReader::new(server.0.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let listener: Value = serde_json::from_str(&line).unwrap();
    let port = listener["port"].as_u64().unwrap() as u16;
    let endpoint = format!("http://127.0.0.1:{port}/v1/credentials");
    let desk = f.desk(&endpoint);
    let action = desk
        .authorized_action(
            String::from_utf8(f.payload.clone()).unwrap(),
            "social.publish".into(),
            0,
            "synthetic-request".into(),
        )
        .unwrap();
    f.request["action_request"] = serde_json::from_str(&action).unwrap();
    let body = canonical(&f.request).unwrap();
    let (status, bytes) = post(port, TOKEN, &body, "");
    assert_eq!(status, 200);
    let request = String::from_utf8(bytes.clone()).unwrap();
    let consent: Value = serde_json::from_str(
        &desk
            .custody_prepare(
                request.clone(),
                String::from_utf8(f.payload.clone()).unwrap(),
                "social.publish".into(),
                0,
            )
            .unwrap(),
    )
    .unwrap();
    let token = consent["token"].as_str().unwrap().to_string();
    let presentation = desk.custody_approve(token.clone()).unwrap();
    assert!(desk.custody_approve(token).is_err());
    verify_receipt(
        &bytes,
        presentation.as_bytes(),
        &f.policy(),
        &f.payload,
        0,
        now(),
    )
    .unwrap();
    assert!(
        verify_receipt(
            &bytes,
            presentation.as_bytes(),
            &f.policy(),
            b"different payload",
            0,
            now()
        )
        .is_err()
    );
    assert_eq!(
        post(port, TOKEN, &body, "Origin: http://127.0.0.1\r\n").0,
        403
    );
    assert_eq!(post(port, "invalid-synthetic-token", &body, "").0, 403);
    let duplicate = br#"{"schema":"a","schema":"b"}"#;
    assert_eq!(post(port, TOKEN, duplicate, "").0, 400);
    f.config["active"] = false.into();
    f.save();
    assert_eq!(post(port, TOKEN, &body, "").0, 403);
    drop(desk);
}
