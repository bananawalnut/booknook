// SPDX-License-Identifier: MIT OR Apache-2.0
//! Standalone Castalia custody: no Book Nook desk, database or account is required.
use crate::{NativeError, Result, custody::Custody};
use booknook_core::{
    authority::{CastaliaTrustPolicy, Disclosure},
    canonical, strict_parse,
};
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(uniffi::Object)]
pub struct CastaliaCustody {
    inner: Mutex<Custody>,
}
#[uniffi::export]
impl CastaliaCustody {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Custody::default()),
        })
    }
    /// Attempts invalidate earlier preparations even if recovery validation fails.
    pub fn unlock(&self, recovery_json: String, passphrase: String) -> Result<String> {
        Ok(self.guard()?.unlock(recovery_json.as_bytes(), passphrase)?)
    }
    pub fn lock(&self) -> Result<()> {
        self.guard()?.lock();
        Ok(())
    }
    pub fn public_key(&self) -> Result<Option<String>> {
        Ok(self.guard()?.public_key())
    }
    /// Isolated validation never changes this object's unlocked identity.
    pub fn validate_recovery(&self, recovery_json: String, passphrase: String) -> Result<String> {
        Ok(Custody::default().unlock(recovery_json.as_bytes(), passphrase)?)
    }
    pub fn canonical_request(&self, request_json: String) -> Result<String> {
        if request_json.len() > 131072 {
            return Err(booknook_core::Error::Invalid("Request too large".into()).into());
        }
        let value: serde_json::Value = strict_parse(request_json.as_bytes())?;
        if !value.is_object() {
            return Err(
                booknook_core::Error::Invalid("Expected request must be an object".into()).into(),
            );
        }
        Ok(String::from_utf8(canonical(&value)?).unwrap())
    }
    /// The host provides its own exact canonical request and disclosure. Incoming
    /// credentials cannot replace independent pins, holder, caller or audience.
    pub fn prepare(
        &self,
        request_json: String,
        expected_request_json: String,
        expected_disclosure_json: String,
        policy_json: String,
    ) -> Result<String> {
        let policy = policy(&policy_json)?;
        if expected_disclosure_json.len() > 131072 {
            return Err(booknook_core::Error::Invalid("Disclosure too large".into()).into());
        }
        let disclosure: Disclosure = strict_parse(expected_disclosure_json.as_bytes())?;
        let prepared = self.guard()?.prepare_castalia(
            request_json.as_bytes(),
            expected_request_json.as_bytes(),
            disclosure,
            policy,
            crate::now()?,
        )?;
        Ok(serde_json::to_string(&prepared)?)
    }
    /// Call only after explicit consent, with newly loaded independent trust.
    pub fn approve(&self, token: String, current_policy_json: String) -> Result<String> {
        let mut custody = self.guard()?;
        let current = match policy(&current_policy_json) {
            Ok(value) => value,
            Err(error) => {
                custody.cancel(&token);
                return Err(error);
            }
        };
        Ok(custody.approve_castalia(&token, &current, crate::now()?)?)
    }
    pub fn cancel(&self, token: String) -> Result<()> {
        self.guard()?.cancel(&token);
        Ok(())
    }
}
impl CastaliaCustody {
    fn guard(&self) -> Result<MutexGuard<'_, Custody>> {
        self.inner.lock().map_err(|_| NativeError::Failure {
            message: "Custody lock unavailable; create a new locked session".into(),
        })
    }
}
fn policy(raw: &str) -> Result<CastaliaTrustPolicy> {
    if raw.len() > 131072 {
        return Err(booknook_core::Error::Invalid("Trust policy too large".into()).into());
    }
    Ok(strict_parse(raw.as_bytes())?)
}
