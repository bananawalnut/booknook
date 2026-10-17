// SPDX-License-Identifier: MIT OR Apache-2.0
//! Native contract test driver. Synthetic signing seeds are public test data, never custody.
//! Build with `cargo build -p booknook-core --example contract-probe` before JS parity tests.
use booknook_core::{Command, model::State, reconcile, social_protocol as social};
use serde::Deserialize;
use std::io::Read;

#[path = "../tests/fixtures/social_receipts.rs"]
mod receipts;
#[path = "../tests/fixtures/receipts.rs"]
mod sync_receipts;

// This short-lived test driver uses direct portable contracts for transparent parity checks.
#[allow(clippy::large_enum_variant)]
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Probe {
    ProviderPosition {
        state: String,
        registry: String,
        generation: u64,
        execution: String,
        observation: String,
        pass_id: String,
        now: u64,
    },
    ProviderCatalog {
        state: String,
        registry: String,
        generation: u64,
        execution: String,
        now: u64,
    },
    CastaliaVerify {
        request: String,
        presentation: String,
        expected_request: String,
        disclosure: String,
        policy: String,
        now: u64,
    },
    Recommendations {
        raw: String,
    },
    SocialRecommendations {
        state: social::SocialState,
        candidates: Vec<booknook_core::social::CatalogCandidate>,
        excluded: std::collections::BTreeSet<String>,
        policy: booknook_core::authority::AuthorityPolicy,
        now: u64,
    },
    SyncPrepare {
        actor: String,
        revision: u64,
        recipients: Vec<String>,
        plaintext: Vec<u8>,
    },
    SyncReceipt {
        batch: booknook_core::sync::Batch,
    },
    SyncSeal {
        batch: booknook_core::sync::Batch,
        plaintext: Vec<u8>,
        key: Vec<u8>,
        request: String,
        presentation: String,
        policy: booknook_core::authority::AuthorityPolicy,
        now: u64,
    },
    SyncOpen {
        raw: String,
        key: Vec<u8>,
        recipient: String,
        policy: booknook_core::authority::AuthorityPolicy,
        now: u64,
    },
    CastaliaProfile {
        raw: String,
    },
    ProviderConnections {
        raw: String,
    },
    Normalize {
        state: State,
    },
    Execute {
        state: State,
        command: Command,
    },
    Reconcile {
        base: Option<State>,
        local: State,
        incoming: State,
    },
    Resolve {
        state: State,
        conflict: reconcile::Conflict,
        use_incoming: bool,
    },
    AudioCandidates {
        state: State,
        observation: booknook_core::audio::AudioObservation,
        now: u64,
    },
    AudioReview {
        state: State,
        observation: booknook_core::audio::AudioObservation,
        pass_id: String,
        confirmed_edition_id: Option<String>,
        now: u64,
    },
    SocialNew,
    SocialLocal {
        state: social::SocialState,
        command: social::LocalSocialCommand,
    },
    SocialPrepare {
        state: social::SocialState,
        actor: String,
        action: social::SocialAction,
        now: u64,
    },
    Receipt {
        event: social::SocialEvent,
        seed: u8,
    },
    SocialApprove {
        state: social::SocialState,
        event: social::SocialEvent,
        request: String,
        presentation: String,
        policy: booknook_core::authority::AuthorityPolicy,
        now: u64,
    },
    SocialReceive {
        state: social::SocialState,
        publication: social::ApprovedPublication,
        policy: booknook_core::authority::AuthorityPolicy,
        now: u64,
    },
}
fn run(probe: Probe) -> booknook_core::Result<serde_json::Value> {
    use serde_json::to_value as value;
    Ok(match probe {
        Probe::ProviderPosition {
            state,
            registry,
            generation,
            execution,
            observation,
            pass_id,
            now,
        } => {
            let mut state: State = booknook_core::strict_parse(state.as_bytes())?;
            booknook_core::validate(&state)?;
            let result = booknook_core::provider_runtime::apply_source_position(
                &mut state,
                &booknook_core::provider_runtime::parse_registry(registry.as_bytes())?,
                generation,
                execution.as_bytes(),
                observation.as_bytes(),
                &pass_id,
                now,
            )?;
            booknook_core::validate(&state)?;
            serde_json::json!({"state":state,"result":result})
        }
        Probe::ProviderCatalog {
            state,
            registry,
            generation,
            execution,
            now,
        } => {
            let mut state = booknook_core::strict_parse(state.as_bytes())?;
            booknook_core::validate(&state)?;
            let result = booknook_core::provider_runtime::apply_catalog(
                &mut state,
                &booknook_core::provider_runtime::parse_registry(registry.as_bytes())?,
                generation,
                execution.as_bytes(),
                now,
            )?;
            serde_json::json!({"state":state,"result":result})
        }
        Probe::CastaliaVerify {
            request,
            presentation,
            expected_request,
            disclosure,
            policy,
            now,
        } => {
            let result = booknook_core::authority::verify_castalia_receipt(
                request.as_bytes(),
                presentation.as_bytes(),
                &booknook_core::strict_parse(policy.as_bytes())?,
                expected_request.as_bytes(),
                &booknook_core::strict_parse(disclosure.as_bytes())?,
                now,
            )?;
            serde_json::json!({"holder_public_key":result.holder_public_key,
                "request_digest_sha256":result.request_digest_sha256,
                "expires_at":result.expires_at,"replay_key":result.replay_key()})
        }
        Probe::Recommendations { raw } => value(booknook_core::providers::recommend(
            &booknook_core::providers::CatalogRecommendations,
            &booknook_core::strict_parse(raw.as_bytes())?,
        )?)?,
        Probe::SocialRecommendations {
            state,
            candidates,
            excluded,
            policy,
            now,
        } => value(social::audited_recommendations(
            &state, &policy, candidates, excluded, now,
        )?)?,
        Probe::SyncPrepare {
            actor,
            revision,
            recipients,
            plaintext,
        } => value(booknook_core::sync::prepare(
            &actor, revision, recipients, &plaintext,
        )?)?,
        Probe::SyncReceipt { batch } => {
            let (request, presentation, policy) = sync_receipts::receipt(
                &booknook_core::canonical(&batch)?,
                "sync.send",
                batch.revision,
            );
            serde_json::json!({"request":String::from_utf8(request).unwrap(),
                "presentation":String::from_utf8(presentation).unwrap(),"policy":policy})
        }
        Probe::SyncSeal {
            batch,
            plaintext,
            key,
            request,
            presentation,
            policy,
            now,
        } => value(booknook_core::sync::seal_packet(
            batch,
            &plaintext,
            pairing_key(&key)?,
            request,
            presentation,
            &policy,
            now,
        )?)?,
        Probe::SyncOpen {
            raw,
            key,
            recipient,
            policy,
            now,
        } => value(booknook_core::sync::open_packet(
            &booknook_core::strict_parse(raw.as_bytes())?,
            pairing_key(&key)?,
            &recipient,
            &policy,
            now,
        )?)?,
        Probe::CastaliaProfile { raw } => value(
            booknook_core::castalia_connections::NativeAuthorityProfile::parse(raw.as_bytes())?,
        )?,
        Probe::ProviderConnections { raw } => value(
            booknook_core::castalia_connections::parse_connections(raw.as_bytes())?,
        )?,
        Probe::Normalize { state } => {
            booknook_core::validate(&state)?;
            value(state)?
        }
        Probe::Execute { mut state, command } => {
            booknook_core::validate(&state)?;
            let result = booknook_core::apply(&mut state, command)?;
            serde_json::json!({"state":state,"result":result})
        }
        Probe::Reconcile {
            base,
            local,
            incoming,
        } => value(reconcile::reconcile(base.as_ref(), &local, &incoming)?)?,
        Probe::Resolve {
            state,
            conflict,
            use_incoming,
        } => value(reconcile::resolve(&state, &conflict, use_incoming)?)?,
        Probe::AudioCandidates {
            state,
            observation,
            now,
        } => {
            booknook_core::validate(&state)?;
            observation.validate(std::slice::from_ref(&observation.asin), now)?;
            value(booknook_core::audio::edition_candidates(
                &state,
                observation.metadata.as_ref(),
            )?)?
        }
        Probe::AudioReview {
            state,
            observation,
            pass_id,
            confirmed_edition_id,
            now,
        } => {
            booknook_core::validate(&state)?;
            value(booknook_core::audio::reviewed_observation(
                &state,
                &observation,
                &pass_id,
                confirmed_edition_id.as_deref(),
                now,
            )?)?
        }
        Probe::SocialNew => value(social::SocialState::default())?,
        Probe::SocialLocal { mut state, command } => {
            social::validate(&state)?;
            social::local(&mut state, command)?;
            value(state)?
        }
        Probe::SocialPrepare {
            state,
            actor,
            action,
            now,
        } => {
            social::validate(&state)?;
            value(social::prepare(&state, &actor, action, now)?)?
        }
        Probe::Receipt { event, seed } => {
            let (request, presentation, policy) = receipts::proof(&event, seed);
            serde_json::json!({"request":request,"presentation":presentation,"policy":policy})
        }
        Probe::SocialApprove {
            mut state,
            event,
            request,
            presentation,
            policy,
            now,
        } => {
            social::validate(&state)?;
            let publication =
                social::approve(&mut state, event, request, presentation, &policy, now)?;
            serde_json::json!({"state":state,"publication":publication})
        }
        Probe::SocialReceive {
            mut state,
            publication,
            policy,
            now,
        } => {
            social::validate(&state)?;
            let changed = social::receive(&mut state, publication, &policy, now)?;
            serde_json::json!({"state":state,"changed":changed})
        }
    })
}
fn pairing_key(key: &[u8]) -> booknook_core::Result<&[u8; 32]> {
    key.try_into().map_err(|_| {
        booknook_core::Error::Invalid("Pairing key must contain exactly 32 bytes".into())
    })
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input)?;
    let result = run(booknook_core::strict_parse(&input)?)?;
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}
