//! Castalia generic presentation v2 verifier; signing is delegated to approved custody.
//! Rolodex attribution and Nostr signatures never enter this authorization decision.
use crate::{Error, Result, canonical, digest, strict_parse};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Caller {
    pub kind: String,
    pub id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Disclosure {
    pub title: String,
    pub statements: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialClaims {
    pub schema: String,
    pub issuer: String,
    pub key_id: String,
    pub holder_public_key: String,
    pub audience: String,
    pub caller: Caller,
    pub request_digest_sha256: String,
    pub disclosure_digest_sha256: String,
    pub policy_digest_sha256: String,
    pub nonce: String,
    pub issued_at: u64,
    pub expires_at: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credential {
    pub claims: CredentialClaims,
    pub signature: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresentationRequest {
    pub schema: String,
    pub request_bytes_base64: String,
    pub disclosure: Disclosure,
    pub credential: Credential,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnsignedPresentation {
    pub schema: String,
    pub holder_public_key: String,
    pub issuer: String,
    pub key_id: String,
    pub audience: String,
    pub caller: Caller,
    pub request_digest_sha256: String,
    pub credential_digest_sha256: String,
    pub disclosure_digest_sha256: String,
    pub nonce: String,
    pub issued_at: u64,
    pub expires_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TrustPin {
    pub issuer: String,
    pub key_id: String,
    pub public_key: String,
    pub audience: String,
    pub callers: Vec<Caller>,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuthorityPolicy {
    pub pins: Vec<TrustPin>,
    pub expected_caller: Caller,
    pub audience: String,
    pub actor: String,
    pub action: String,
    pub policy_digest: String,
}

/// Independently configured Castalia trust, without an application action schema.
/// Importing a credential or provider descriptor must never populate these pins.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CastaliaTrustPolicy {
    pub pins: Vec<TrustPin>,
    pub expected_caller: Caller,
    pub audience: String,
    pub holder_public_key: String,
    pub policy_digest: String,
}
impl From<&AuthorityPolicy> for CastaliaTrustPolicy {
    fn from(policy: &AuthorityPolicy) -> Self {
        Self {
            pins: policy.pins.clone(),
            expected_caller: policy.expected_caller.clone(),
            audience: policy.audience.clone(),
            holder_public_key: policy.actor.clone(),
            policy_digest: policy.policy_digest.clone(),
        }
    }
}
/// Proof of a verified Castalia presentation. It does not authorize an application's
/// operation by itself. The receiving application must check its own request schema
/// and atomically consume this replay key together with the authorized mutation.
#[derive(Debug)]
pub struct VerifiedCastaliaPresentation {
    pub holder_public_key: String,
    pub request_digest_sha256: String,
    pub expires_at: u64,
    replay_key: String,
}
impl VerifiedCastaliaPresentation {
    pub fn replay_key(&self) -> &str {
        &self.replay_key
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionRequest {
    pub schema: String,
    pub action: String,
    pub actor: String,
    pub payload_digest: String,
    pub resource_version: u64,
    pub idempotency_key: String,
}

#[derive(Debug)]
pub struct VerifiedAuthorization {
    actor: String,
    action: String,
    payload_digest: String,
    resource_version: u64,
    expires_at: u64,
    pub(crate) nonce: String,
}
impl VerifiedAuthorization {
    pub fn require(&self, action: &str, actor: &str, payload: &[u8], now: u64) -> Result<()> {
        check(
            self.action == action
                && self.actor == actor
                && self.payload_digest == digest(payload)
                && now < self.expires_at,
            "Authorization binding or expiry mismatch",
        )
    }
    pub fn replay_key(&self) -> &str {
        &self.nonce
    }
    pub fn require_version(&self, version: u64) -> Result<()> {
        check(
            self.resource_version == version,
            "Authorization resource version mismatch",
        )
    }
}
fn check(ok: bool, reason: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Error::Denied(reason.into()))
    }
}
fn ascii(s: &str) -> bool {
    !s.is_empty() && s.len() <= 200 && s.bytes().all(|b| (0x20..=0x7e).contains(&b))
}
fn display(s: &str, max: usize) -> bool {
    !s.is_empty()&&s.len()<=max&&!s.chars().any(|c|c.is_control()||matches!(c,'\u{061c}'|'\u{200e}'|'\u{200f}'|'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}'))
}
fn caller(c: &Caller) -> bool {
    if c.kind == "terminal" {
        return ascii(&c.id);
    }
    if c.kind != "browser" {
        return false;
    }
    let Ok(u) = url::Url::parse(&c.id) else {
        return false;
    };
    let local = u
        .host_str()
        .is_some_and(|h| h == "localhost" || h == "127.0.0.1" || h == "[::1]");
    (u.scheme() == "https" || u.scheme() == "http" && local)
        && u.origin().ascii_serialization() == c.id
}
fn decode<const N: usize>(s: &str) -> Result<[u8; N]> {
    check(
        s.len() == N * 2
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "Noncanonical hex",
    )?;
    hex::decode(s)
        .map_err(|_| Error::Denied("Invalid hex".into()))?
        .try_into()
        .map_err(|_| Error::Denied("Invalid hex length".into()))
}
fn verify(public: &str, signature: &str, domain: &str, value: &impl Serialize) -> Result<()> {
    let key = VerifyingKey::from_bytes(&decode::<32>(public)?)
        .map_err(|_| Error::Denied("Invalid public key".into()))?;
    let sig = Signature::from_bytes(&decode::<64>(signature)?);
    let mut bytes = domain.as_bytes().to_vec();
    bytes.extend(canonical(value)?);
    key.verify_strict(&bytes, &sig)
        .map_err(|_| Error::Denied("Invalid Castalia signature".into()))
}
fn interval(start: u64, end: u64, now: u64, max: u64) -> Result<()> {
    check(
        start <= now
            && now < end
            && end > start
            && end - start <= max
            && end <= 9_007_199_254_740_991,
        "Expired or invalid validity interval",
    )
}

/// Validate generic Castalia credentials against exact, independently supplied
/// request and disclosure expectations. No Book Nook action schema is assumed.
pub fn validate_castalia_request(
    request_json: &[u8],
    policy: &CastaliaTrustPolicy,
    expected_request: &[u8],
    expected_disclosure: &Disclosure,
    now: u64,
) -> Result<PresentationRequest> {
    check(
        !expected_request.is_empty() && expected_request.len() <= 131072,
        "Invalid expected request size",
    )?;
    let expected: serde_json::Value = strict_parse(expected_request)?;
    check(
        expected.is_object() && canonical(&expected)? == expected_request,
        "Expected request must be a canonical JSON object",
    )?;
    let (request, bytes) = validate_credential_request(request_json, policy, now)?;
    check(
        bytes == expected_request && request.disclosure == *expected_disclosure,
        "Issuer changed the expected request or disclosure",
    )?;
    Ok(request)
}

/// Book Nook's application binding remains mandatory for every Book Nook action.
pub fn validate_request(
    request_json: &[u8],
    policy: &AuthorityPolicy,
    expected_payload: &[u8],
    expected_version: u64,
    now: u64,
) -> Result<(PresentationRequest, ActionRequest)> {
    let (request, bytes) = validate_credential_request(request_json, &policy.into(), now)?;
    let action: ActionRequest = strict_parse(&bytes)?;
    check(
        action.schema == "booknook.authorized-action.v1" && canonical(&action)? == bytes,
        "Invalid Book Nook action",
    )?;
    check(
        action.actor == policy.actor
            && action.action == policy.action
            && action.payload_digest == digest(expected_payload)
            && action.resource_version == expected_version
            && !action.idempotency_key.is_empty(),
        "Action does not match current execution bindings",
    )?;
    Ok((request, action))
}

fn validate_credential_request(
    request_json: &[u8],
    policy: &CastaliaTrustPolicy,
    now: u64,
) -> Result<(PresentationRequest, Vec<u8>)> {
    check(request_json.len() <= 262144, "Receipt too large")?;
    let request: PresentationRequest = strict_parse(request_json)?;
    check(
        request.schema == "castalia.credential-presentation-request.v2",
        "Wrong request schema",
    )?;
    let bytes = STANDARD
        .decode(&request.request_bytes_base64)
        .map_err(|_| Error::Denied("Invalid base64".into()))?;
    check(
        bytes.len() <= 131072 && STANDARD.encode(&bytes) == request.request_bytes_base64,
        "Noncanonical or oversized request bytes",
    )?;
    let c = &request.credential.claims;
    check(
        c.schema == "castalia.request-credential.v1"
            && ascii(&c.issuer)
            && ascii(&c.key_id)
            && ascii(&c.audience)
            && caller(&c.caller),
        "Invalid credential fields",
    )?;
    decode::<32>(&c.holder_public_key)?;
    decode::<16>(&c.nonce)?;
    decode::<32>(&c.policy_digest_sha256)?;
    check(
        c.holder_public_key == policy.holder_public_key
            && c.audience == policy.audience
            && c.caller == policy.expected_caller
            && c.policy_digest_sha256 == policy.policy_digest,
        "Credential authority mismatch",
    )?;
    interval(c.issued_at, c.expires_at, now, 120)?;
    check(
        display(&request.disclosure.title, 160)
            && !request.disclosure.statements.is_empty()
            && request.disclosure.statements.len() <= 144
            && request
                .disclosure
                .statements
                .iter()
                .all(|s| display(s, 512)),
        "Unsafe disclosure",
    )?;
    check(
        c.request_digest_sha256 == digest(&bytes)
            && c.disclosure_digest_sha256 == digest(&canonical(&request.disclosure)?),
        "Credential digest mismatch",
    )?;
    let pin = policy
        .pins
        .iter()
        .find(|p| {
            p.issuer == c.issuer
                && p.key_id == c.key_id
                && p.audience == c.audience
                && p.callers.contains(&c.caller)
        })
        .ok_or_else(|| Error::Denied("Issuer is not independently trusted".into()))?;
    verify(
        &pin.public_key,
        &request.credential.signature,
        "castalia.request-credential.v1/signature\0",
        c,
    )?;
    Ok((request, bytes))
}

pub fn verify_receipt(
    request_json: &[u8],
    presentation_json: &[u8],
    policy: &AuthorityPolicy,
    expected_payload: &[u8],
    expected_version: u64,
    now: u64,
) -> Result<VerifiedAuthorization> {
    let (request, action) = validate_request(
        request_json,
        policy,
        expected_payload,
        expected_version,
        now,
    )?;
    let verified = verify_presentation(&request, presentation_json, now)?;
    Ok(VerifiedAuthorization {
        actor: action.actor,
        action: action.action,
        payload_digest: action.payload_digest,
        resource_version: action.resource_version,
        expires_at: verified.expires_at,
        nonce: verified.replay_key,
    })
}

pub fn verify_castalia_receipt(
    request_json: &[u8],
    presentation_json: &[u8],
    policy: &CastaliaTrustPolicy,
    expected_request: &[u8],
    expected_disclosure: &Disclosure,
    now: u64,
) -> Result<VerifiedCastaliaPresentation> {
    let request = validate_castalia_request(
        request_json,
        policy,
        expected_request,
        expected_disclosure,
        now,
    )?;
    verify_presentation(&request, presentation_json, now)
}

fn verify_presentation(
    request: &PresentationRequest,
    presentation_json: &[u8],
    now: u64,
) -> Result<VerifiedCastaliaPresentation> {
    check(presentation_json.len() <= 16384, "Presentation too large")?;
    let c = &request.credential.claims;
    let mut value: serde_json::Value = strict_parse(presentation_json)?;
    let signature = value
        .as_object_mut()
        .and_then(|v| v.remove("signature"))
        .and_then(|v| v.as_str().map(str::to_owned))
        .ok_or_else(|| Error::Denied("Missing signature".into()))?;
    let p: UnsignedPresentation = serde_json::from_value(value)?;
    interval(p.issued_at, p.expires_at, now, 60)?;
    decode::<16>(&p.nonce)?;
    check(
        p.schema == "castalia.credential-presentation.v2"
            && p.holder_public_key == c.holder_public_key
            && p.issuer == c.issuer
            && p.key_id == c.key_id
            && p.audience == c.audience
            && p.caller == c.caller
            && p.request_digest_sha256 == c.request_digest_sha256
            && p.disclosure_digest_sha256 == c.disclosure_digest_sha256
            && p.credential_digest_sha256 == digest(&canonical(&request.credential)?)
            && p.issued_at >= c.issued_at
            && p.expires_at <= c.expires_at,
        "Presentation binding mismatch",
    )?;
    verify(
        &p.holder_public_key,
        &signature,
        "castalia.credential-presentation.v2/signature\0",
        &p,
    )?;
    Ok(VerifiedCastaliaPresentation {
        holder_public_key: p.holder_public_key.clone(),
        request_digest_sha256: p.request_digest_sha256,
        expires_at: p.expires_at,
        replay_key: format!("{}:{}", p.holder_public_key, p.nonce),
    })
}
