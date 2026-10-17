// SPDX-License-Identifier: MIT OR Apache-2.0
//! Descriptive Rolodex references. These types cannot construct an authorization receipt.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
pub const TERMS: &str = "https://zenith-research.ca/ontology/terms/";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
// Schemars does not include serde aliases: publish the full accepted input vocabulary.
#[schemars(extend("enum" = ["Entity", "Person", "Agent", "Organization", "Actor"]))]
pub enum DirectoryKind {
    #[serde(alias = "Actor")]
    Entity,
    Person,
    Agent,
    Organization,
}
impl DirectoryKind {
    pub fn canonical_iri(&self) -> String {
        format!(
            "{TERMS}{}",
            match self {
                Self::Entity => "Entity",
                Self::Person => "Person",
                Self::Agent => "Agent",
                Self::Organization => "Organization",
            }
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DirectoryReference {
    pub entity_iri: String,
    pub kind: DirectoryKind,
    pub display_name: Option<String>,
}
impl DirectoryReference {
    pub fn validate(&self) -> Result<()> {
        let url = url::Url::parse(&self.entity_iri)
            .map_err(|_| Error::Invalid("Directory reference requires an absolute IRI".into()))?;
        if !["https", "urn", "did"].contains(&url.scheme())
            || !url.username().is_empty()
            || url.password().is_some()
            || self.entity_iri.len() > 2048
            || self
                .display_name
                .as_ref()
                .is_some_and(|n| n.len() > 500 || n.chars().any(char::is_control))
        {
            return Err(Error::Invalid("Invalid directory reference".into()));
        }
        Ok(())
    }
}
/// A source's descriptive assertion, retained with provenance. Wallet possession and actual
/// permission continue through the independent Castalia presentation verifier.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttributionLink {
    pub local_reader: String,
    pub directory: DirectoryReference,
    pub castalia_holder: Option<String>,
    pub evidence_source: String,
    pub observed_at: u64,
}
pub trait CastaliaIdentityProvider: crate::providers::Provider {
    fn resolve(&self, entity_iri: &str) -> Result<DirectoryReference>;
    fn attribution(&self, local_reader: &str) -> Result<Vec<AttributionLink>>;
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actor_is_only_an_entity_alias_and_literary_characters_are_not_directory_people() {
        let kind: DirectoryKind = serde_json::from_str("\"Actor\"").unwrap();
        assert_eq!(kind, DirectoryKind::Entity);
        assert_eq!(serde_json::to_string(&kind).unwrap(), "\"Entity\"");
        let schema = serde_json::to_value(schemars::schema_for!(DirectoryKind)).unwrap();
        assert!(
            schema["enum"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("Actor"))
        );
        assert_eq!(kind.canonical_iri(), format!("{TERMS}Entity"));
        assert!(serde_json::from_str::<DirectoryKind>("\"Character\"").is_err());
        assert!(
            DirectoryReference {
                entity_iri: "file:///private/data".into(),
                kind: DirectoryKind::Person,
                display_name: None
            }
            .validate()
            .is_err()
        );
        assert!(
            DirectoryReference {
                entity_iri: "https://directory.example/person/1".into(),
                kind: DirectoryKind::Person,
                display_name: Some("Reader".into())
            }
            .validate()
            .is_ok()
        );
    }
}
