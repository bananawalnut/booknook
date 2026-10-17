// SPDX-License-Identifier: MIT OR Apache-2.0
//! Replaceable portable notebook state. The composed Desk keeps its own transactional
//! settings, social, asset and edit-journal storage; this object does not replace it.
use crate::{NativeError, Result, store::Store};
use booknook_core::{
    Error, canonical,
    model::State,
    providers::{self, Capability, NotebookStorage, Provider, ProviderDescriptor},
    strict_parse,
};
use serde::Deserialize;
use std::{path::Path, sync::Mutex};

/// A host-owned, synchronous atomic store. Callbacks may run on any calling thread.
/// The host initializes its state and must serialize CAS with every other writer.
/// Return false on revision conflict, or throw on I/O failure, without writing.
/// A true result means the complete state is durably committed before returning.
#[uniffi::export(callback_interface)]
pub trait NotebookStorageHost: Send + Sync {
    fn descriptor_json(&self) -> Result<String>;
    fn load_json(&self) -> Result<String>;
    fn compare_and_save(&self, expected_revision: u64, state_json: String) -> Result<bool>;
}

enum Backend {
    Local(Mutex<Store>),
    Host(Box<dyn NotebookStorageHost>),
}

/// Standalone portable notebook command engine. It has no cache, network, signing,
/// social/settings persistence, content-asset ingestion or native text-edit journal.
#[derive(uniffi::Object)]
pub struct NotebookEngine {
    backend: Backend,
}

#[uniffi::export]
impl NotebookEngine {
    /// Use a separate directory for this standalone SQLite store, not an active Desk.
    #[uniffi::constructor]
    pub fn local(directory: String) -> Result<Self> {
        let value = Self {
            backend: Backend::Local(Mutex::new(Store::open(Path::new(&directory))?)),
        };
        value.snapshot()?;
        Ok(value)
    }

    /// No implicit initial state or writes. The host must already contain valid State.
    #[uniffi::constructor]
    pub fn with_storage(host: Box<dyn NotebookStorageHost>) -> Result<Self> {
        let value = Self {
            backend: Backend::Host(host),
        };
        value.snapshot()?;
        Ok(value)
    }

    pub fn snapshot(&self) -> Result<String> {
        self.with_storage_port(|storage| Ok(serde_json::to_string(&providers::snapshot(storage)?)?))
    }

    pub fn execute(&self, command_json: String) -> Result<String> {
        let command = strict_parse(command_json.as_bytes())?;
        self.with_storage_port(|storage| {
            Ok(serde_json::to_string(&providers::execute(
                storage, command,
            )?)?)
        })
    }
}

impl NotebookEngine {
    fn with_storage_port<T>(
        &self,
        operation: impl FnOnce(&dyn NotebookStorage) -> Result<T>,
    ) -> Result<T> {
        match &self.backend {
            Backend::Local(store) => {
                let store = store.lock().map_err(|_| NativeError::Failure {
                    message: "Notebook storage lock poisoned".into(),
                })?;
                operation(&*store)
            }
            Backend::Host(host) => {
                // Descriptor errors cannot be hidden by Provider::descriptor's
                // infallible signature. Read/validate first for every operation.
                let descriptor = read_descriptor(host.as_ref())?;
                operation(&HostPort {
                    host: host.as_ref(),
                    descriptor,
                })
            }
        }
    }
}

struct HostPort<'a> {
    host: &'a dyn NotebookStorageHost,
    descriptor: ProviderDescriptor,
}
impl Provider for HostPort<'_> {
    fn descriptor(&self) -> ProviderDescriptor {
        self.descriptor.clone()
    }
}
impl NotebookStorage for HostPort<'_> {
    fn load(&self) -> booknook_core::Result<State> {
        let json = self.host.load_json().map_err(host_error)?;
        let state = strict_parse(json.as_bytes())?;
        booknook_core::validate(&state)?;
        Ok(state)
    }
    fn compare_and_save(&self, expected_revision: u64, state: &State) -> booknook_core::Result<()> {
        booknook_core::validate(state)?;
        let json = serde_json::to_string(state)?;
        if json.len() > booknook_core::MAX_JSON_BYTES {
            return Err(Error::Invalid(
                "Notebook state exceeds the 128 MiB storage limit".into(),
            ));
        }
        let current = read_descriptor(self.host).map_err(host_error)?;
        let current_port = HostPort {
            host: self.host,
            descriptor: current,
        };
        providers::require_capability(&current_port, Capability::NotebookStorage)?;
        if canonical(&current_port.descriptor)? != canonical(&self.descriptor)? {
            return Err(Error::Conflict(
                "Notebook storage descriptor changed; reload before retrying".into(),
            ));
        }
        if !self
            .host
            .compare_and_save(expected_revision, json)
            .map_err(host_error)?
        {
            return Err(Error::Conflict(
                "Notebook storage changed; reload before retrying".into(),
            ));
        }
        Ok(())
    }
}

// Keep the existing core ProviderDescriptor wire shape, while rejecting unknown
// fields and oversized or ambiguous callback input at this foreign boundary.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HostDescriptor {
    id: String,
    capabilities: Vec<Capability>,
    available: bool,
    reason: Option<String>,
}
fn read_descriptor(host: &dyn NotebookStorageHost) -> Result<ProviderDescriptor> {
    let raw = host.descriptor_json()?;
    if raw.len() > 16 * 1024 {
        return Err(Error::Invalid("Notebook storage descriptor exceeds 16 KiB".into()).into());
    }
    let descriptor: HostDescriptor = strict_parse(raw.as_bytes())?;
    if descriptor.id.is_empty()
        || descriptor.id.len() > 256
        || descriptor.id.chars().any(char::is_control)
        || descriptor.reason.as_ref().is_some_and(|s| s.len() > 4096)
        || descriptor
            .capabilities
            .iter()
            .enumerate()
            .any(|(i, c)| descriptor.capabilities[..i].contains(c))
    {
        return Err(Error::Invalid("Invalid notebook storage descriptor".into()).into());
    }
    Ok(ProviderDescriptor {
        id: descriptor.id,
        capabilities: descriptor.capabilities,
        available: descriptor.available,
        reason: descriptor.reason,
    })
}
fn host_error(error: NativeError) -> Error {
    Error::Unavailable(format!("Notebook storage: {error}"))
}
