// SPDX-License-Identifier: MIT OR Apache-2.0
// Synthetic issuer/holder credentials only; never used by a runtime provider.
use base64::{Engine, engine::general_purpose::STANDARD};
use booknook_core::{authority::*, *};
use ed25519_dalek::{Signer, SigningKey};

fn signed(key: &SigningKey, domain: &str, value: &impl serde::Serialize) -> String {
    let mut data = domain.as_bytes().to_vec();
    data.extend(canonical(value).unwrap());
    hex::encode(key.sign(&data).to_bytes())
}
pub fn receipt(
    payload: &[u8],
    action_name: &str,
    version: u64,
) -> (Vec<u8>, Vec<u8>, AuthorityPolicy) {
    let issuer = SigningKey::from_bytes(&[1; 32]);
    let holder = SigningKey::from_bytes(&[2; 32]);
    let actor = hex::encode(holder.verifying_key().as_bytes());
    let caller = Caller {
        kind: "terminal".into(),
        id: "booknook-test".into(),
    };
    let policy = AuthorityPolicy {
        pins: vec![TrustPin {
            issuer: "test-issuer".into(),
            key_id: "test-key".into(),
            public_key: hex::encode(issuer.verifying_key().as_bytes()),
            audience: "booknook-test".into(),
            callers: vec![caller.clone()],
        }],
        expected_caller: caller.clone(),
        audience: "booknook-test".into(),
        actor: actor.clone(),
        action: action_name.into(),
        policy_digest: digest(b"policy"),
    };
    let action = ActionRequest {
        schema: "booknook.authorized-action.v1".into(),
        action: action_name.into(),
        actor: actor.clone(),
        payload_digest: digest(payload),
        resource_version: version,
        idempotency_key: "test-once".into(),
    };
    let bytes = canonical(&action).unwrap();
    let disclosure = Disclosure {
        title: "Publish selected content".into(),
        statements: vec!["Only the reviewed selection will leave this device.".into()],
    };
    let claims = CredentialClaims {
        schema: "castalia.request-credential.v1".into(),
        issuer: "test-issuer".into(),
        key_id: "test-key".into(),
        holder_public_key: actor.clone(),
        audience: "booknook-test".into(),
        caller: caller.clone(),
        request_digest_sha256: digest(&bytes),
        disclosure_digest_sha256: digest(&canonical(&disclosure).unwrap()),
        policy_digest_sha256: policy.policy_digest.clone(),
        nonce: "11".repeat(16),
        issued_at: 100,
        expires_at: 220,
    };
    let credential = Credential {
        signature: signed(
            &issuer,
            "castalia.request-credential.v1/signature\0",
            &claims,
        ),
        claims,
    };
    let unsigned = UnsignedPresentation {
        schema: "castalia.credential-presentation.v2".into(),
        holder_public_key: actor,
        issuer: "test-issuer".into(),
        key_id: "test-key".into(),
        audience: "booknook-test".into(),
        caller,
        request_digest_sha256: digest(&bytes),
        credential_digest_sha256: digest(&canonical(&credential).unwrap()),
        disclosure_digest_sha256: digest(&canonical(&disclosure).unwrap()),
        nonce: "22".repeat(16),
        issued_at: 101,
        expires_at: 161,
    };
    let signature = signed(
        &holder,
        "castalia.credential-presentation.v2/signature\0",
        &unsigned,
    );
    let mut presentation = serde_json::to_value(unsigned).unwrap();
    presentation["signature"] = signature.into();
    let request = PresentationRequest {
        schema: "castalia.credential-presentation-request.v2".into(),
        request_bytes_base64: STANDARD.encode(bytes),
        disclosure,
        credential,
    };
    (
        canonical(&request).unwrap(),
        canonical(&presentation).unwrap(),
        policy,
    )
}
