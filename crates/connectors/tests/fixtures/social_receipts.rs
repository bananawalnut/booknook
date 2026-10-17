// SPDX-License-Identifier: MIT OR Apache-2.0
// Synthetic credentials for conformance tests only.
use base64::{Engine, engine::general_purpose::STANDARD};
use booknook_core::{
    authority::*,
    canonical, digest,
    social_protocol::{ACTION, SocialEvent},
};
use ed25519_dalek::{Signer, SigningKey};
pub fn actor(seed: u8) -> String {
    hex::encode(
        SigningKey::from_bytes(&[seed; 32])
            .verifying_key()
            .as_bytes(),
    )
}
fn sign(key: &SigningKey, domain: &str, value: &impl serde::Serialize) -> String {
    let mut bytes = domain.as_bytes().to_vec();
    bytes.extend(canonical(value).unwrap());
    hex::encode(key.sign(&bytes).to_bytes())
}
pub fn receipt(
    payload: &[u8],
    action_name: &str,
    version: u64,
    at: u64,
    seed: u8,
) -> (String, String, AuthorityPolicy) {
    let issuer = SigningKey::from_bytes(&[1; 32]);
    let holder = SigningKey::from_bytes(&[seed; 32]);
    let caller = Caller {
        kind: "terminal".into(),
        id: "synthetic-booknook-test".into(),
    };
    let policy = AuthorityPolicy {
        pins: vec![TrustPin {
            issuer: "synthetic-issuer".into(),
            key_id: "test".into(),
            public_key: hex::encode(issuer.verifying_key().as_bytes()),
            audience: "booknook-test".into(),
            callers: vec![caller.clone()],
        }],
        expected_caller: caller.clone(),
        audience: "booknook-test".into(),
        actor: actor(seed),
        action: action_name.into(),
        policy_digest: digest(b"synthetic policy"),
    };
    let action = ActionRequest {
        schema: "booknook.authorized-action.v1".into(),
        action: action_name.into(),
        actor: actor(seed),
        payload_digest: digest(payload),
        resource_version: version,
        idempotency_key: digest(payload),
    };
    let bytes = canonical(&action).unwrap();
    let disclosure = Disclosure {
        title: "Publish reviewed selection".into(),
        statements: vec!["Synthetic test data only".into()],
    };
    let nonce = digest(payload)[..32].to_string();
    let claims = CredentialClaims {
        schema: "castalia.request-credential.v1".into(),
        issuer: "synthetic-issuer".into(),
        key_id: "test".into(),
        holder_public_key: actor(seed),
        audience: "booknook-test".into(),
        caller: caller.clone(),
        request_digest_sha256: digest(&bytes),
        disclosure_digest_sha256: digest(&canonical(&disclosure).unwrap()),
        policy_digest_sha256: policy.policy_digest.clone(),
        nonce: nonce.clone(),
        issued_at: at - 1,
        expires_at: at + 119,
    };
    let credential = Credential {
        signature: sign(
            &issuer,
            "castalia.request-credential.v1/signature\0",
            &claims,
        ),
        claims,
    };
    let unsigned = UnsignedPresentation {
        schema: "castalia.credential-presentation.v2".into(),
        holder_public_key: actor(seed),
        issuer: "synthetic-issuer".into(),
        key_id: "test".into(),
        audience: "booknook-test".into(),
        caller,
        request_digest_sha256: digest(&bytes),
        credential_digest_sha256: digest(&canonical(&credential).unwrap()),
        disclosure_digest_sha256: digest(&canonical(&disclosure).unwrap()),
        nonce,
        issued_at: at,
        expires_at: at + 60,
    };
    let signature = sign(
        &holder,
        "castalia.credential-presentation.v2/signature\0",
        &unsigned,
    );
    let mut p = serde_json::to_value(unsigned).unwrap();
    p["signature"] = signature.into();
    let r = PresentationRequest {
        schema: "castalia.credential-presentation-request.v2".into(),
        request_bytes_base64: STANDARD.encode(bytes),
        disclosure,
        credential,
    };
    (
        String::from_utf8(canonical(&r).unwrap()).unwrap(),
        String::from_utf8(canonical(&p).unwrap()).unwrap(),
        policy,
    )
}

pub fn proof(e: &SocialEvent, seed: u8) -> (String, String, AuthorityPolicy) {
    receipt(
        &canonical(e).unwrap(),
        ACTION,
        e.resource_version,
        e.at,
        seed,
    )
}
