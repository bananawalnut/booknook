// SPDX-License-Identifier: MIT OR Apache-2.0
// Public synthetic issuer only. Never accepts or reads a production private key.
use base64::{Engine, engine::general_purpose::STANDARD};
use booknook_core::{authority::*, canonical, digest};
use ed25519_dalek::{Signer, SigningKey};
pub fn fixture(holder: &str, now: u64) -> (Vec<u8>, Vec<u8>, Disclosure, CastaliaTrustPolicy) {
    let issuer = SigningKey::from_bytes(&[29; 32]);
    let caller = Caller {
        kind: "terminal".into(),
        id: "second-castalia-app".into(),
    };
    let expected = canonical(&serde_json::json!({"schema":"second-castalia-app.operation.v1","operation":"review-document","document":"synthetic-document","revision":3})).unwrap();
    let disclosure = Disclosure {
        title: "Review one synthetic document".into(),
        statements: vec!["Approve only this document revision in the second Castalia app.".into()],
    };
    let claims = CredentialClaims {
        schema: "castalia.request-credential.v1".into(),
        issuer: "synthetic-castalia-issuer".into(),
        key_id: "synthetic-29".into(),
        holder_public_key: holder.into(),
        audience: "second-castalia-app".into(),
        caller: caller.clone(),
        request_digest_sha256: digest(&expected),
        disclosure_digest_sha256: digest(&canonical(&disclosure).unwrap()),
        policy_digest_sha256: "cd".repeat(32),
        nonce: "ee".repeat(16),
        issued_at: now,
        expires_at: now + 120,
    };
    let mut transcript = b"castalia.request-credential.v1/signature\0".to_vec();
    transcript.extend(canonical(&claims).unwrap());
    let request = PresentationRequest {
        schema: "castalia.credential-presentation-request.v2".into(),
        request_bytes_base64: STANDARD.encode(&expected),
        disclosure: disclosure.clone(),
        credential: Credential {
            claims,
            signature: hex::encode(issuer.sign(&transcript).to_bytes()),
        },
    };
    let policy = CastaliaTrustPolicy {
        pins: vec![TrustPin {
            issuer: "synthetic-castalia-issuer".into(),
            key_id: "synthetic-29".into(),
            public_key: hex::encode(issuer.verifying_key().as_bytes()),
            audience: "second-castalia-app".into(),
            callers: vec![caller.clone()],
        }],
        expected_caller: caller,
        audience: "second-castalia-app".into(),
        holder_public_key: holder.into(),
        policy_digest: "cd".repeat(32),
    };
    (canonical(&request).unwrap(), expected, disclosure, policy)
}
