use crate::{
    Error, Result,
    authority::{AuthorityPolicy, VerifiedAuthorization, verify_receipt},
    canonical, digest,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Batch {
    pub schema: String,
    pub id: String,
    pub actor: String,
    pub revision: u64,
    pub recipients: Vec<String>,
    pub content_digest: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncryptedBatch {
    pub batch: Batch,
    pub nonce: String,
    pub ciphertext: String,
}

/// The native file transport's existing wire format, also available to portable hosts.
/// Approval fields are historical evidence, never an issuer trust configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovedPacket {
    pub schema: String,
    pub envelope: EncryptedBatch,
    pub approved_at: u64,
    pub request_json: String,
    pub presentation_json: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SealedPacket {
    pub packet: ApprovedPacket,
    /// Commit consumption of this receipt with the durable outbox before sending.
    pub replay_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenedPacket {
    pub plaintext: Vec<u8>,
    /// Atomically store batch ID + digest with the application of the plaintext.
    /// An equal ID/digest is an idempotent duplicate; a changed digest is a conflict.
    pub packet_digest: String,
    pub replay_key: String,
}

pub const MAX_CONTENT_BYTES: usize = 64 * 1024 * 1024;

fn validate_batch(batch: &Batch) -> Result<()> {
    // Native peer IDs historically permit Unicode. Do not narrow existing paired identities.
    let bounded_id = |value: &str| !value.is_empty() && value.len() <= 200;
    let recipients: std::collections::BTreeSet<_> = batch.recipients.iter().collect();
    if batch.schema != "booknook.sync-batch.v1"
        || !bounded_id(&batch.id)
        || !bounded_id(&batch.actor)
        || batch.recipients.is_empty()
        || batch.recipients.len() > 64
        || recipients.len() != batch.recipients.len()
        || batch.recipients.iter().any(|id| !bounded_id(id))
        || batch.content_digest.len() != 64
        || !batch
            .content_digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::Invalid("Invalid sync batch".into()));
    }
    Ok(())
}

/// Validate historical approval using independently trusted *current* issuer pins.
/// A packet cannot supply or replace the host's policy, paired peer, or replay ledger.
pub fn verify_packet(
    packet: &ApprovedPacket,
    policy: &AuthorityPolicy,
    now: u64,
) -> Result<VerifiedAuthorization> {
    if packet.schema != "booknook.sync-packet.v1" || packet.approved_at > now {
        return Err(Error::Invalid(
            "Unsupported sync packet or future approval".into(),
        ));
    }
    validate_batch(&packet.envelope.batch)?;
    let batch = &packet.envelope.batch;
    let payload = canonical(batch)?;
    let receipt = verify_receipt(
        packet.request_json.as_bytes(),
        packet.presentation_json.as_bytes(),
        policy,
        &payload,
        batch.revision,
        packet.approved_at,
    )?;
    receipt.require("sync.send", &batch.actor, &payload, packet.approved_at)?;
    Ok(receipt)
}

/// Encode only a freshly approved batch. This does not consume the receipt or send it.
/// The host must commit `replay_key` consumption and the exact packet in one transaction.
#[allow(clippy::too_many_arguments)]
pub fn seal_packet(
    batch: Batch,
    plaintext: &[u8],
    key: &[u8; 32],
    request_json: String,
    presentation_json: String,
    policy: &AuthorityPolicy,
    now: u64,
) -> Result<SealedPacket> {
    validate_batch(&batch)?;
    if plaintext.len() > MAX_CONTENT_BYTES {
        return Err(Error::Invalid("Sync content exceeds 64 MiB".into()));
    }
    let receipt = verify_receipt(
        request_json.as_bytes(),
        presentation_json.as_bytes(),
        policy,
        &canonical(&batch)?,
        batch.revision,
        now,
    )?;
    let replay_key = receipt.replay_key().to_owned();
    let packet = ApprovedPacket {
        schema: "booknook.sync-packet.v1".into(),
        envelope: seal(batch, plaintext, key, &receipt, now)?,
        approved_at: now,
        request_json,
        presentation_json,
    };
    Ok(SealedPacket { packet, replay_key })
}

/// Verify approval before decrypting. The host still checks its paired source identity,
/// payload schema/source/version and atomic batch replay ledger before applying content.
pub fn open_packet(
    packet: &ApprovedPacket,
    key: &[u8; 32],
    recipient: &str,
    policy: &AuthorityPolicy,
    now: u64,
) -> Result<OpenedPacket> {
    let receipt = verify_packet(packet, policy, now)?;
    // Bound the authenticated plaintext before allocating its decoded ciphertext.
    if packet.envelope.ciphertext.len() > (MAX_CONTENT_BYTES + 16).div_ceil(3) * 4 {
        return Err(Error::Invalid("Sync content exceeds 64 MiB".into()));
    }
    let plaintext = open(&packet.envelope, key, recipient)?;
    if plaintext.len() > MAX_CONTENT_BYTES {
        return Err(Error::Invalid("Sync content exceeds 64 MiB".into()));
    }
    Ok(OpenedPacket {
        plaintext,
        packet_digest: digest(&canonical(packet)?),
        replay_key: receipt.replay_key().to_owned(),
    })
}

pub fn prepare(
    actor: &str,
    revision: u64,
    recipients: Vec<String>,
    plaintext: &[u8],
) -> Result<Batch> {
    if recipients.is_empty() {
        return Err(Error::Invalid("Select sync recipients".into()));
    }
    let batch = Batch {
        schema: "booknook.sync-batch.v1".into(),
        id: crate::new_id(),
        actor: actor.into(),
        revision,
        recipients,
        content_digest: digest(plaintext),
    };
    validate_batch(&batch)?;
    if plaintext.len() > MAX_CONTENT_BYTES {
        return Err(Error::Invalid("Sync content exceeds 64 MiB".into()));
    }
    Ok(batch)
}
pub fn seal(
    batch: Batch,
    plaintext: &[u8],
    key: &[u8; 32],
    authorization: &VerifiedAuthorization,
    now: u64,
) -> Result<EncryptedBatch> {
    authorization.require("sync.send", &batch.actor, &canonical(&batch)?, now)?;
    authorization.require_version(batch.revision)?;
    if batch.content_digest != digest(plaintext) {
        return Err(Error::Conflict("Content changed after approval".into()));
    }
    let mut nonce = [0u8; 24];
    getrandom::getrandom(&mut nonce)
        .map_err(|_| Error::Unavailable("Entropy unavailable".into()))?;
    let ciphertext = XChaCha20Poly1305::new(key.into())
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: &canonical(&batch)?,
            },
        )
        .map_err(|_| Error::Invalid("Encryption failed".into()))?;
    Ok(EncryptedBatch {
        batch,
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(ciphertext),
    })
}
pub fn open(envelope: &EncryptedBatch, key: &[u8; 32], recipient: &str) -> Result<Vec<u8>> {
    if envelope.batch.schema != "booknook.sync-batch.v1"
        || !envelope.batch.recipients.iter().any(|r| r == recipient)
    {
        return Err(Error::Denied("Wrong sync recipient".into()));
    }
    let nonce = STANDARD
        .decode(&envelope.nonce)
        .map_err(|_| Error::Invalid("Invalid nonce".into()))?;
    if nonce.len() != 24 {
        return Err(Error::Invalid("Invalid nonce size".into()));
    }
    let ciphertext = STANDARD
        .decode(&envelope.ciphertext)
        .map_err(|_| Error::Invalid("Invalid ciphertext".into()))?;
    let plain = XChaCha20Poly1305::new(key.into())
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &ciphertext,
                aad: &canonical(&envelope.batch)?,
            },
        )
        .map_err(|_| Error::Denied("Encrypted batch authentication failed".into()))?;
    if digest(&plain) != envelope.batch.content_digest {
        return Err(Error::Denied("Content digest mismatch".into()));
    }
    Ok(plain)
}
