// SPDX-License-Identifier: MIT OR Apache-2.0
//! Native Castalia custody using the published v1 recovery envelope and v2 presentation contract.
//! Cryptographic operations are delegated to RustCrypto and ed25519-dalek. The recovery
//! parameters and compatibility vector come from castalia-wallet-core (MIT), snapshot ff8de6d.
//! No raw signing API is exposed; each presentation requires a validated single-use preparation.
use aes_gcm::{
    Aes256Gcm, KeyInit,
    aead::{Aead, Payload},
};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use booknook_core::{Error, Result, authority::*, canonical, digest, strict_parse};
use ed25519_dalek::{Signer, SigningKey, pkcs8::DecodePrivateKey};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use unicode_normalization::UnicodeNormalization;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Kdf {
    name: String,
    version: u32,
    salt: String,
    #[serde(rename = "memoryKiB")]
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
    derived_key_bytes: u32,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct AeadHeader {
    name: String,
    nonce: String,
    tag_bits: u32,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Recovery {
    schema: String,
    key_suite: String,
    public_key: String,
    created_at: u64,
    kdf: Kdf,
    aead: AeadHeader,
    ciphertext: String,
}
#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Secret {
    schema: String,
    key_id: String,
    #[serde(deserialize_with = "secret_string")]
    private_key_pkcs8: Zeroizing<String>,
    created_at: u64,
}

struct Pending {
    request: Vec<u8>,
    payload: Vec<u8>,
    policy: AuthorityPolicy,
    version: u64,
    prepared_at: u64,
    expires_at: u64,
    epoch: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedConsent {
    pub token: String,
    pub title: String,
    pub statements: Vec<String>,
    pub expires_at: u64,
    pub holder_public_key: String,
    pub action: String,
    pub payload_digest: String,
    pub resource_version: u64,
}
struct GenericPending {
    request: Vec<u8>,
    expected_request: Vec<u8>,
    expected_disclosure: Disclosure,
    policy: CastaliaTrustPolicy,
    prepared_at: u64,
    expires_at: u64,
    epoch: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CastaliaPreparedConsent {
    pub token: String,
    pub title: String,
    pub statements: Vec<String>,
    pub expires_at: u64,
    pub holder_public_key: String,
    pub request_digest_sha256: String,
    pub audience: String,
    pub caller: Caller,
}
#[derive(Default)]
pub struct Custody {
    key: Option<SigningKey>,
    pending: BTreeMap<String, Pending>,
    generic_pending: BTreeMap<String, GenericPending>,
    epoch: u64,
}
fn invalid() -> Error {
    Error::Denied("Invalid Castalia recovery material or passphrase".into())
}
fn decode(s: &str) -> Result<Vec<u8>> {
    let bytes = URL_SAFE_NO_PAD.decode(s).map_err(|_| invalid())?;
    if URL_SAFE_NO_PAD.encode(&bytes) != s {
        return Err(invalid());
    }
    Ok(bytes)
}
fn secret_string<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Zeroizing<String>, D::Error> {
    String::deserialize(deserializer).map(Zeroizing::new)
}
fn decode_private(s: &str) -> Result<Zeroizing<Vec<u8>>> {
    let bytes = Zeroizing::new(URL_SAFE_NO_PAD.decode(s).map_err(|_| invalid())?);
    let encoded = Zeroizing::new(URL_SAFE_NO_PAD.encode(bytes.as_slice()));
    if encoded.as_str() != s {
        return Err(invalid());
    }
    Ok(bytes)
}
fn random_token() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|_| Error::Unavailable("Secure randomness unavailable".into()))?;
    Ok(hex::encode(bytes))
}
impl Custody {
    pub fn lock(&mut self) {
        self.key = None;
        self.pending.clear();
        self.generic_pending.clear();
        self.epoch = self.epoch.wrapping_add(1);
    }
    pub fn public_key(&self) -> Option<String> {
        self.key
            .as_ref()
            .map(|k| hex::encode(k.verifying_key().as_bytes()))
    }
    /// Every unlock attempt invalidates outstanding approvals, including a failed attempt.
    pub fn unlock(&mut self, recovery: &[u8], passphrase: String) -> Result<String> {
        self.lock();
        let passphrase = Zeroizing::new(passphrase);
        if recovery.is_empty() || recovery.len() > 1_048_576 || passphrase.len() > 16_384 {
            return Err(invalid());
        }
        let r: Recovery = strict_parse(recovery).map_err(|_| invalid())?;
        if r.schema != "castalia.wallet-custody.v1"
            || r.key_suite != "Ed25519"
            || r.kdf.name != "Argon2id"
            || r.kdf.version != 19
            || r.kdf.memory_kib != 65_536
            || r.kdf.iterations != 3
            || r.kdf.parallelism != 1
            || r.kdf.derived_key_bytes != 32
            || r.aead.name != "AES-256-GCM"
            || r.aead.tag_bits != 128
            || r.created_at > 9_007_199_254_740_991
        {
            return Err(invalid());
        }
        let salt: [u8; 16] = decode(&r.kdf.salt)?.try_into().map_err(|_| invalid())?;
        let nonce: [u8; 12] = decode(&r.aead.nonce)?.try_into().map_err(|_| invalid())?;
        let mut header = serde_json::to_value(&r)?;
        header.as_object_mut().unwrap().remove("ciphertext");
        let aad = canonical(&header)?;
        let normalized = Zeroizing::new(passphrase.nfc().collect::<String>());
        if normalized.is_empty() {
            return Err(invalid());
        }
        let mut derived = Zeroizing::new([0u8; 32]);
        let params = Params::new(65_536, 3, 1, Some(32)).map_err(|_| invalid())?;
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password_into(normalized.as_bytes(), &salt, derived.as_mut())
            .map_err(|_| invalid())?;
        let cipher = Aes256Gcm::new_from_slice(derived.as_ref()).map_err(|_| invalid())?;
        let plaintext = Zeroizing::new(
            cipher
                .decrypt(
                    (&nonce).into(),
                    Payload {
                        msg: &decode(&r.ciphertext)?,
                        aad: &aad,
                    },
                )
                .map_err(|_| invalid())?,
        );
        // Typed serde rejects duplicate/unknown fields without an intermediate JSON Value
        // retaining an unzeroized copy of the private PKCS#8 string.
        let secret: Secret = serde_json::from_slice(&plaintext).map_err(|_| invalid())?;
        if secret.schema != "castalia.wallet-secret.v1" || secret.created_at != r.created_at {
            return Err(invalid());
        }
        let pkcs8 = decode_private(&secret.private_key_pkcs8)?;
        let key = SigningKey::from_pkcs8_der(&pkcs8).map_err(|_| invalid())?;
        let public = hex::encode(key.verifying_key().as_bytes());
        if public != r.public_key
            || blake3::hash(key.verifying_key().as_bytes())
                .to_hex()
                .as_str()
                != secret.key_id
        {
            return Err(invalid());
        }
        self.key = Some(key);
        Ok(public)
    }
    pub fn prepare(
        &mut self,
        request: &[u8],
        payload: &[u8],
        policy: AuthorityPolicy,
        version: u64,
        now: u64,
    ) -> Result<PreparedConsent> {
        if self.public_key().as_deref() != Some(policy.actor.as_str()) {
            return Err(Error::Denied(
                "Unlock the configured Castalia identity first".into(),
            ));
        }
        let (validated, action) = validate_request(request, &policy, payload, version, now)?;
        self.pending.retain(|_, p| now < p.expires_at);
        self.generic_pending.retain(|_, p| now < p.expires_at);
        if self.pending.len() + self.generic_pending.len() >= 16 {
            return Err(Error::Denied("Too many outstanding approvals".into()));
        }
        let expires_at = validated
            .credential
            .claims
            .expires_at
            .min(now.saturating_add(120));
        let token = random_token()?;
        let summary = PreparedConsent {
            token: token.clone(),
            title: validated.disclosure.title,
            statements: validated.disclosure.statements,
            expires_at,
            holder_public_key: policy.actor.clone(),
            action: action.action,
            payload_digest: action.payload_digest,
            resource_version: version,
        };
        self.pending.insert(
            token,
            Pending {
                request: request.to_vec(),
                payload: payload.to_vec(),
                policy,
                version,
                prepared_at: now,
                expires_at,
                epoch: self.epoch,
            },
        );
        Ok(summary)
    }
    /// Caller must obtain fresh consent for the returned preparation. Consumes token even on error.
    /// Policy is independently reloaded at approval so changed trust settings invalidate preparation.
    pub fn approve(
        &mut self,
        token: &str,
        current_policy: &AuthorityPolicy,
        now: u64,
    ) -> Result<String> {
        let p = self
            .pending
            .remove(token)
            .ok_or_else(|| Error::Denied("Approval expired, cancelled, or already used".into()))?;
        if now < p.prepared_at
            || now >= p.expires_at
            || p.epoch != self.epoch
            || !same_policy(&p.policy, current_policy)
        {
            return Err(Error::Denied("Approval context changed or expired".into()));
        }
        let (r, _) = validate_request(&p.request, current_policy, &p.payload, p.version, now)?;
        let bytes = self.sign_presentation(&r, now)?;
        verify_receipt(
            &p.request,
            &bytes,
            current_policy,
            &p.payload,
            p.version,
            now,
        )?;
        String::from_utf8(bytes).map_err(|_| Error::Invalid("Invalid encoding".into()))
    }
    fn sign_presentation(&self, r: &PresentationRequest, now: u64) -> Result<Vec<u8>> {
        let key = self
            .key
            .as_ref()
            .ok_or_else(|| Error::Denied("Wallet is locked".into()))?;
        let c = &r.credential.claims;
        let u = UnsignedPresentation {
            schema: "castalia.credential-presentation.v2".into(),
            holder_public_key: c.holder_public_key.clone(),
            issuer: c.issuer.clone(),
            key_id: c.key_id.clone(),
            audience: c.audience.clone(),
            caller: c.caller.clone(),
            request_digest_sha256: c.request_digest_sha256.clone(),
            credential_digest_sha256: digest(&canonical(&r.credential)?),
            disclosure_digest_sha256: c.disclosure_digest_sha256.clone(),
            nonce: random_token()?,
            issued_at: now,
            expires_at: c.expires_at.min(now.saturating_add(60)),
        };
        let mut transcript = b"castalia.credential-presentation.v2/signature\0".to_vec();
        transcript.extend(canonical(&u)?);
        let signature = key.sign(&transcript);
        let mut result = serde_json::to_value(u)?;
        result
            .as_object_mut()
            .unwrap()
            .insert("signature".into(), hex::encode(signature.to_bytes()).into());
        canonical(&result)
    }
    pub fn prepare_castalia(
        &mut self,
        request: &[u8],
        expected_request: &[u8],
        expected_disclosure: Disclosure,
        policy: CastaliaTrustPolicy,
        now: u64,
    ) -> Result<CastaliaPreparedConsent> {
        if self.public_key().as_deref() != Some(policy.holder_public_key.as_str()) {
            return Err(Error::Denied(
                "Unlock the configured Castalia identity first".into(),
            ));
        }
        let validated = validate_castalia_request(
            request,
            &policy,
            expected_request,
            &expected_disclosure,
            now,
        )?;
        self.pending.retain(|_, p| now < p.expires_at);
        self.generic_pending.retain(|_, p| now < p.expires_at);
        if self.pending.len() + self.generic_pending.len() >= 16 {
            return Err(Error::Denied("Too many outstanding approvals".into()));
        }
        let expires_at = validated
            .credential
            .claims
            .expires_at
            .min(now.saturating_add(120));
        let token = random_token()?;
        let summary = CastaliaPreparedConsent {
            token: token.clone(),
            title: validated.disclosure.title,
            statements: validated.disclosure.statements,
            expires_at,
            holder_public_key: policy.holder_public_key.clone(),
            request_digest_sha256: validated.credential.claims.request_digest_sha256,
            audience: policy.audience.clone(),
            caller: policy.expected_caller.clone(),
        };
        self.generic_pending.insert(
            token,
            GenericPending {
                request: request.to_vec(),
                expected_request: expected_request.to_vec(),
                expected_disclosure,
                policy,
                prepared_at: now,
                expires_at,
                epoch: self.epoch,
            },
        );
        Ok(summary)
    }
    /// A host supplies current independent trust again after obtaining explicit consent.
    /// Every attempt consumes the preparation, including revoked/expired contexts.
    pub fn approve_castalia(
        &mut self,
        token: &str,
        current_policy: &CastaliaTrustPolicy,
        now: u64,
    ) -> Result<String> {
        let p = self
            .generic_pending
            .remove(token)
            .ok_or_else(|| Error::Denied("Approval expired, cancelled, or already used".into()))?;
        if now < p.prepared_at
            || now >= p.expires_at
            || p.epoch != self.epoch
            || p.policy != *current_policy
        {
            return Err(Error::Denied("Approval context changed or expired".into()));
        }
        let request = validate_castalia_request(
            &p.request,
            current_policy,
            &p.expected_request,
            &p.expected_disclosure,
            now,
        )?;
        let bytes = self.sign_presentation(&request, now)?;
        verify_castalia_receipt(
            &p.request,
            &bytes,
            current_policy,
            &p.expected_request,
            &p.expected_disclosure,
            now,
        )?;
        String::from_utf8(bytes).map_err(|_| Error::Invalid("Invalid encoding".into()))
    }
    pub fn cancel(&mut self, token: &str) {
        self.pending.remove(token);
        self.generic_pending.remove(token);
    }
    pub fn pending_action(&self, token: &str) -> Option<&str> {
        self.pending.get(token).map(|p| p.policy.action.as_str())
    }
}
fn same_policy(a: &AuthorityPolicy, b: &AuthorityPolicy) -> bool {
    a.actor == b.actor
        && a.action == b.action
        && a.policy_digest == b.policy_digest
        && a.audience == b.audience
        && a.expected_caller == b.expected_caller
        && a.pins.len() == b.pins.len()
        && a.pins.iter().zip(&b.pins).all(|(a, b)| {
            a.issuer == b.issuer
                && a.key_id == b.key_id
                && a.public_key == b.public_key
                && a.audience == b.audience
                && a.callers == b.callers
        })
}

#[cfg(test)]
#[path = "../tests/fixtures/generic_castalia.rs"]
mod generic_fixture;

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    const VECTOR: &str = include_str!("../tests/fixtures/castalia-custody-vector.json");
    fn vector() -> (Vec<u8>, String, String) {
        let v: serde_json::Value = serde_json::from_str(VECTOR).unwrap();
        (
            serde_json::to_vec(&v["container"]).unwrap(),
            v["passphrase"].as_str().unwrap().into(),
            v["public_key"].as_str().unwrap().into(),
        )
    }
    fn request(holder: &str) -> (Vec<u8>, AuthorityPolicy) {
        let issuer = SigningKey::from_bytes(&[1; 32]);
        let caller = Caller {
            kind: "terminal".into(),
            id: "booknook-native".into(),
        };
        let action = ActionRequest {
            schema: "booknook.authorized-action.v1".into(),
            action: "sync.send".into(),
            actor: holder.into(),
            payload_digest: digest(b"batch"),
            resource_version: 7,
            idempotency_key: "specific-batch".into(),
        };
        let bytes = canonical(&action).unwrap();
        let disclosure = Disclosure {
            title: "Sync one notebook".into(),
            statements: vec!["Send encrypted data to the paired device".into()],
        };
        let claims = CredentialClaims {
            schema: "castalia.request-credential.v1".into(),
            issuer: "test-issuer".into(),
            key_id: "test".into(),
            holder_public_key: holder.into(),
            audience: "booknook".into(),
            caller: caller.clone(),
            request_digest_sha256: digest(&bytes),
            disclosure_digest_sha256: digest(&canonical(&disclosure).unwrap()),
            policy_digest_sha256: "aa".repeat(32),
            nonce: "bb".repeat(16),
            issued_at: 100,
            expires_at: 220,
        };
        let mut transcript = b"castalia.request-credential.v1/signature\0".to_vec();
        transcript.extend(canonical(&claims).unwrap());
        let credential = Credential {
            claims,
            signature: hex::encode(issuer.sign(&transcript).to_bytes()),
        };
        let r = PresentationRequest {
            schema: "castalia.credential-presentation-request.v2".into(),
            request_bytes_base64: STANDARD.encode(bytes),
            disclosure,
            credential,
        };
        let policy = AuthorityPolicy {
            pins: vec![TrustPin {
                issuer: "test-issuer".into(),
                key_id: "test".into(),
                public_key: hex::encode(issuer.verifying_key().as_bytes()),
                audience: "booknook".into(),
                callers: vec![caller.clone()],
            }],
            expected_caller: caller,
            audience: "booknook".into(),
            actor: holder.into(),
            action: "sync.send".into(),
            policy_digest: "aa".repeat(32),
        };
        (canonical(&r).unwrap(), policy)
    }
    #[test]
    fn exact_upstream_recovery_vector_and_fresh_approval_are_interoperable() {
        let (raw, pass, public) = vector();
        let mut c = Custody::default();
        assert_eq!(c.unlock(&raw, pass).unwrap(), public);
        let (request, policy) = request(&public);
        let prepared = c
            .prepare(&request, b"batch", policy.clone(), 7, 101)
            .unwrap();
        let receipt = c.approve(&prepared.token, &policy, 102).unwrap();
        verify_receipt(&request, receipt.as_bytes(), &policy, b"batch", 7, 102).unwrap();
        assert!(c.approve(&prepared.token, &policy, 102).is_err());
        let p = c
            .prepare(&request, b"batch", policy.clone(), 7, 103)
            .unwrap();
        c.lock();
        assert!(c.approve(&p.token, &policy, 104).is_err());
    }
    #[test]
    fn wrong_passphrase_tampering_and_context_changes_fail_closed() {
        let (raw, pass, public) = vector();
        let mut c = Custody::default();
        assert!(c.unlock(&raw, "wrong".into()).is_err());
        let mut r: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        r["kdf"]["iterations"] = 1.into();
        assert!(
            c.unlock(&serde_json::to_vec(&r).unwrap(), pass.clone())
                .is_err()
        );
        c.unlock(&raw, pass).unwrap();
        let (request, policy) = request(&public);
        assert!(
            c.prepare(&request, b"changed", policy.clone(), 7, 101)
                .is_err()
        );
        let p = c
            .prepare(&request, b"batch", policy.clone(), 7, 101)
            .unwrap();
        let mut revoked = policy.clone();
        revoked.pins.clear();
        assert!(c.approve(&p.token, &revoked, 102).is_err());
        let p = c
            .prepare(&request, b"batch", policy.clone(), 7, 101)
            .unwrap();
        assert!(c.approve(&p.token, &policy, 220).is_err());
    }
    #[test]
    fn generic_custody_binds_current_trust_disclosure_expiry_and_identity() {
        let (raw, pass, holder) = vector();
        let mut custody = Custody::default();
        custody.unlock(&raw, pass).unwrap();
        let (request, expected, disclosure, policy) = generic_fixture::fixture(&holder, 100);
        let mut changed = policy.clone();
        changed.expected_caller.id = "different-host".into();
        assert!(
            custody
                .prepare_castalia(&request, &expected, disclosure.clone(), changed, 101)
                .is_err()
        );
        let mut changed = policy.clone();
        changed.holder_public_key = "00".repeat(32);
        assert!(
            custody
                .prepare_castalia(&request, &expected, disclosure.clone(), changed, 101)
                .is_err()
        );
        assert!(
            custody
                .prepare_castalia(&request, b"{}", disclosure.clone(), policy.clone(), 101)
                .is_err()
        );
        let mut altered = disclosure.clone();
        altered.statements.push("Unapproved disclosure".into());
        assert!(
            custody
                .prepare_castalia(&request, &expected, altered, policy.clone(), 101)
                .is_err()
        );
        let prepared = custody
            .prepare_castalia(&request, &expected, disclosure.clone(), policy.clone(), 101)
            .unwrap();
        let mut changed = policy.clone();
        changed.pins.clear();
        assert!(
            custody
                .approve_castalia(&prepared.token, &changed, 102)
                .is_err()
        );
        assert!(
            custody
                .approve_castalia(&prepared.token, &policy, 102)
                .is_err()
        );
        let expired = custody
            .prepare_castalia(&request, &expected, disclosure.clone(), policy.clone(), 101)
            .unwrap();
        assert!(
            custody
                .approve_castalia(&expired.token, &policy, 220)
                .is_err()
        );
        let fresh = custody
            .prepare_castalia(&request, &expected, disclosure.clone(), policy.clone(), 101)
            .unwrap();
        let receipt = custody
            .approve_castalia(&fresh.token, &policy, 102)
            .unwrap();
        verify_castalia_receipt(
            &request,
            receipt.as_bytes(),
            &policy,
            &expected,
            &disclosure,
            102,
        )
        .unwrap();
        assert!(
            verify_castalia_receipt(
                &request,
                receipt.as_bytes(),
                &policy,
                &expected,
                &disclosure,
                162
            )
            .is_err()
        );
        for _ in 0..16 {
            custody
                .prepare_castalia(&request, &expected, disclosure.clone(), policy.clone(), 103)
                .unwrap();
        }
        assert!(
            custody
                .prepare_castalia(&request, &expected, disclosure, policy.clone(), 103)
                .is_err()
        );
        assert!(custody.unlock(&raw, "not-the-passphrase".into()).is_err());
        assert!(custody.public_key().is_none());
        assert!(custody.generic_pending.is_empty());
    }
}
