use wasm_bindgen::prelude::*;
mod castalia;
mod providers;

/// Full export envelope (state, original assets, archived social proofs), not State alone.
#[wasm_bindgen]
pub fn validate_portable_archive(archive_json: &str) -> Result<String, JsValue> {
    booknook_core::portable_archive::roundtrip(archive_json.as_bytes())
        .map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Browser hosts own persistence. Exactly the native core command/validation contract.
#[wasm_bindgen]
pub fn new_state() -> Result<String, JsValue> {
    serde_json::to_string(&booknook_core::model::State::default())
        .map_err(|e| JsValue::from_str(&e.to_string()))
}
#[wasm_bindgen]
pub fn execute(state_json: &str, command_json: &str) -> Result<String, JsValue> {
    let run = || -> booknook_core::Result<String> {
        let mut state = booknook_core::strict_parse(state_json.as_bytes())?;
        booknook_core::validate(&state)?;
        let command = booknook_core::strict_parse(command_json.as_bytes())?;
        let result = booknook_core::apply(&mut state, command)?;
        Ok(serde_json::to_string(
            &serde_json::json!({"state":state,"result":result}),
        )?)
    };
    run().map_err(|e| JsValue::from_str(&e.to_string()))
}
#[wasm_bindgen]
pub fn validate_archive(state_json: &str) -> Result<(), JsValue> {
    let run = || -> booknook_core::Result<()> {
        booknook_core::validate(&booknook_core::strict_parse(state_json.as_bytes())?)
    };
    run().map_err(|e| JsValue::from_str(&e.to_string()))
}

fn bridge<T: serde::Serialize>(result: booknook_core::Result<T>) -> Result<String, JsValue> {
    result
        .and_then(|value| Ok(serde_json::to_string(&value)?))
        .map_err(|e| JsValue::from_str(&e.to_string()))
}
/// Same three-way private reconciliation used by the native encrypted transport.
#[wasm_bindgen]
pub fn reconcile_states(
    base_json: &str,
    local_json: &str,
    incoming_json: &str,
) -> Result<String, JsValue> {
    bridge((|| {
        let base = if base_json.is_empty() {
            None
        } else {
            Some(booknook_core::strict_parse(base_json.as_bytes())?)
        };
        booknook_core::reconcile::reconcile(
            base.as_ref(),
            &booknook_core::strict_parse(local_json.as_bytes())?,
            &booknook_core::strict_parse(incoming_json.as_bytes())?,
        )
    })())
}
#[wasm_bindgen]
pub fn resolve_conflict(
    state_json: &str,
    conflict_json: &str,
    use_incoming: bool,
) -> Result<String, JsValue> {
    bridge((|| {
        booknook_core::reconcile::resolve(
            &booknook_core::strict_parse(state_json.as_bytes())?,
            &booknook_core::strict_parse(conflict_json.as_bytes())?,
            use_incoming,
        )
    })())
}
#[wasm_bindgen]
pub fn new_social_state() -> Result<String, JsValue> {
    bridge(Ok(booknook_core::social_protocol::SocialState::default()))
}
#[wasm_bindgen]
pub fn social_local(state_json: &str, command_json: &str) -> Result<String, JsValue> {
    bridge((|| {
        let mut state = booknook_core::strict_parse(state_json.as_bytes())?;
        booknook_core::social_protocol::validate(&state)?;
        booknook_core::social_protocol::local(
            &mut state,
            booknook_core::strict_parse(command_json.as_bytes())?,
        )?;
        Ok(state)
    })())
}
#[wasm_bindgen]
pub fn social_prepare(
    state_json: &str,
    actor: &str,
    action_json: &str,
    now: u64,
) -> Result<String, JsValue> {
    bridge((|| {
        let state = booknook_core::strict_parse(state_json.as_bytes())?;
        booknook_core::social_protocol::validate(&state)?;
        booknook_core::social_protocol::prepare(
            &state,
            actor,
            booknook_core::strict_parse(action_json.as_bytes())?,
            now,
        )
    })())
}
/// The host must load policy from its own independently trusted configuration. Never take
/// issuer pins, actor or permissions from an incoming event or an untrusted relay response.
#[wasm_bindgen]
pub fn social_approve(
    state_json: &str,
    event_json: &str,
    request_json: &str,
    presentation_json: &str,
    policy_json: &str,
    now: u64,
) -> Result<String, JsValue> {
    bridge((|| {
        let mut state = booknook_core::strict_parse(state_json.as_bytes())?;
        booknook_core::social_protocol::validate(&state)?;
        let publication = booknook_core::social_protocol::approve(
            &mut state,
            booknook_core::strict_parse(event_json.as_bytes())?,
            request_json.into(),
            presentation_json.into(),
            &booknook_core::strict_parse(policy_json.as_bytes())?,
            now,
        )?;
        Ok(serde_json::json!({"state":state,"publication":publication}))
    })())
}
#[wasm_bindgen]
pub fn social_receive(
    state_json: &str,
    publication_json: &str,
    policy_json: &str,
    now: u64,
) -> Result<String, JsValue> {
    bridge((|| {
        let mut state = booknook_core::strict_parse(state_json.as_bytes())?;
        booknook_core::social_protocol::validate(&state)?;
        let changed = booknook_core::social_protocol::receive(
            &mut state,
            booknook_core::strict_parse(publication_json.as_bytes())?,
            &booknook_core::strict_parse(policy_json.as_bytes())?,
            now,
        )?;
        Ok(serde_json::json!({"state":state,"changed":changed}))
    })())
}
#[wasm_bindgen]
pub fn social_feed(
    state_json: &str,
    viewer: &str,
    reveal_spoilers: bool,
) -> Result<String, JsValue> {
    bridge((|| {
        let state = booknook_core::strict_parse(state_json.as_bytes())?;
        booknook_core::social_protocol::validate(&state)?;
        Ok(booknook_core::social_protocol::feed(
            &state,
            viewer,
            reveal_spoilers,
        ))
    })())
}

/// Conservative source-metadata matches; no selection or position mapping is implied.
#[wasm_bindgen]
pub fn audio_edition_candidates(
    state_json: &str,
    observation_json: &str,
    now: u64,
) -> Result<String, JsValue> {
    bridge((|| {
        let state = booknook_core::strict_parse(state_json.as_bytes())?;
        booknook_core::validate(&state)?;
        let observation: booknook_core::audio::AudioObservation =
            booknook_core::strict_parse(observation_json.as_bytes())?;
        observation.validate(std::slice::from_ref(&observation.asin), now)?;
        booknook_core::audio::edition_candidates(&state, observation.metadata.as_ref())
    })())
}
/// Prepare the explicitly reviewed source observation; persist with ObservePosition.
#[wasm_bindgen]
pub fn reviewed_audio_observation(
    state_json: &str,
    observation_json: &str,
    pass_id: &str,
    confirmed_edition_id: Option<String>,
    now: u64,
) -> Result<String, JsValue> {
    bridge((|| {
        let state = booknook_core::strict_parse(state_json.as_bytes())?;
        booknook_core::validate(&state)?;
        booknook_core::audio::reviewed_observation(
            &state,
            &booknook_core::strict_parse(observation_json.as_bytes())?,
            pass_id,
            confirmed_edition_id.as_deref(),
            now,
        )
    })())
}

/// Public metadata validation only; does not install issuer trust or access credentials.
#[wasm_bindgen]
pub fn validate_provider_connections(connections_json: &str) -> Result<String, JsValue> {
    bridge(booknook_core::castalia_connections::parse_connections(
        connections_json.as_bytes(),
    ))
}
#[wasm_bindgen]
pub fn validate_native_authority_profile(profile_json: &str) -> Result<String, JsValue> {
    bridge(
        booknook_core::castalia_connections::NativeAuthorityProfile::parse(profile_json.as_bytes()),
    )
}

/// Catalog-only evaluation. Hosts must independently verify any supplied shared ratings;
/// use social_recommendations for imported social history with receipt/projection auditing.
#[wasm_bindgen]
pub fn recommendations(input_json: &str) -> Result<String, JsValue> {
    bridge((|| {
        booknook_core::providers::recommend(
            &booknook_core::providers::CatalogRecommendations,
            &booknook_core::strict_parse(input_json.as_bytes())?,
        )
    })())
}

#[wasm_bindgen]
pub fn social_recommendations(
    state_json: &str,
    candidates_json: &str,
    excluded_json: &str,
    policy_json: &str,
    now: u64,
) -> Result<String, JsValue> {
    bridge((|| {
        booknook_core::social_protocol::audited_recommendations(
            &booknook_core::strict_parse(state_json.as_bytes())?,
            &booknook_core::strict_parse(policy_json.as_bytes())?,
            booknook_core::strict_parse(candidates_json.as_bytes())?,
            booknook_core::strict_parse(excluded_json.as_bytes())?,
            now,
        )
    })())
}

#[wasm_bindgen]
pub fn prepare_sync_batch(
    actor: &str,
    revision: u64,
    recipients_json: &str,
    plaintext_bytes: &[u8],
) -> Result<String, JsValue> {
    bridge((|| {
        booknook_core::sync::prepare(
            actor,
            revision,
            booknook_core::strict_parse(recipients_json.as_bytes())?,
            plaintext_bytes,
        )
    })())
}

fn pairing_key(bytes: &[u8]) -> booknook_core::Result<&[u8; 32]> {
    bytes.try_into().map_err(|_| {
        booknook_core::Error::Invalid("Pairing key must contain exactly 32 bytes".into())
    })
}

/// Validate a fresh Castalia receipt before encryption. The returned replay key and packet
/// must be atomically committed by the host before transport. No signing is performed.
#[wasm_bindgen]
#[allow(clippy::too_many_arguments)]
pub fn seal_sync_batch(
    batch_json: &str,
    plaintext_bytes: &[u8],
    key_bytes: &[u8],
    request_json: &str,
    presentation_json: &str,
    policy_json: &str,
    now: u64,
) -> Result<String, JsValue> {
    bridge((|| {
        booknook_core::sync::seal_packet(
            booknook_core::strict_parse(batch_json.as_bytes())?,
            plaintext_bytes,
            pairing_key(key_bytes)?,
            request_json.into(),
            presentation_json.into(),
            &booknook_core::strict_parse(policy_json.as_bytes())?,
            now,
        )
    })())
}

/// Historical approval is verified against the host's current pins before decryption.
/// The host must separately enforce paired-peer identity and atomic application/replay.
#[wasm_bindgen]
pub fn open_sync_batch(
    packet_json: &str,
    key_bytes: &[u8],
    recipient: &str,
    policy_json: &str,
    now: u64,
) -> Result<String, JsValue> {
    bridge((|| {
        booknook_core::sync::open_packet(
            &booknook_core::strict_parse(packet_json.as_bytes())?,
            pairing_key(key_bytes)?,
            recipient,
            &booknook_core::strict_parse(policy_json.as_bytes())?,
            now,
        )
    })())
}
