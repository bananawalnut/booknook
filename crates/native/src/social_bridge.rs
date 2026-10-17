// SPDX-License-Identifier: MIT OR Apache-2.0
//! Native persistence and transport orchestration for the portable social protocol.
#[cfg(test)]
#[path = "social_relay_acceptance.rs"]
mod relay_acceptance;
use crate::{Desk, NativeError, Result, now};
use booknook_connectors::{
    KeyBinding, VerifiedBinding,
    nostr_transport::{self as wire, BindingDocument, SocialRelayTransport},
};
use booknook_core::{
    authority::AuthorityPolicy,
    canonical,
    portable_archive::{ArchiveBinding, BindingHistory, PendingEnvelope, SocialArchiveBundle},
    social_protocol::{self as social, *},
};
use nostr_sdk::prelude::*;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;
use std::collections::BTreeMap;

fn sql(value: u64) -> Result<i64> {
    i64::try_from(value)
        .map_err(|_| booknook_core::Error::Invalid("Version exceeds SQLite range".into()).into())
}
fn unsigned(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}
fn schema(c: &Connection) -> Result<()> {
    c.execute_batch("CREATE TABLE IF NOT EXISTS social_delivery_cursor (id INTEGER PRIMARY KEY CHECK(id=1),last_event TEXT NOT NULL); CREATE TABLE IF NOT EXISTS social_state (id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL,json TEXT NOT NULL); CREATE TABLE IF NOT EXISTS social_bindings (holder TEXT PRIMARY KEY,version INTEGER NOT NULL,minimum_version INTEGER NOT NULL,json TEXT NOT NULL); CREATE TABLE IF NOT EXISTS social_pending (id TEXT PRIMARY KEY,json TEXT NOT NULL,reason TEXT NOT NULL); CREATE TABLE IF NOT EXISTS social_binding_history (holder TEXT NOT NULL,version INTEGER NOT NULL,json TEXT NOT NULL,retired_at INTEGER,PRIMARY KEY(holder,version)); INSERT OR IGNORE INTO social_binding_history SELECT holder,version,json,CASE WHEN minimum_version>version THEN CAST(strftime('%s','now') AS INTEGER) ELSE NULL END FROM social_bindings;")?;
    c.execute(
        "INSERT OR IGNORE INTO social_state VALUES(1,0,?1)",
        [serde_json::to_string(&SocialState::default())?],
    )?;
    Ok(())
}
/// Rotate bounded scans through the entire outbox. Invalid or temporarily unavailable entries
/// advance the cursor too, so a permanent failure cannot starve unrelated approved work.
fn delivery_page<T>(entries: &BTreeMap<String, T>, after: Option<&str>) -> Vec<String> {
    entries
        .keys()
        .filter(|id| after.is_none_or(|cursor| id.as_str() > cursor))
        .chain(
            entries
                .keys()
                .filter(|id| after.is_some_and(|cursor| id.as_str() <= cursor)),
        )
        .take(200)
        .cloned()
        .collect()
}
fn delivery_cursor(c: &Connection) -> Result<Option<String>> {
    Ok(c.query_row(
        "SELECT last_event FROM social_delivery_cursor WHERE id=1",
        [],
        |r| r.get(0),
    )
    .optional()?)
}
fn advance_delivery_cursor(c: &Connection, id: &str) -> Result<()> {
    c.execute("INSERT INTO social_delivery_cursor VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET last_event=excluded.last_event",[id])?;
    Ok(())
}
fn load(c: &Connection) -> Result<SocialState> {
    schema(c)?;
    let raw: String = c.query_row("SELECT json FROM social_state WHERE id=1", [], |r| r.get(0))?;
    let state = booknook_core::strict_parse(raw.as_bytes())?;
    social::validate(&state)?;
    Ok(state)
}
fn save(c: &mut Connection, previous: u64, s: &SocialState) -> Result<()> {
    social::validate(s)?;
    let json = serde_json::to_string(s)?;
    if json.len() > booknook_core::MAX_JSON_BYTES {
        return Err(booknook_core::Error::Invalid(
            "Social store exceeds 128 MiB; export before continuing".into(),
        )
        .into());
    }
    let tx = c.transaction()?;
    let n = tx.execute(
        "UPDATE social_state SET revision=?1,json=?2 WHERE id=1 AND revision=?3",
        params![sql(s.revision)?, json, sql(previous)?],
    )?;
    if n != 1 {
        return Err(booknook_core::Error::Conflict(
            "Social store changed; retry with current state".into(),
        )
        .into());
    }
    tx.commit()?;
    Ok(())
}
fn keys(secret: &[u8]) -> Result<Keys> {
    let secret = SecretKey::from_slice(secret)
        .map_err(|_| booknook_core::Error::Invalid("Invalid Nostr custody key".into()))?;
    Ok(Keys::new(secret))
}
fn documents(c: &Connection) -> Result<Vec<(BindingDocument, u64)>> {
    schema(c)?;
    let mut q = c.prepare("SELECT json,minimum_version FROM social_bindings ORDER BY holder")?;
    let rows = q.query_map([], |r| Ok((r.get::<_, String>(0)?, unsigned(r, 1)?)))?;
    rows.map(|r| {
        let (raw, min) = r?;
        Ok((booknook_core::strict_parse(raw.as_bytes())?, min))
    })
    .collect()
}
fn history(c: &Connection) -> Result<Vec<BindingHistory>> {
    schema(c)?;
    let mut query =
        c.prepare("SELECT json,retired_at FROM social_binding_history ORDER BY holder,version")?;
    query
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?))
        })?
        .map(|row| {
            let (raw, retired) = row?;
            Ok(BindingHistory {
                document: booknook_core::strict_parse(raw.as_bytes())?,
                retired_at: retired
                    .map(|v| {
                        u64::try_from(v).map_err(|_| {
                            booknook_core::Error::Invalid("Invalid retirement timestamp".into())
                        })
                    })
                    .transpose()?,
            })
        })
        .collect()
}
fn retain_history(c: &Connection, doc: &BindingDocument) -> Result<()> {
    let raw = serde_json::to_string(doc)?;
    let prior: Option<String> = c
        .query_row(
            "SELECT json FROM social_binding_history WHERE holder=?1 AND version=?2",
            params![doc.binding.castalia_holder, sql(doc.binding.version)?],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(prior) = prior
        && canonical(&booknook_core::strict_parse::<BindingDocument>(
            prior.as_bytes(),
        )?)? != canonical(doc)?
    {
        return Err(
            booknook_core::Error::Conflict("Transport history version collision".into()).into(),
        );
    }
    let later:Option<i64>=c.query_row("SELECT MIN(json_extract(json,'$.approved_at')) FROM social_binding_history WHERE holder=?1 AND version>?2",params![doc.binding.castalia_holder,sql(doc.binding.version)?],|r|r.get(0))?;
    c.execute(
        "INSERT OR IGNORE INTO social_binding_history VALUES(?1,?2,?3,?4)",
        params![
            doc.binding.castalia_holder,
            sql(doc.binding.version)?,
            raw,
            later
        ],
    )?;
    c.execute("UPDATE social_binding_history SET retired_at=CASE WHEN retired_at IS NULL THEN ?1 ELSE MIN(retired_at,?1) END WHERE holder=?2 AND version<?3",params![sql(doc.approved_at)?,doc.binding.castalia_holder,sql(doc.binding.version)?])?;
    Ok(())
}
struct HistoricalIdentity {
    holder: String,
    key: String,
    approved_at: u64,
    retired_at: Option<u64>,
    binding: VerifiedBinding,
}
fn historical_sender<'a>(
    entries: &'a [HistoricalIdentity],
    event: &Event,
) -> Option<&'a HistoricalIdentity> {
    entries.iter().rev().find(|entry| {
        entry.key == event.pubkey.to_hex()
            && event.created_at.as_secs() >= entry.approved_at
            && entry
                .retired_at
                .is_none_or(|end| event.created_at.as_secs() < end)
    })
}
fn store_binding(
    c: &Connection,
    doc: &BindingDocument,
    policy: &AuthorityPolicy,
    at: u64,
) -> Result<()> {
    schema(c)?;
    let transaction = c.unchecked_transaction()?;
    let previous: Option<(u64, u64, String)> = c
        .query_row(
            "SELECT version,minimum_version,json FROM social_bindings WHERE holder=?1",
            [&doc.binding.castalia_holder],
            |r| Ok((unsigned(r, 0)?, unsigned(r, 1)?, r.get(2)?)),
        )
        .optional()?;
    let min = previous.as_ref().map_or(1, |(_, min, _)| *min);
    wire::verify_binding(doc, policy, min, at)?;
    if let Some((version, _, raw)) = &previous
        && (*version > doc.binding.version
            || (*version == doc.binding.version
                && booknook_core::canonical(&booknook_core::strict_parse::<BindingDocument>(
                    raw.as_bytes(),
                )?)? != canonical(doc)?))
    {
        return Err(booknook_core::Error::Conflict(
            "Transport binding rollback or reused version".into(),
        )
        .into());
    }
    c.execute("INSERT INTO social_bindings VALUES(?1,?2,?2,?3) ON CONFLICT(holder) DO UPDATE SET version=excluded.version,minimum_version=MAX(social_bindings.minimum_version,excluded.minimum_version),json=excluded.json",params![doc.binding.castalia_holder,sql(doc.binding.version)?,serde_json::to_string(doc)?])?;
    retain_history(c, doc)?;
    transaction.commit()?;
    Ok(())
}
#[uniffi::export]
impl Desk {
    pub fn social_snapshot(&self, reveal_spoilers: bool) -> Result<String> {
        let actor = self
            .policy(social::ACTION, None)
            .map(|p| p.actor)
            .unwrap_or_default();
        let store = self.lock()?;
        let state = load(&store.connection)?;
        let bindings=documents(&store.connection)?.into_iter().map(|(d,min)|json!({"holder":d.binding.castalia_holder,"nostr_public_key":d.binding.nostr_public_key,"version":d.binding.version,"minimum_version":min,"valid_until":d.binding.valid_until})).collect::<Vec<_>>();
        let pending: i64 =
            store
                .connection
                .query_row("SELECT count(*) FROM social_pending", [], |r| r.get(0))?;
        Ok(serde_json::to_string(
            &json!({"state":state,"actor":actor,"feed":social::feed(&state,&actor,reveal_spoilers),"bindings":bindings,"pending":pending}),
        )?)
    }
    pub fn social_prepare(&self, action_json: String) -> Result<String> {
        let who = self.policy(social::ACTION, None)?.actor;
        let action: SocialAction = booknook_core::strict_parse(action_json.as_bytes())?;
        let state = {
            let store = self.lock()?;
            load(&store.connection)?
        };
        let at = now()?;
        self.audit_social(&state, at)?;
        Ok(serde_json::to_string(&social::prepare(
            &state, &who, action, at,
        )?)?)
    }
    pub fn social_local(&self, command_json: String) -> Result<()> {
        let command: LocalSocialCommand = booknook_core::strict_parse(command_json.as_bytes())?;
        let mut store = self.lock()?;
        let mut state = load(&store.connection)?;
        let revision = state.revision;
        social::local(&mut state, command)?;
        save(&mut store.connection, revision, &state)
    }
    pub fn social_approve(
        &self,
        event_json: String,
        request_json: String,
        presentation_json: String,
    ) -> Result<()> {
        let event: SocialEvent = booknook_core::strict_parse(event_json.as_bytes())?;
        let original = {
            let store = self.lock()?;
            load(&store.connection)?
        };
        self.audit_social(&original, now()?)?;
        let policy = self.policy(social::ACTION, None)?;
        let mut store = self.lock()?;
        let mut state = load(&store.connection)?;
        if state.revision != original.revision {
            return Err(booknook_core::Error::Conflict(
                "Social state changed during verification; review again".into(),
            )
            .into());
        }
        let revision = state.revision;
        social::approve(
            &mut state,
            event,
            request_json,
            presentation_json,
            &policy,
            now()?,
        )?;
        save(&mut store.connection, revision, &state)
    }
    pub fn social_new_owned_id(&self) -> Result<String> {
        Ok(social::new_owned_id(
            &self.policy(social::ACTION, None)?.actor,
        )?)
    }
    /// Invoked only by the explicit native Generate transport key action. The result goes
    /// directly to Keychain, and never enters settings, SQLite, exported archives or logs.
    pub fn social_generate_transport_secret(&self) -> Vec<u8> {
        Keys::generate().secret_key().to_secret_bytes().to_vec()
    }
    pub fn social_prepare_binding(&self, secret: Vec<u8>) -> Result<String> {
        let secret = zeroize::Zeroizing::new(secret);
        let policy = self.policy("nostr.bind", None)?;
        let keys = keys(&secret)?;
        let at = now()?;
        let store = self.lock()?;
        schema(&store.connection)?;
        let previous: Option<u64> = store
            .connection
            .query_row(
                "SELECT MAX(version,minimum_version) FROM social_bindings WHERE holder=?1",
                [&policy.actor],
                |r| unsigned(r, 0),
            )
            .optional()?;
        let version = previous
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| booknook_core::Error::Invalid("Binding version overflow".into()))?;
        Ok(serde_json::to_string(&KeyBinding {
            schema: "booknook.nostr-binding.v1".into(),
            castalia_holder: policy.actor,
            nostr_public_key: keys.public_key().to_hex(),
            valid_from: at,
            valid_until: at + 365 * 86400,
            version,
        })?)
    }
    pub fn social_approve_binding(
        &self,
        binding_json: String,
        request_json: String,
        presentation_json: String,
    ) -> Result<String> {
        let binding: KeyBinding = booknook_core::strict_parse(binding_json.as_bytes())?;
        let policy = self.policy("nostr.bind", None)?;
        if binding.castalia_holder != policy.actor {
            return Err(booknook_core::Error::Denied(
                "Cannot bind another reader's transport key".into(),
            )
            .into());
        }
        let doc = BindingDocument {
            schema: "booknook.transport-identity.v1".into(),
            binding,
            approved_at: now()?,
            request_json,
            presentation_json,
        };
        let store = self.lock()?;
        store_binding(&store.connection, &doc, &policy, now()?)?;
        Ok(serde_json::to_string(&doc)?)
    }
    pub fn social_import_binding(&self, document_json: String) -> Result<()> {
        let doc: BindingDocument = booknook_core::strict_parse(document_json.as_bytes())?;
        let policy = self.policy("nostr.bind", Some(&doc.binding.castalia_holder))?;
        let store = self.lock()?;
        store_binding(&store.connection, &doc, &policy, now()?)
    }
    pub fn social_export_binding(&self, holder: String) -> Result<String> {
        let store = self.lock()?;
        schema(&store.connection)?;
        store
            .connection
            .query_row(
                "SELECT json FROM social_bindings WHERE holder=?1",
                [holder],
                |r| r.get(0),
            )
            .map_err(Into::into)
    }
    pub fn social_revoke_binding(&self, holder: String) -> Result<()> {
        let store = self.lock()?;
        schema(&store.connection)?;
        let tx = store.connection.unchecked_transaction()?;
        let n=tx.execute("UPDATE social_bindings SET minimum_version=MAX(version,minimum_version)+1 WHERE holder=?1",[&holder])?;
        if n == 0 {
            return Err(booknook_core::Error::NotFound("Transport identity".into()).into());
        }
        tx.execute("UPDATE social_binding_history SET retired_at=CASE WHEN retired_at IS NULL THEN ?1 ELSE MIN(retired_at,?1) END WHERE holder=?2",params![sql(now()?)?,holder])?;
        tx.commit()?;
        Ok(())
    }
    pub fn social_recommendations(
        &self,
        candidates_json: String,
        excluded_json: String,
    ) -> Result<String> {
        let candidates = booknook_core::strict_parse(candidates_json.as_bytes())?;
        let excluded = booknook_core::strict_parse(excluded_json.as_bytes())?;
        let state = {
            let store = self.lock()?;
            load(&store.connection)?
        };
        // Catalog interests are private, local functionality. Social evidence is optional
        // and must be reconstructed from valid receipts under the current independent pins.
        let verified_actor = self.policy(social::ACTION, None).ok().and_then(|policy| {
            now()
                .ok()
                .filter(|at| self.audit_social(&state, *at).is_ok())
                .map(|_| policy.actor)
        });
        let recommendations = if let Some(who) = verified_actor {
            social::recommendations(&state, &who, candidates, excluded)
        } else {
            booknook_core::providers::recommend(
                &booknook_core::providers::CatalogRecommendations,
                &booknook_core::social::RecommendationInput {
                    candidates,
                    preferred_subjects: state.preferred_subjects.into_iter().collect(),
                    shared_ratings: Vec::new(),
                    friends: Default::default(),
                    excluded,
                },
            )?
        };
        Ok(serde_json::to_string(&recommendations)?)
    }
    pub fn social_export(&self, path: String) -> Result<()> {
        let store = self.lock()?;
        let state = load(&store.connection)?;
        crate::portable::write_atomic(
            std::path::Path::new(&path),
            &state,
            booknook_core::MAX_JSON_BYTES,
        )
    }
    /// Restore local social history only into an empty social desk. Network authorization is
    /// still required on later exchange; this never imports custody secrets or trusted pins.
    pub fn social_restore(&self, path: String) -> Result<()> {
        let state: SocialState = booknook_core::strict_parse(&crate::portable::read_bounded(
            std::path::Path::new(&path),
        )?)?;
        social::validate(&state)?;
        let mut store = self.lock()?;
        let current = load(&store.connection)?;
        if current.revision != 0 {
            return Err(booknook_core::Error::Conflict(
                "Social restore requires an empty social desk".into(),
            )
            .into());
        }
        save(&mut store.connection, 0, &state)
    }
    /// Calls are run off the UI/repository actor by the Swift facade. No SQLite lock is held
    /// while awaiting relays. Every accepted event and acknowledgment is committed separately.
    /// Same approval and recipient checks as the built-in exchange; only transport is replaced.
    pub fn provider_social_exchange(
        &self,
        provider_id: String,
        secret: Vec<u8>,
        discover: bool,
    ) -> Result<String> {
        if provider_id == crate::provider_bridge::NOSTR {
            return self.social_exchange(secret, discover);
        }
        let generation = self.provider_lock()?.generation;
        let secret = zeroize::Zeroizing::new(secret);
        let keys = keys(&secret)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| NativeError::Failure {
                message: e.to_string(),
            })?;
        runtime.block_on(self.exchange_transport(
            ExchangeTransport::Provider {
                desk: self,
                provider_id,
                generation,
            },
            keys,
            discover,
        ))
    }
    pub async fn provider_social_recommendations(
        &self,
        provider_id: String,
        candidates_json: String,
        excluded_json: String,
    ) -> Result<String> {
        let candidates = booknook_core::strict_parse(candidates_json.as_bytes())?;
        let excluded = booknook_core::strict_parse(excluded_json.as_bytes())?;
        let state = {
            let store = self.lock()?;
            load(&store.connection)?
        };
        let actor = self.policy(social::ACTION, None).ok().and_then(|policy| {
            now()
                .ok()
                .filter(|at| self.audit_social(&state, *at).is_ok())
                .map(|_| policy.actor)
        });
        let input = if let Some(who) = actor {
            social::recommendation_input(&state, &who, candidates, excluded)
        } else {
            booknook_core::social::RecommendationInput {
                candidates,
                preferred_subjects: state.preferred_subjects.into_iter().collect(),
                shared_ratings: vec![],
                friends: Default::default(),
                excluded,
            }
        };
        let reply = self
            .fetch_provider_request(
                &provider_id,
                &booknook_core::provider_runtime::ProviderOperation::Recommendations { input },
            )
            .await?;
        let execution: booknook_core::provider_runtime::ProviderExecution =
            booknook_core::strict_parse(reply.response_json.as_bytes())?;
        let booknook_core::provider_runtime::ProviderResult::Recommendations { recommendations } =
            execution.response.result
        else {
            return Err(booknook_core::Error::Invalid("Expected recommendations".into()).into());
        };
        Ok(serde_json::to_string(&recommendations)?)
    }
    pub fn social_exchange(&self, secret: Vec<u8>, discover: bool) -> Result<String> {
        let config: booknook_core::settings::ConnectionSettings =
            booknook_core::strict_parse(self.connection_settings()?.as_bytes())?;
        let secret = zeroize::Zeroizing::new(secret);
        let keys = keys(&secret)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| NativeError::Failure {
                message: e.to_string(),
            })?;
        runtime.block_on(async {
            let transport = SocialRelayTransport::connect(&config.nostr_relays).await?;
            self.exchange_connected(transport, keys, discover).await
        })
    }
}
impl Desk {
    /// Prepared ciphertext is bound to the exact identity documents used at
    /// encryption time. Every dispatch repeats these checks after earlier IO;
    /// changing a key cannot silently retarget an already prepared envelope.
    fn validate_prepared_delivery(
        &self,
        publication: &ApprovedPublication,
        prepared_bindings: &[(BindingDocument, u64)],
        at: u64,
    ) -> Result<()> {
        let policy = self.policy(social::ACTION, None)?;
        if policy.actor != publication.event.actor {
            return Err(booknook_core::Error::Denied(
                "Current identity changed during delivery; refresh before retrying".into(),
            )
            .into());
        }
        let (current, bindings) = {
            let store = self.lock()?;
            (load(&store.connection)?, documents(&store.connection)?)
        };
        social::validate_outgoing(&current, publication, &policy, at)?;
        let mut required = std::collections::BTreeSet::from([publication.event.actor.as_str()]);
        if let Delivery::Direct { recipients } = &publication.event.delivery {
            required.extend(recipients.iter().map(String::as_str));
        }
        for holder in required {
            let previous = prepared_bindings
                .iter()
                .find(|(doc, _)| doc.binding.castalia_holder == holder)
                .ok_or_else(|| {
                    booknook_core::Error::Denied("Missing prepared identity binding".into())
                })?;
            let (document, minimum) = bindings
                .iter()
                .find(|(doc, _)| doc.binding.castalia_holder == holder)
                .ok_or_else(|| {
                    booknook_core::Error::Denied("Identity binding removed during delivery".into())
                })?;
            if canonical(document)? != canonical(&previous.0)? {
                return Err(booknook_core::Error::Denied(
                    "Identity binding changed during delivery; refresh before retrying".into(),
                )
                .into());
            }
            wire::verify_binding(
                document,
                &self.policy("nostr.bind", Some(holder))?,
                *minimum,
                at,
            )?;
        }
        Ok(())
    }
    async fn exchange_connected(
        &self,
        transport: SocialRelayTransport,
        keys: Keys,
        discover: bool,
    ) -> Result<String> {
        self.exchange_transport(
            ExchangeTransport::Native(Box::new(transport)),
            keys,
            discover,
        )
        .await
    }
    async fn exchange_transport(
        &self,
        transport: ExchangeTransport<'_>,
        keys: Keys,
        discover: bool,
    ) -> Result<String> {
        let actor = self.policy(social::ACTION, None)?.actor;
        let at = now()?;
        let initial = {
            let store = self.lock()?;
            load(&store.connection)?
        };
        self.audit_social(&initial, at)?;
        let mut messages = Vec::<String>::new();
        let mut sent = 0;
        let mut received = 0;
        let docs = {
            let s = self.lock()?;
            documents(&s.connection)?
        };
        let mut bindings = BTreeMap::<String, VerifiedBinding>::new();
        for (doc, min) in &docs {
            let policy = self.policy("nostr.bind", Some(&doc.binding.castalia_holder))?;
            match wire::verify_binding(doc, &policy, *min, at) {
                Ok(v) => {
                    bindings.insert(doc.binding.castalia_holder.clone(), v);
                }
                Err(e) => messages.push(e.to_string()),
            }
        }
        let own = bindings.get(&actor).ok_or_else(|| {
            booknook_core::Error::Unavailable(
                "Bind your Nostr key with Castalia before connecting".into(),
            )
        })?;
        if let Some((doc, _)) = docs
            .iter()
            .find(|(d, _)| d.binding.castalia_holder == actor)
            && let Err(e) = transport
                .publish_binding(doc, &self.policy("nostr.bind", None)?, &keys, at)
                .await
        {
            messages.push(e.to_string());
        }
        let (snapshot, cursor) = {
            let s = self.lock()?;
            (load(&s.connection)?, delivery_cursor(&s.connection)?)
        };
        let mut attempted = 0;
        for id in delivery_page(&snapshot.outbox, cursor.as_deref()) {
            if attempted >= 50 {
                break;
            }
            {
                let store = self.lock()?;
                advance_delivery_cursor(&store.connection, &id)?;
            }
            let item = &snapshot.outbox[&id];
            let policy = self.policy(social::ACTION, Some(&item.publication.event.actor))?;
            match wire::encode_publication(
                &snapshot,
                &item.publication,
                &policy,
                own,
                &bindings,
                &keys,
                at,
            ) {
                Ok(deliveries) => {
                    attempted += 1;
                    for delivery in deliveries {
                        if item
                            .acknowledged
                            .get(delivery.recipient())
                            .is_some_and(|r| !r.is_empty())
                        {
                            continue;
                        }
                        if let Err(error) =
                            self.validate_prepared_delivery(&item.publication, &docs, now()?)
                        {
                            messages.push(error.to_string());
                            break;
                        }
                        match transport.publish(&delivery).await {
                            Ok(relays) => {
                                let mut store = self.lock()?;
                                let mut current = load(&store.connection)?;
                                // ACK is a fact about a send already dispatched, not
                                // permission for another send. Preserve it even if
                                // trust changed in flight, but never resurrect a
                                // cancelled/replaced item or attach it to new content.
                                sent += 1;
                                if !current
                                    .outbox
                                    .get(&item.publication.event.id)
                                    .is_some_and(|entry| entry.publication == item.publication)
                                {
                                    messages.push("Relay acknowledged a delivery cancelled or replaced while in flight; no outbox item was recreated.".into());
                                    break;
                                }
                                let rev = current.revision;
                                social::mark_delivered(
                                    &mut current,
                                    &item.publication.event.id,
                                    delivery.recipient(),
                                    relays,
                                )?;
                                save(&mut store.connection, rev, &current)?;
                            }
                            Err(e) => messages.push(e.to_string()),
                        }
                    }
                }
                Err(e) => messages.push(e.to_string()),
            }
        }
        let authors = if discover {
            Vec::new()
        } else {
            docs.iter()
                .filter(|(d, _)| bindings.contains_key(&d.binding.castalia_holder))
                .filter_map(|(d, _)| PublicKey::from_hex(&d.binding.nostr_public_key).ok())
                .collect()
        };
        let mut fetched = Vec::new();
        let mut before = None;
        let mut seen = std::collections::BTreeSet::new();
        for page in 0..8 {
            let batch = match transport.fetch_page(authors.clone(), None, 0, before).await {
                Ok(v) => v,
                Err(e) => {
                    messages.push(e.to_string());
                    break;
                }
            };
            let full = batch.len() >= 500;
            let oldest = batch.iter().map(|e| e.created_at.as_secs()).min();
            let old_count = fetched.len();
            for event in batch {
                if seen.insert(event.id) {
                    fetched.push(event);
                }
            }
            if !full || oldest.is_none() {
                break;
            }
            if fetched.len() == old_count {
                messages.push("Relay history pagination stopped at a timestamp boundary; use a narrower relay or import missing history.".into());
                break;
            }
            before = oldest;
            if page == 7 {
                messages.push("Reached 4,000-event history budget. Some older group history may require a narrower relay or portable import.".into());
            }
        }
        transport.disconnect().await;
        fetched.sort_by_key(|e| e.created_at);
        for event in fetched.iter().rev() {
            if event
                .tags
                .identifier()
                .is_some_and(|v| v.starts_with("booknook:identity:"))
                && let Ok(doc) =
                    booknook_core::strict_parse::<BindingDocument>(event.content.as_bytes())
            {
                let policy = self.policy("nostr.bind", Some(&doc.binding.castalia_holder))?;
                let store = self.lock()?;
                let minimum: Option<u64> = store
                    .connection
                    .query_row(
                        "SELECT minimum_version FROM social_bindings WHERE holder=?1",
                        [&doc.binding.castalia_holder],
                        |r| unsigned(r, 0),
                    )
                    .optional()?;
                match wire::decode_binding(event, &policy, minimum.unwrap_or(1), at) {
                    Ok(doc) => {
                        if let Err(e) = store_binding(&store.connection, &doc, &policy, at) {
                            messages.push(e.to_string());
                        }
                    }
                    Err(_) => {
                        let version: Option<u64> = store
                            .connection
                            .query_row(
                                "SELECT version FROM social_bindings WHERE holder=?1",
                                [&doc.binding.castalia_holder],
                                |r| unsigned(r, 0),
                            )
                            .optional()?;
                        if version.is_some_and(|v| v > doc.binding.version)
                            && doc.approved_at <= at
                            && wire::decode_binding(event, &policy, 1, doc.approved_at).is_ok()
                        {
                            retain_history(&store.connection, &doc)?;
                        }
                    }
                }
            }
        }
        let docs = {
            let store = self.lock()?;
            documents(&store.connection)?
        };
        let mut bound_by_key = BTreeMap::new();
        for (doc, min) in &docs {
            let policy = self.policy("nostr.bind", Some(&doc.binding.castalia_holder))?;
            if let Ok(binding) = wire::verify_binding(doc, &policy, *min, at) {
                bound_by_key.insert(
                    doc.binding.nostr_public_key.clone(),
                    (doc.binding.castalia_holder.clone(), binding),
                );
            }
        }
        let recipient = bound_by_key
            .get(&keys.public_key().to_hex())
            .filter(|(who, _)| who == &actor);
        let historical = {
            let store = self.lock()?;
            history(&store.connection)?
        };
        let mut senders = Vec::new();
        for entry in historical {
            let doc = entry.document;
            let policy = self.policy("nostr.bind", Some(&doc.binding.castalia_holder))?;
            if let Ok(binding) = wire::verify_binding(&doc, &policy, 1, doc.approved_at) {
                senders.push(HistoricalIdentity {
                    holder: doc.binding.castalia_holder,
                    key: doc.binding.nostr_public_key,
                    approved_at: doc.approved_at,
                    retired_at: entry.retired_at,
                    binding,
                });
            }
        }
        for event in fetched {
            if event.tags.identifier().is_none_or(|v| {
                !v.starts_with("booknook:social:") && !v.starts_with("booknook:private:")
            }) || event.content.len() > 100_000
            {
                continue;
            }
            if event
                .tags
                .identifier()
                .is_some_and(|v| v.starts_with("booknook:private:"))
                && !event.tags.public_keys().any(|p| p == &keys.public_key())
            {
                continue;
            }
            let Some(sender) = historical_sender(&senders, &event) else {
                continue;
            };
            let who = &sender.holder;
            let binding = &sender.binding;
            let policy = self.policy(social::ACTION, Some(who))?;
            let publication =
                match wire::decode_event(&event, binding, recipient.map(|(_, b)| (b, &keys)), at) {
                    Ok(p) => p,
                    Err(_) => continue,
                };
            if social::verify_publication(&publication, &policy, at).is_err() {
                continue;
            }
            let store = self.lock()?;
            let current = load(&store.connection)?;
            if current.blocked.contains(who)
                || current.publications.get(&publication.event.id) == Some(&publication)
            {
                continue;
            }
            let count: i64 =
                store
                    .connection
                    .query_row("SELECT count(*) FROM social_pending", [], |r| r.get(0))?;
            if count >= 1000 {
                messages.push(
                    "Pending verified-event limit reached; retry after resolving missing history"
                        .into(),
                );
                break;
            }
            store.connection.execute("INSERT OR IGNORE INTO social_pending VALUES(?1,?2,'Verified approval; awaiting dependencies')",params![event.id.to_hex(),event.as_json()])?;
        }
        // Multiple passes resolve same-timestamp dependency ordering. Unresolved history
        // remains durable, with a reason, so relay outages never discard a group update.
        for _ in 0..8 {
            let pending: Vec<(String, String)> = {
                let store = self.lock()?;
                let mut q = store
                    .connection
                    .prepare("SELECT id,json FROM social_pending")?;
                q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<std::result::Result<_, _>>()?
            };
            let events: Vec<_> = pending
                .into_iter()
                .filter_map(|(id, raw)| Event::from_json(raw).ok().map(|e| (id, e)))
                .collect();
            let mut decoded = Vec::new();
            for (id, event) in events {
                let Some(sender) = historical_sender(&senders, &event) else {
                    let store = self.lock()?;
                    store
                        .connection
                        .execute("DELETE FROM social_pending WHERE id=?1", [id])?;
                    continue;
                };
                let who = &sender.holder;
                let binding = &sender.binding;
                let policy = self.policy(social::ACTION, Some(who))?;
                match wire::decode_event(&event, binding, recipient.map(|(_, b)| (b, &keys)), at) {
                    Ok(publication)
                        if social::verify_publication(&publication, &policy, at).is_ok() =>
                    {
                        decoded.push((id, publication, policy))
                    }
                    _ => {
                        self.lock()?
                            .connection
                            .execute("DELETE FROM social_pending WHERE id=?1", [id])?;
                    }
                }
            }
            let order: BTreeMap<_, _> =
                social::causal_replay_order(decoded.iter().map(|(_, p, _)| p))
                    .into_iter()
                    .enumerate()
                    .map(|(rank, p)| (p.event.id.clone(), rank))
                    .collect();
            decoded.sort_by_key(|(_, p, _)| order[&p.event.id]);
            let mut progress = false;
            for (id, publication, policy) in decoded {
                let mut store = self.lock()?;
                let mut current = load(&store.connection)?;
                let rev = current.revision;
                match social::receive(&mut current, publication, &policy, at) {
                    Ok(changed) => {
                        if changed {
                            save(&mut store.connection, rev, &current)?;
                            received += 1;
                        }
                        store
                            .connection
                            .execute("DELETE FROM social_pending WHERE id=?1", [id])?;
                        progress = true;
                    }
                    Err(
                        e @ booknook_core::Error::NotFound(_)
                        | e @ booknook_core::Error::Conflict(_),
                    ) => {
                        store.connection.execute(
                            "UPDATE social_pending SET reason=?1 WHERE id=?2",
                            params![e.to_string(), id],
                        )?;
                    }
                    Err(_) => {
                        store
                            .connection
                            .execute("DELETE FROM social_pending WHERE id=?1", [id])?;
                    }
                }
            }
            if !progress {
                break;
            }
        }
        Ok(serde_json::to_string(
            &json!({"sent":sent,"received":received,"messages":messages}),
        )?)
    }
}

