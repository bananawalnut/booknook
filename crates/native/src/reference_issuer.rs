// SPDX-License-Identifier: MIT OR Apache-2.0
//! Application-specific issuance policy over the unchanged generic Castalia v2 contract.
//! This module is compiled only by the optional issuer binary, not the native app library.
use base64::{Engine, engine::general_purpose::STANDARD};
use booknook_core::castalia_connections::{
    NativeAuthorityProfile, NativeAuthorityProfileSchema, NativeCaller, NativeCallerKind,
};
use booknook_core::{authority::*, canonical, digest, strict_parse};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroizing;

const MAX: usize = 262_144;
const SAFE: u64 = 9_007_199_254_740_991;
const ACTIONS: &[&str] = &[
    "sync.send",
    "device.enroll",
    "social.execute",
    "social.publish",
    "nostr.bind",
    "group.membership",
];
#[derive(Debug)]
pub enum Failure {
    Invalid,
    Denied,
    Configuration,
}
impl Failure {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid => "issuer_invalid_request",
            Self::Denied => "issuer_denied",
            Self::Configuration => "issuer_not_configured",
        }
    }
    pub fn exit(&self) -> i32 {
        match self {
            Self::Invalid => 2,
            Self::Denied => 3,
            Self::Configuration => 4,
        }
    }
}
type Result<T> = std::result::Result<T, Failure>;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
    schema: String,
    active: bool,
    issuer: String,
    key_id: String,
    public_key: String,
    signing_key_file: String,
    policy: Policy,
    clients: Vec<Client>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    schema: String,
    audience: String,
    version: u64,
    disclosure: DisclosureLimits,
    grants: Vec<Grant>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DisclosureLimits {
    mode: String,
    maximum_statements: usize,
    maximum_statement_bytes: usize,
    maximum_title_bytes: usize,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Grant {
    holder_public_key: String,
    caller: Caller,
    actions: Vec<String>,
    disclosure_titles: Vec<String>,
    not_before: u64,
    expires_at: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Client {
    token_sha256: String,
    holder_public_key: String,
    caller: Caller,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    schema: String,
    authorization: String,
    request: Request,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema: String,
    action_request: ActionRequest,
    disclosure: Disclosure,
    holder_public_key: String,
    audience: String,
    caller: Caller,
    policy_digest: String,
}
fn ascii(s: &str) -> bool {
    !s.is_empty() && s.len() <= 200 && s.bytes().all(|c| (0x20..=0x7e).contains(&c))
}
fn hex32(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn key(s: &str) -> bool {
    hex32(s)
        && hex::decode(s)
            .ok()
            .and_then(|b| <[u8; 32]>::try_from(b).ok())
            .is_some_and(|b| VerifyingKey::from_bytes(&b).is_ok())
}
fn caller(c: &Caller) -> bool {
    c.kind == "terminal" && ascii(&c.id)
}
fn display(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.chars().any(|c| c.is_control() || matches!(c, '\u{061c}'|'\u{200e}'|'\u{200f}'|'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}'))
}
impl Configuration {
    fn validate(&self) -> Result<()> {
        let p = &self.policy;
        let d = &p.disclosure;
        if self.schema != "booknook.issuer-config.v1"
            || !ascii(&self.issuer)
            || !ascii(&self.key_id)
            || !key(&self.public_key)
            || p.schema != "booknook.issuer-policy.v1"
            || !ascii(&p.audience)
            || p.version == 0
            || p.version > SAFE
            || d.mode != "holder_supplied_no_payload_inspection"
            || !(1..=144).contains(&d.maximum_statements)
            || !(1..=512).contains(&d.maximum_statement_bytes)
            || !(1..=160).contains(&d.maximum_title_bytes)
            || self.clients.len() > 256
            || p.grants.len() > 1024
            || self.clients.iter().any(|c| {
                !hex32(&c.token_sha256) || !key(&c.holder_public_key) || !caller(&c.caller)
            })
            || p.grants.iter().any(|g| {
                !key(&g.holder_public_key)
                    || !caller(&g.caller)
                    || g.not_before >= g.expires_at
                    || g.expires_at > SAFE
                    || g.actions.len() > ACTIONS.len()
                    || g.actions.iter().any(|a| !ACTIONS.contains(&a.as_str()))
                    || g.disclosure_titles.len() > 32
                    || g.disclosure_titles
                        .iter()
                        .any(|t| !display(t, d.maximum_title_bytes))
            })
        {
            return Err(Failure::Configuration);
        }
        Ok(())
    }
}

// Resolve each path component relative to an already-open directory. No symlink/hardlink
// is followed; the immediate parent and file must be private and owned by this process user.
#[cfg(unix)]
fn private_read(path: &Path, maximum: usize) -> Result<Zeroizing<Vec<u8>>> {
    use std::{
        ffi::CString,
        fs::File,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::{ffi::OsStrExt, fs::MetadataExt},
        },
        path::Component,
    };
    if !path.is_absolute() {
        return Err(Failure::Configuration);
    }
    let mut directory = File::open("/").map_err(|_| Failure::Configuration)?;
    let parts: Vec<_> = path.components().skip(1).collect();
    if parts.is_empty() {
        return Err(Failure::Configuration);
    }
    for (i, part) in parts.iter().enumerate() {
        let Component::Normal(name) = part else {
            return Err(Failure::Configuration);
        };
        let name = CString::new(name.as_bytes()).map_err(|_| Failure::Configuration)?;
        let last = i == parts.len() - 1;
        if last {
            let m = directory.metadata().map_err(|_| Failure::Configuration)?;
            // SAFETY: geteuid has no arguments or ownership transfer.
            if m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o077 != 0 {
                return Err(Failure::Configuration);
            }
        }
        let flags = libc::O_RDONLY
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | if last { 0 } else { libc::O_DIRECTORY };
        // SAFETY: directory and CString remain alive; successful descriptor is owned exactly once below.
        let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(Failure::Configuration);
        }
        // SAFETY: openat returned a new owned descriptor.
        let file = unsafe { File::from_raw_fd(fd) };
        if last {
            let m = file.metadata().map_err(|_| Failure::Configuration)?;
            // SAFETY: geteuid has no arguments or ownership transfer.
            if !m.is_file()
                || m.nlink() != 1
                || m.uid() != unsafe { libc::geteuid() }
                || m.mode() & 0o077 != 0
                || m.len() > maximum as u64
            {
                return Err(Failure::Configuration);
            }
            let mut out = Zeroizing::new(Vec::new());
            file.take(maximum as u64 + 1)
                .read_to_end(&mut out)
                .map_err(|_| Failure::Configuration)?;
            if out.len() > maximum {
                return Err(Failure::Configuration);
            }
            return Ok(out);
        }
        directory = file;
    }
    Err(Failure::Configuration)
}
#[cfg(not(unix))]
fn private_read(_: &Path, _: usize) -> Result<Zeroizing<Vec<u8>>> {
    Err(Failure::Configuration)
}

fn load(path: &Path) -> Result<(Configuration, SigningKey)> {
    let config: Configuration =
        strict_parse(&private_read(path, MAX)?).map_err(|_| Failure::Configuration)?;
    config.validate()?;
    let bytes = private_read(Path::new(&config.signing_key_file), 32)?;
    let seed: Zeroizing<[u8; 32]> = Zeroizing::new(
        bytes
            .as_slice()
            .try_into()
            .map_err(|_| Failure::Configuration)?,
    );
    let signer = SigningKey::from_bytes(&seed);
    if hex::encode(signer.verifying_key().as_bytes()) != config.public_key {
        return Err(Failure::Configuration);
    }
    Ok((config, signer))
}
fn issue(
    config: &Configuration,
    signer: &SigningKey,
    call: &Call,
    now: u64,
) -> Result<PresentationRequest> {
    let r = &call.request;
    let a = &r.action_request;
    if call.schema != "booknook.issuer-call.v1"
        || r.schema != "booknook.credential-request.v1"
        || a.schema != "booknook.authorized-action.v1"
        || !hex32(&a.payload_digest)
        || !key(&r.holder_public_key)
        || a.actor != r.holder_public_key
        || !caller(&r.caller)
        || !ascii(&a.idempotency_key)
        || a.resource_version > SAFE
        || !ACTIONS.contains(&a.action.as_str())
        || now > SAFE - 120
    {
        return Err(Failure::Invalid);
    }
    if !config.active
        || r.audience != config.policy.audience
        || r.policy_digest
            != digest(&canonical(&config.policy).map_err(|_| Failure::Configuration)?)
        || !call.authorization.starts_with("Bearer ")
    {
        return Err(Failure::Denied);
    }
    let token = &call.authorization[7..];
    if token.len() < 32 || token.len() > 1024 || !token.bytes().all(|b| (0x21..=0x7e).contains(&b))
    {
        return Err(Failure::Denied);
    }
    let token_digest = digest(token.as_bytes());
    if !config.clients.iter().any(|c| {
        c.token_sha256 == token_digest
            && c.holder_public_key == r.holder_public_key
            && c.caller == r.caller
    }) {
        return Err(Failure::Denied);
    }
    let d = &config.policy.disclosure;
    if !display(&r.disclosure.title, d.maximum_title_bytes)
        || r.disclosure.statements.is_empty()
        || r.disclosure.statements.len() > d.maximum_statements
        || r.disclosure
            .statements
            .iter()
            .any(|s| !display(s, d.maximum_statement_bytes))
    {
        return Err(Failure::Denied);
    }
    let expires_at = config
        .policy
        .grants
        .iter()
        .filter(|g| {
            g.holder_public_key == r.holder_public_key
                && g.caller == r.caller
                && g.actions.contains(&a.action)
                && g.disclosure_titles.contains(&r.disclosure.title)
                && g.not_before <= now
                && now < g.expires_at
        })
        .map(|g| g.expires_at.min(now + 120))
        .max()
        .ok_or(Failure::Denied)?;
    let bytes = canonical(a).map_err(|_| Failure::Invalid)?;
    let mut nonce = [0u8; 16];
    getrandom::getrandom(&mut nonce).map_err(|_| Failure::Configuration)?;
    let claims = CredentialClaims {
        schema: "castalia.request-credential.v1".into(),
        issuer: config.issuer.clone(),
        key_id: config.key_id.clone(),
        holder_public_key: r.holder_public_key.clone(),
        audience: config.policy.audience.clone(),
        caller: r.caller.clone(),
        request_digest_sha256: digest(&bytes),
        disclosure_digest_sha256: digest(&canonical(&r.disclosure).map_err(|_| Failure::Invalid)?),
        policy_digest_sha256: r.policy_digest.clone(),
        nonce: hex::encode(nonce),
        issued_at: now,
        expires_at,
    };
    let mut transcript = b"castalia.request-credential.v1/signature\0".to_vec();
    transcript.extend(canonical(&claims).map_err(|_| Failure::Invalid)?);
    let credential = Credential {
        claims,
        signature: hex::encode(signer.sign(&transcript).to_bytes()),
    };
    Ok(PresentationRequest {
        schema: "castalia.credential-presentation-request.v2".into(),
        request_bytes_base64: STANDARD.encode(bytes),
        disclosure: r.disclosure.clone(),
        credential,
    })
}
fn authority_profile(
    config: &Configuration,
    endpoint: &str,
    caller_id: &str,
    name: &str,
) -> Result<NativeAuthorityProfile> {
    // A profile proposes public issuer configuration only. The supplied caller must
    // already be configured, but grants and active state are evaluated at issuance.
    if !config
        .clients
        .iter()
        .any(|client| client.caller.kind == "terminal" && client.caller.id == caller_id)
    {
        return Err(Failure::Denied);
    }
    let profile = NativeAuthorityProfile {
        schema: NativeAuthorityProfileSchema::V1,
        display_name: name.into(),
        issuer: config.issuer.clone(),
        key_id: config.key_id.clone(),
        issuer_public_key: config.public_key.clone(),
        audience: config.policy.audience.clone(),
        caller: NativeCaller {
            kind: NativeCallerKind::Terminal,
            id: caller_id.into(),
        },
        policy_digest_sha256: digest(
            &canonical(&config.policy).map_err(|_| Failure::Configuration)?,
        ),
        credential_endpoint: endpoint.into(),
    };
    profile.validate().map_err(|_| Failure::Invalid)?;
    Ok(profile)
}
pub fn run() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() < 3 || args[0] != "--config" {
        return Err(Failure::Invalid);
    }
    let profile_command = args[2] == "profile";
    if if profile_command {
        args.len() != 9 || args[3] != "--endpoint" || args[5] != "--caller" || args[7] != "--name"
    } else {
        args.len() != 3 || (args[2] != "issue" && args[2] != "describe")
    } {
        return Err(Failure::Invalid);
    }
    let (config, signer) = load(Path::new(&args[1]))?;
    let output = if profile_command {
        let endpoint = args[4].to_str().ok_or(Failure::Invalid)?;
        let caller = args[6].to_str().ok_or(Failure::Invalid)?;
        let name = args[8].to_str().ok_or(Failure::Invalid)?;
        canonical(&authority_profile(&config, endpoint, caller, name)?)
            .map_err(|_| Failure::Configuration)?
    } else if args[2] == "describe" {
        canonical(&serde_json::json!({"schema":"booknook.issuer-description.v1","issuer":config.issuer,"key_id":config.key_id,
            "issuer_public_key":config.public_key,"audience":config.policy.audience,"policy_digest":digest(&canonical(&config.policy).map_err(|_| Failure::Configuration)?),"active":config.active})).map_err(|_| Failure::Configuration)?
    } else {
        let mut raw = Zeroizing::new(Vec::new());
        std::io::stdin()
            .take((MAX * 2 + 1) as u64)
            .read_to_end(&mut raw)
            .map_err(|_| Failure::Invalid)?;
        if raw.len() > MAX * 2 {
            return Err(Failure::Invalid);
        }
        let mut call: Call = strict_parse(&raw).map_err(|_| Failure::Invalid)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Failure::Configuration)?
            .as_secs();
        let result = issue(&config, &signer, &call, now);
        zeroize::Zeroize::zeroize(&mut call.authorization);
        canonical(&result?).map_err(|_| Failure::Invalid)?
    };
    if output.len() > MAX {
        return Err(Failure::Invalid);
    }
    std::io::stdout()
        .write_all(&output)
        .map_err(|_| Failure::Invalid)
}
