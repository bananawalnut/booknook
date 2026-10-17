// SPDX-License-Identifier: MIT OR Apache-2.0
use crate::{Desk, Result};
use booknook_core::{Command, audio, strict_parse};

#[uniffi::export]
impl Desk {
    pub fn audio_edition_candidates(&self, observation_json: String) -> Result<String> {
        let observation: audio::AudioObservation = strict_parse(observation_json.as_bytes())?;
        observation.validate(std::slice::from_ref(&observation.asin), crate::now()?)?;
        let state = self.lock()?.load()?;
        Ok(serde_json::to_string(&audio::edition_candidates(
            &state,
            observation.metadata.as_ref(),
        )?)?)
    }

    /// Call only after the reader reviews the chosen pass and optional edition association.
    pub fn attach_audio_observation(
        &self,
        observation_json: String,
        pass_id: String,
        confirmed_edition_id: Option<String>,
    ) -> Result<String> {
        let source: audio::AudioObservation = strict_parse(observation_json.as_bytes())?;
        let mut store = self.lock()?;
        let observation = audio::reviewed_observation(
            &store.load()?,
            &source,
            &pass_id,
            confirmed_edition_id.as_deref(),
            crate::now()?,
        )?;
        store.execute(Command::ObservePosition { observation })
    }
    pub fn attach_audio_observation_from_connection(
        &self,
        observation_json: String,
        pass_id: String,
        confirmed_edition_id: Option<String>,
        connection_id: String,
    ) -> Result<String> {
        let source: audio::AudioObservation = strict_parse(observation_json.as_bytes())?;
        let mut store = self.lock()?;
        crate::connection_bridge::require_connection(
            &crate::connection_bridge::load(&store.connection)?,
            &connection_id,
            "com.audible",
            "audible.positions",
        )?;
        let mut observation = audio::reviewed_observation(
            &store.load()?,
            &source,
            &pass_id,
            confirmed_edition_id.as_deref(),
            crate::now()?,
        )?;
        observation.id = booknook_core::digest(&booknook_core::canonical(&(
            "booknook.source-bound-position.v1",
            &connection_id,
            &observation.id,
        ))?);
        observation.provider_connection_id = Some(connection_id);
        store.execute(Command::ObservePosition { observation })
    }
}
