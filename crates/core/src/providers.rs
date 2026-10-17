//! Ports: a catalog capability does not imply content or position access.
use crate::{Command, Error, Result, model::*};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Catalog,
    Content,
    Positions,
    NotebookStorage,
    SocialExchange,
    Recommendations,
    CredentialPresentationV2,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderDescriptor {
    pub id: String,
    pub capabilities: Vec<Capability>,
    pub available: bool,
    pub reason: Option<String>,
}

pub trait Provider {
    fn descriptor(&self) -> ProviderDescriptor;
}
pub trait LibraryProvider: Provider {
    fn import_catalog(&self, bytes: &[u8]) -> Result<Vec<Book>>;
}
pub trait ContentProvider: Provider {
    fn read_asset(&self, key: &str) -> Result<Vec<u8>>;
}
pub trait PositionProvider: Provider {
    fn observations(&self) -> Result<Vec<ProgressObservation>>;
}
pub trait NotebookStorage: Provider {
    /// Return the current complete portable state. The provider initializes its own store.
    fn load(&self) -> Result<State>;
    /// Atomically commit only if the stored revision equals `expected_revision`.
    /// On conflict or failure, return an error without modifying the stored state.
    fn compare_and_save(&self, expected_revision: u64, state: &State) -> Result<()>;
}
pub trait RecommendationProvider: Provider {
    fn recommend(
        &self,
        input: &crate::social::RecommendationInput,
    ) -> Result<Vec<crate::social::Recommendation>>;
}
pub trait SocialTransport: Provider {
    fn send(&self, signed_event: &[u8]) -> Result<String>;
}
pub trait CastaliaAuthority: Provider {
    /// Native custody must present trusted disclosure and obtain fresh user consent.
    /// An unavailable provider MUST fail; a local UUID cannot authorize publication.
    fn present_credential(&self, request: &[u8]) -> Result<Vec<u8>>;
}

/// The same command/result boundary used by the native reference app. Hosts can supply any
/// atomic NotebookStorage, without a Book Nook service or SQLite dependency in the core.
#[derive(Debug, Serialize, Deserialize)]
pub struct CommandResponse {
    pub state: State,
    pub result: serde_json::Value,
}

pub fn require_capability(provider: &dyn Provider, capability: Capability) -> Result<()> {
    let descriptor = provider.descriptor();
    if !descriptor.available {
        return Err(Error::Unavailable(descriptor.reason.unwrap_or_else(|| {
            format!("Provider {} is unavailable", descriptor.id)
        })));
    }
    if !descriptor.capabilities.contains(&capability) {
        return Err(Error::Unavailable(format!(
            "Provider {} does not offer {capability:?}",
            descriptor.id
        )));
    }
    Ok(())
}

/// Validate a provider's state before exposing it, regardless of its storage format.
pub fn snapshot(storage: &dyn NotebookStorage) -> Result<State> {
    require_capability(storage, Capability::NotebookStorage)?;
    let state = storage.load()?;
    crate::validate(&state)?;
    Ok(state)
}

/// Execute through the advertised storage contract. A failed CAS never returns a successful
/// response or updates a cached projection; retry requires a fresh load and user decision.
pub fn execute(storage: &dyn NotebookStorage, command: Command) -> Result<CommandResponse> {
    let mut state = snapshot(storage)?;
    let revision = state.revision;
    let result = crate::apply(&mut state, command)?;
    storage.compare_and_save(revision, &state)?;
    Ok(CommandResponse { state, result })
}

pub struct Notebook<S: NotebookStorage> {
    storage: S,
}
impl<S: NotebookStorage> Notebook<S> {
    pub fn open(storage: S) -> Result<Self> {
        snapshot(&storage)?;
        Ok(Self { storage })
    }
    pub fn snapshot(&self) -> Result<State> {
        snapshot(&self.storage)
    }
    pub fn execute(&self, command: Command) -> Result<CommandResponse> {
        execute(&self.storage, command)
    }
    pub fn into_storage(self) -> S {
        self.storage
    }
}

/// Local explainable recommendations. The input deliberately has no private-writing fields.
pub struct CatalogRecommendations;
impl Provider for CatalogRecommendations {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: "booknook.local-recommendations".into(),
            capabilities: vec![Capability::Recommendations],
            available: true,
            reason: None,
        }
    }
}
impl RecommendationProvider for CatalogRecommendations {
    fn recommend(
        &self,
        input: &crate::social::RecommendationInput,
    ) -> Result<Vec<crate::social::Recommendation>> {
        Ok(crate::social::recommend(input))
    }
}
pub fn recommend(
    provider: &dyn RecommendationProvider,
    input: &crate::social::RecommendationInput,
) -> Result<Vec<crate::social::Recommendation>> {
    require_capability(provider, Capability::Recommendations)?;
    provider.recommend(input)
}

pub fn release_gates() -> Vec<ProviderDescriptor> {
    vec![
        ProviderDescriptor { id: "kindle-live".into(), capabilities: vec![Capability::Catalog,Capability::Positions], available: false, reason: Some("Account authentication and real catalog qualification required; file catalog import is available.".into()) },
        ProviderDescriptor { id: "audible-live".into(), capabilities: vec![Capability::Positions], available: false, reason: Some("Non-public endpoint adapter requires valid account access and live validation.".into()) },
        ProviderDescriptor { id: "castalia-native".into(), capabilities: vec![Capability::CredentialPresentationV2], available: false, reason: Some("Native custody and explicit consent are implemented and tested against the upstream synthetic recovery vector. Real issuer provisioning and physical-device custody qualification remain required for release.".into()) },
        ProviderDescriptor { id: "physical-ipad".into(), capabilities: vec![], available: false, reason: Some("Pencil, IME, accessibility and reflow qualification on physical hardware required.".into()) },
    ]
}