pub(crate) fn export_bundle(connection: &Connection) -> Result<serde_json::Value> {
    let state = load(connection)?;
    let bindings = documents(connection)?
        .into_iter()
        .map(|(document, minimum_version)| ArchiveBinding {
            document,
            minimum_version,
        })
        .collect();
    let mut query = connection.prepare("SELECT id,json,reason FROM social_pending ORDER BY id")?;
    let pending = query
        .query_map([], |r| {
            Ok(PendingEnvelope {
                id: r.get(0)?,
                event_json: r.get(1)?,
                reason: r.get(2)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()?;
    let value = serde_json::to_value(SocialArchiveBundle {
        schema: "booknook.social-archive.v1".into(),
        state,
        bindings,
        pending,
        binding_history: history(connection)?,
    })?;
    validate_archive_bundle(&value)?;
    Ok(value)
}
pub(crate) fn validate_archive_bundle(value: &serde_json::Value) -> Result<()> {
    booknook_core::portable_archive::validate_social_bundle(value).map_err(Into::into)
}
/// Called inside the same transaction that restores the library. Imported proofs remain
/// data: exchange re-verifies issuer pins, holder signatures and derived social projections.
pub(crate) fn write_archive_bundle(
    tx: &rusqlite::Transaction<'_>,
    value: &serde_json::Value,
) -> Result<()> {
    validate_archive_bundle(value)?;
    schema(tx)?;
    let current = load(tx)?;
    let identities: i64 = tx.query_row("SELECT count(*) FROM social_bindings", [], |r| r.get(0))?;
    let pending: i64 = tx.query_row("SELECT count(*) FROM social_pending", [], |r| r.get(0))?;
    let historical: i64 = tx.query_row("SELECT count(*) FROM social_binding_history", [], |r| {
        r.get(0)
    })?;
    if current.revision != 0 || identities != 0 || pending != 0 || historical != 0 {
        return Err(booknook_core::Error::Conflict(
            "Archive restore requires an empty social desk and identity directory".into(),
        )
        .into());
    }
    let bundle: SocialArchiveBundle = booknook_core::strict_parse(&canonical(value)?)?;
    tx.execute(
        "UPDATE social_state SET revision=?1,json=?2 WHERE id=1",
        params![
            sql(bundle.state.revision)?,
            serde_json::to_string(&bundle.state)?
        ],
    )?;
    for b in bundle.bindings {
        tx.execute(
            "INSERT INTO social_bindings VALUES(?1,?2,?3,?4)",
            params![
                b.document.binding.castalia_holder,
                sql(b.document.binding.version)?,
                sql(b.minimum_version)?,
                serde_json::to_string(&b.document)?
            ],
        )?;
    }
    for h in bundle.binding_history {
        tx.execute(
            "INSERT INTO social_binding_history VALUES(?1,?2,?3,?4)",
            params![
                h.document.binding.castalia_holder,
                sql(h.document.binding.version)?,
                serde_json::to_string(&h.document)?,
                h.retired_at.map(sql).transpose()?
            ],
        )?;
    }
    for p in bundle.pending {
        tx.execute(
            "INSERT INTO social_pending VALUES(?1,?2,?3)",
            params![p.id, p.event_json, p.reason],
        )?;
    }
    Ok(())
}
impl Desk {
    /// Imported snapshots do not supply authority. Rebuild every shared projection from the
    /// independently verified publication journal before performing new network operations.
    fn audit_social(&self, state: &SocialState, at: u64) -> Result<()> {
        social::audit(
            state,
            |who| {
                self.policy(social::ACTION, Some(who))
                    .map_err(|error| booknook_core::Error::Denied(error.to_string()))
            },
            at,
        )?;
        Ok(())
    }
}

/// Exchange protocol owns signing, encryption, receipts and acceptance. This enum owns IO only.
enum ExchangeTransport<'a> {
    Native(Box<SocialRelayTransport>),
    Provider {
        desk: &'a Desk,
        provider_id: String,
        generation: u64,
    },
}
impl ExchangeTransport<'_> {
    async fn execute(
        &self,
        operation: booknook_core::provider_runtime::ProviderOperation,
    ) -> Result<booknook_core::provider_runtime::ProviderResult> {
        let Self::Provider {
            desk,
            provider_id,
            generation,
        } = self
        else {
            return Err(booknook_core::Error::Invalid("Expected injected transport".into()).into());
        };
        if desk.provider_lock()?.generation != *generation {
            return Err(booknook_core::Error::Conflict(
                "Social provider changed during exchange".into(),
            )
            .into());
        }
        let reply = desk.fetch_provider_request(provider_id, &operation).await?;
        if desk.provider_lock()?.generation != *generation {
            return Err(booknook_core::Error::Conflict(
                "Social provider changed during exchange".into(),
            )
            .into());
        }
        let execution: booknook_core::provider_runtime::ProviderExecution =
            booknook_core::strict_parse(reply.response_json.as_bytes())?;
        Ok(execution.response.result)
    }
    async fn publish(
        &self,
        delivery: &wire::PreparedDelivery,
    ) -> Result<std::collections::BTreeSet<String>> {
        if let Self::Native(t) = self {
            return Ok(t.publish(delivery).await?);
        }
        self.publish_event(delivery.event()).await
    }
    async fn publish_event(&self, event: &Event) -> Result<std::collections::BTreeSet<String>> {
        let result = self
            .execute(
                booknook_core::provider_runtime::ProviderOperation::SocialPublish {
                    event: serde_json::to_value(event)?,
                },
            )
            .await?;
        let booknook_core::provider_runtime::ProviderResult::SocialPublish { relays, .. } = result
        else {
            return Err(booknook_core::Error::Invalid(
                "Expected signed event acknowledgment".into(),
            )
            .into());
        };
        Ok(relays.into_iter().collect())
    }
    async fn publish_binding(
        &self,
        doc: &BindingDocument,
        policy: &AuthorityPolicy,
        keys: &Keys,
        at: u64,
    ) -> Result<std::collections::BTreeSet<String>> {
        if let Self::Native(t) = self {
            return Ok(t.publish_binding(doc, policy, keys, at).await?);
        }
        self.publish_event(&wire::encode_binding(doc, policy, keys, at)?)
            .await
    }
    async fn fetch_page(
        &self,
        authors: Vec<PublicKey>,
        recipient: Option<PublicKey>,
        since: u64,
        before: Option<u64>,
    ) -> Result<Vec<Event>> {
        if let Self::Native(t) = self {
            return Ok(t.fetch_page(authors, recipient, since, before).await?);
        }
        let result = self
            .execute(
                booknook_core::provider_runtime::ProviderOperation::SocialFetch {
                    authors: authors.into_iter().map(|k| k.to_hex()).collect(),
                    recipient: recipient.map(|k| k.to_hex()),
                    since,
                    before,
                    limit: 500,
                },
            )
            .await?;
        let booknook_core::provider_runtime::ProviderResult::SocialFetch { events } = result else {
            return Err(booknook_core::Error::Invalid("Expected transport events".into()).into());
        };
        events
            .into_iter()
            .map(|e| {
                Event::from_json(serde_json::to_vec(&e)?).map_err(|_| {
                    booknook_core::Error::Invalid("Malformed signed event".into()).into()
                })
            })
            .collect()
    }
    async fn disconnect(&self) {
        if let Self::Native(t) = self {
            t.disconnect().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(dead_code)]
    mod synthetic {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/social_receipts.rs"
        ));
    }
    #[test]
    fn reopened_desk_audits_same_second_creation_before_invitation() {
        use booknook_core::settings::{CastaliaSettings, ConnectionSettings};
        let path =
            std::env::temp_dir().join(format!("booknook-causal-{}", booknook_core::new_id()));
        let desk = Desk::new(path.to_string_lossy().into()).unwrap();
        let (_, _, policy) = synthetic::receipt(b"synthetic configuration", ACTION, 0, 100, 2);
        let pin = &policy.pins[0];
        let settings = ConnectionSettings {
            castalia: Some(CastaliaSettings {
                issuer: pin.issuer.clone(),
                key_id: pin.key_id.clone(),
                issuer_public_key: pin.public_key.clone(),
                audience: pin.audience.clone(),
                terminal_caller: policy.expected_caller.id.clone(),
                holder_public_key: policy.actor.clone(),
                policy_digest: policy.policy_digest.clone(),
                credential_endpoint: "http://127.0.0.1:65500/credential".into(),
            }),
            ..Default::default()
        };
        desk.save_connection_settings(serde_json::to_string(&settings).unwrap())
            .unwrap();
        let mut state = SocialState::default();
        let id = format!("{}:same-second", synthetic::actor(2));
        let create = SocialAction::CreateGroup {
            id: id.clone(),
            name: "Synthetic readers".into(),
            description: String::new(),
            private: true,
        };
        let mut event = social::prepare(&state, &policy.actor, create, 100).unwrap();
        event.id = "z-create".into();
        let (r, p, _) = synthetic::proof(&event, 2);
        social::approve(&mut state, event, r, p, &policy, 100).unwrap();
        let invite = SocialAction::Invite {
            id: "invitation".into(),
            group: state.groups[&id].clone(),
            invitee: synthetic::actor(3),
            expires_at: 200,
        };
        let mut event = social::prepare(&state, &policy.actor, invite, 100).unwrap();
        event.id = "a-invite".into();
        let (r, p, _) = synthetic::proof(&event, 2);
        social::approve(&mut state, event, r, p, &policy, 100).unwrap();
        {
            let mut store = desk.lock().unwrap();
            assert_eq!(load(&store.connection).unwrap().revision, 0);
            save(&mut store.connection, 0, &state).unwrap();
        }
        drop(desk);
        let desk = Desk::new(path.to_string_lossy().into()).unwrap();
        desk.audit_social(&state, 100).unwrap();
        // The public native prepare entry point audits durable history before authorizing
        // another operation. Equal timestamps and reverse IDs must not strand this desk.
        let next = SocialAction::Invite {
            id: "next-invitation".into(),
            group: state.groups[&id].clone(),
            invitee: synthetic::actor(4),
            expires_at: now().unwrap() + 100,
        };
        desk.social_prepare(serde_json::to_string(&next).unwrap())
            .unwrap();
        drop(desk);
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn bounded_outbox_scans_advance_past_failures_and_wrap_after_removal() {
        let entries: BTreeMap<String, ()> =
            (0..601).map(|i| (format!("event-{i:04}"), ())).collect();
        let c = Connection::open_in_memory().unwrap();
        schema(&c).unwrap();
        let mut visited = std::collections::BTreeSet::new();
        // Simulate three pages of invalid bindings. Those entries still advance the cursor.
        for _ in 0..3 {
            let cursor = delivery_cursor(&c).unwrap();
            let page = delivery_page(&entries, cursor.as_deref());
            assert_eq!(page.len(), 200);
            for id in page {
                visited.insert(id.clone());
                advance_delivery_cursor(&c, &id).unwrap();
            }
        }
        let cursor = delivery_cursor(&c).unwrap();
        let page = delivery_page(&entries, cursor.as_deref());
        assert_eq!(page[0], "event-0600");
        assert_eq!(page[1], "event-0000");
        assert_eq!(visited.len(), 600);
        // A full attempt batch with relay failures rotates to the next fifty on refresh.
        advance_delivery_cursor(&c, "event-0049").unwrap();
        assert_eq!(
            delivery_page(&entries, delivery_cursor(&c).unwrap().as_deref())[0],
            "event-0050"
        );
        // The cursor remains useful after the corresponding retry copy is cancelled.
        let mut removed = entries.clone();
        removed.remove("event-0049");
        assert_eq!(delivery_page(&removed, Some("event-0049"))[0], "event-0050");
    }
    fn approved_binding(keys: &Keys, version: u64, at: u64) -> (BindingDocument, AuthorityPolicy) {
        let binding = KeyBinding {
            schema: "booknook.nostr-binding.v1".into(),
            castalia_holder: synthetic::actor(2),
            nostr_public_key: keys.public_key().to_hex(),
            valid_from: at,
            valid_until: at + 1000,
            version,
        };
        let (r, p, policy) =
            synthetic::receipt(&canonical(&binding).unwrap(), "nostr.bind", version, at, 2);
        (
            BindingDocument {
                schema: "booknook.transport-identity.v1".into(),
                binding,
                approved_at: at,
                request_json: r,
                presentation_json: p,
            },
            policy,
        )
    }
    #[test]
    fn renewed_binding_retains_historical_evidence_and_rejects_later_old_key_use() {
        let connection = Connection::open_in_memory().unwrap();
        let old = Keys::parse(&"03".repeat(32)).unwrap();
        let new = Keys::parse(&"04".repeat(32)).unwrap();
        let (first, policy) = approved_binding(&old, 1, 100);
        store_binding(&connection, &first, &policy, 100).unwrap();
        let (second, policy) = approved_binding(&new, 2, 150);
        store_binding(&connection, &second, &policy, 150).unwrap();
        let history = history(&connection).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].retired_at, Some(150));
        let senders: Vec<_> = history
            .into_iter()
            .map(|h| HistoricalIdentity {
                holder: h.document.binding.castalia_holder.clone(),
                key: h.document.binding.nostr_public_key.clone(),
                approved_at: h.document.approved_at,
                retired_at: h.retired_at,
                binding: wire::verify_binding(&h.document, &policy, 1, h.document.approved_at)
                    .unwrap(),
            })
            .collect();
        let event = |at| {
            EventBuilder::new(Kind::Custom(30078), "Synthetic")
                .custom_created_at(Timestamp::from(at))
                .sign_with_keys(&old)
                .unwrap()
        };
        assert!(historical_sender(&senders, &event(110)).is_some());
        assert!(historical_sender(&senders, &event(151)).is_none());
        let archive = export_bundle(&connection).unwrap();
        let mut target = Connection::open_in_memory().unwrap();
        let transaction = target.transaction().unwrap();
        write_archive_bundle(&transaction, &archive).unwrap();
        transaction.commit().unwrap();
        assert_eq!(export_bundle(&target).unwrap(), archive);
    }
}
