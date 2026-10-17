// SPDX-License-Identifier: MIT OR Apache-2.0
//! Durable, explicitly approved encrypted file transport. Keys live in platform custody.
use crate::{Desk, NativeError, Result, content, now, portable, store::Store};
use base64::{Engine, engine::general_purpose::STANDARD};
use booknook_core::{
    authority::verify_receipt,
    canonical, digest,
    model::State,
    reconcile::{self, Conflict},
    strict_parse,
    sync::{self, ApprovedPacket as Packet, Batch},
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Peer {
    schema: String,
    local_device: String,
    device_id: String,
    owner: String,
    label: String,
    key_id: String,
    version: u64,
    enabled: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Content {
    schema: String,
    source_device: String,
    state: State,
    assets: BTreeMap<String, String>,
}
fn sql_number(n: u64) -> Result<i64> {
    i64::try_from(n).map_err(|_| invalid("Value exceeds SQLite integer range"))
}
fn invalid(s: &str) -> NativeError {
    booknook_core::Error::Invalid(s.into()).into()
}
fn schema(s: &Store) -> Result<()> {
    s.connection.execute_batch("CREATE TABLE IF NOT EXISTS sync_meta(id INTEGER PRIMARY KEY CHECK(id=1),device_id TEXT NOT NULL); CREATE TABLE IF NOT EXISTS sync_peers(id TEXT PRIMARY KEY,json TEXT NOT NULL); CREATE TABLE IF NOT EXISTS sync_prepared(id TEXT PRIMARY KEY,batch TEXT NOT NULL,content TEXT NOT NULL); CREATE TABLE IF NOT EXISTS sync_outbox(id TEXT PRIMARY KEY,peer TEXT NOT NULL,packet TEXT NOT NULL,exported INTEGER NOT NULL DEFAULT 0); CREATE TABLE IF NOT EXISTS sync_received(id TEXT PRIMARY KEY,digest TEXT NOT NULL); CREATE TABLE IF NOT EXISTS sync_bases(peer TEXT PRIMARY KEY,json TEXT NOT NULL); CREATE TABLE IF NOT EXISTS sync_conflicts(id TEXT PRIMARY KEY,json TEXT NOT NULL);")?;
    s.connection.execute(
        "INSERT OR IGNORE INTO sync_meta VALUES(1,?1)",
        [booknook_core::new_id()],
    )?;
    Ok(())
}
fn device(s: &Store) -> Result<String> {
    schema(s)?;
    Ok(s.connection
        .query_row("SELECT device_id FROM sync_meta WHERE id=1", [], |r| {
            r.get(0)
        })?)
}
fn peer(s: &Store, id: &str) -> Result<Peer> {
    let raw: String =
        s.connection
            .query_row("SELECT json FROM sync_peers WHERE id=?1", [id], |r| {
                r.get(0)
            })?;
    Ok(strict_parse(raw.as_bytes())?)
}
fn key(raw: &str) -> Result<[u8; 32]> {
    let bytes =
        hex::decode(raw).map_err(|_| invalid("Pairing key must be 64 hexadecimal characters"))?;
    bytes
        .try_into()
        .map_err(|_| invalid("Invalid pairing key length"))
}
fn check_peer(peer: &Peer, key: &[u8; 32], owner: &str) -> Result<()> {
    if !peer.enabled || peer.owner != owner || peer.key_id != digest(key) {
        return Err(booknook_core::Error::Denied(
            "Device is disabled, belongs to another identity, or has a different pairing key"
                .into(),
        )
        .into());
    }
    Ok(())
}
fn encoded<T: Serialize>(v: &T) -> Result<String> {
    let bytes = canonical(v)?;
    if bytes.len() > booknook_core::MAX_JSON_BYTES {
        return Err(invalid("Sync record exceeds 128 MiB"));
    }
    String::from_utf8(bytes).map_err(|_| invalid("Invalid JSON encoding"))
}
fn content(s: &Store) -> Result<Content> {
    let state = s.load()?;
    let mut assets = BTreeMap::new();
    let mut size = 0usize;
    for asset in state.editions.values().flat_map(|e| &e.assets) {
        if assets.contains_key(&asset.id) {
            continue;
        }
        let name = if asset.media_type == "application/pdf" {
            "book.pdf"
        } else {
            "original.epub"
        };
        let path = s.root.join("assets").join(&asset.storage_key).join(name);
        let len = std::fs::metadata(&path)?.len();
        size = size.saturating_add((len as usize).div_ceil(3) * 4);
        if size > 64 * 1024 * 1024 {
            return Err(invalid(
                "This encrypted snapshot exceeds the 64 MiB content limit",
            ));
        }
        let bytes = portable::read_bounded(&path)?;
        if digest(&bytes) != asset.sha256 {
            return Err(invalid("Asset checksum mismatch"));
        }
        assets.insert(asset.id.clone(), STANDARD.encode(bytes));
    }
    Ok(Content {
        schema: "booknook.sync-content.v1".into(),
        source_device: device(s)?,
        state,
        assets,
    })
}
fn import_assets(s: &Store, c: &Content) -> Result<()> {
    for asset in c.state.editions.values().flat_map(|e| &e.assets) {
        let bytes = STANDARD
            .decode(
                c.assets
                    .get(&asset.id)
                    .ok_or_else(|| invalid("Sync content is missing an asset"))?,
            )
            .map_err(|_| invalid("Invalid asset encoding"))?;
        if digest(&bytes) != asset.sha256 {
            return Err(invalid("Sync asset checksum mismatch"));
        }
        let actual = content::import(
            &s.root,
            &bytes,
            if asset.media_type == "application/pdf" {
                "pdf"
            } else {
                "epub"
            },
        )?;
        if actual != *asset {
            return Err(invalid("Sync asset metadata mismatch"));
        }
    }
    Ok(())
}
#[uniffi::export]
impl Desk {
    pub fn sync_status(&self) -> Result<String> {
        let s = self.lock()?;
        let id = device(&s)?;
        let read = |query: &str| -> Result<Vec<serde_json::Value>> {
            let mut st = s.connection.prepare(query)?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?;
            rows.map(|r| Ok(strict_parse(r?.as_bytes())?)).collect()
        };
        let peers = read("SELECT json FROM sync_peers ORDER BY id")?;
        let conflicts = read("SELECT json FROM sync_conflicts ORDER BY id")?;
        let mut st = s
            .connection
            .prepare("SELECT id,peer,exported FROM sync_outbox ORDER BY rowid DESC")?;
        let outbox=st.query_map([],|r|Ok(serde_json::json!({"id":r.get::<_,String>(0)?,"peer":r.get::<_,String>(1)?,"exported":r.get::<_,bool>(2)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        encoded(
            &serde_json::json!({"device_id":id,"peers":peers,"conflicts":conflicts,"outbox":outbox}),
        )
    }
    pub fn sync_new_pairing_key(&self) -> Result<String> {
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes).map_err(|_| invalid("Entropy unavailable"))?;
        Ok(hex::encode(bytes))
    }
    pub fn sync_prepare_peer(
        &self,
        device_id: String,
        label: String,
        key_hex: String,
        enabled: bool,
    ) -> Result<String> {
        let policy = self.policy("device.enroll", None)?;
        let key = key(&key_hex)?;
        let s = self.lock()?;
        let local = device(&s)?;
        if device_id.is_empty()
            || device_id.len() > 200
            || device_id == local
            || label.trim().is_empty()
            || label.len() > 200
        {
            return Err(invalid("Enter a different device ID and a label"));
        }
        let previous = peer(&s, &device_id).ok();
        let version = previous
            .as_ref()
            .map_or(Some(1), |p| p.version.checked_add(1))
            .ok_or_else(|| invalid("Device version exhausted"))?;
        if previous.as_ref().is_some_and(|p| p.key_id != digest(&key)) {
            let count: i64 = s.connection.query_row(
                "SELECT count(*) FROM sync_outbox WHERE peer=?1",
                [&device_id],
                |r| r.get(0),
            )?;
            if count > 0 {
                return Err(invalid(
                    "Remove or complete this device's pending outbox before changing its pairing key",
                ));
            }
        }
        encoded(&Peer {
            schema: "booknook.device-pairing.v1".into(),
            local_device: local,
            device_id,
            owner: policy.actor,
            label,
            key_id: digest(&key),
            version,
            enabled,
        })
    }
    pub fn sync_approve_peer(
        &self,
        peer_json: String,
        request_json: String,
        presentation_json: String,
    ) -> Result<()> {
        let policy = self.policy("device.enroll", None)?;
        let proposed: Peer = strict_parse(peer_json.as_bytes())?;
        let at = now()?;
        let receipt = verify_receipt(
            request_json.as_bytes(),
            presentation_json.as_bytes(),
            &policy,
            &canonical(&proposed)?,
            proposed.version,
            at,
        )?;
        let mut s = self.lock()?;
        let local = device(&s)?;
        if proposed.schema != "booknook.device-pairing.v1"
            || proposed.local_device != local
            || proposed.owner != policy.actor
            || proposed.device_id == local
            || proposed.device_id.is_empty()
            || proposed.label.trim().is_empty()
        {
            return Err(invalid("Pairing record does not match this device"));
        }
        let previous = peer(&s, &proposed.device_id).ok();
        let expected = previous
            .as_ref()
            .map_or(Some(1), |p| p.version.checked_add(1))
            .ok_or_else(|| invalid("Device version exhausted"))?;
        if expected != proposed.version {
            return Err(invalid("Device changed after review"));
        }
        if previous
            .as_ref()
            .is_some_and(|p| p.key_id != proposed.key_id)
        {
            let count: i64 = s.connection.query_row(
                "SELECT count(*) FROM sync_outbox WHERE peer=?1",
                [&proposed.device_id],
                |r| r.get(0),
            )?;
            if count > 0 {
                return Err(invalid(
                    "Pending encrypted outbox prevents changing this device's pairing key",
                ));
            }
        }
        let tx = s.connection.transaction()?;
        tx.execute(
            "INSERT INTO consumed_receipts VALUES(?1,?2)",
            params![receipt.replay_key(), sql_number(at)?],
        )?;
        tx.execute(
            "INSERT INTO sync_peers VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET json=excluded.json",
            params![proposed.device_id, encoded(&proposed)?],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn sync_prepare(&self, peer_id: String) -> Result<String> {
        let policy = self.policy("sync.send", None)?;
        let mut s = self.lock()?;
        crate::edit_journal::recover(&mut s)?;
        schema(&s)?;
        let peer = peer(&s, &peer_id)?;
        if !peer.enabled || peer.owner != policy.actor {
            return Err(invalid("Device is disabled or belongs to another identity"));
        }
        let c = content(&s)?;
        let plaintext = encoded(&c)?;
        if plaintext.len() > 64 * 1024 * 1024 {
            return Err(invalid(
                "This encrypted snapshot exceeds the 64 MiB content limit",
            ));
        }
        let batch = sync::prepare(
            &policy.actor,
            c.state.revision,
            vec![peer_id],
            plaintext.as_bytes(),
        )?;
        s.connection.execute(
            "INSERT INTO sync_prepared VALUES(?1,?2,?3)",
            params![batch.id, encoded(&batch)?, plaintext],
        )?;
        encoded(
            &serde_json::json!({"batch":batch,"books":c.state.books.len(),"notebooks":c.state.notebooks.len(),"cards":c.state.cards.len(),"annotations":c.state.annotations.len(),"assets":c.assets.len(),"bytes":plaintext.len()}),
        )
    }
    pub fn sync_approve(
        &self,
        batch_id: String,
        key_hex: String,
        request_json: String,
        presentation_json: String,
    ) -> Result<String> {
        let policy = self.policy("sync.send", None)?;
        let key = key(&key_hex)?;
        let at = now()?;
        let mut s = self.lock()?;
        schema(&s)?;
        let (raw, plain): (String, String) = s.connection.query_row(
            "SELECT batch,content FROM sync_prepared WHERE id=?1",
            [&batch_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let batch: Batch = strict_parse(raw.as_bytes())?;
        if batch.recipients.len() != 1 {
            return Err(invalid("Invalid sync audience"));
        }
        let peer = peer(&s, &batch.recipients[0])?;
        check_peer(&peer, &key, &policy.actor)?;
        let approved = sync::seal_packet(
            batch,
            plain.as_bytes(),
            &key,
            request_json,
            presentation_json,
            &policy,
            at,
        )?;
        let json = encoded(&approved.packet)?;
        let tx = s.connection.transaction()?;
        tx.execute(
            "INSERT INTO consumed_receipts VALUES(?1,?2)",
            params![approved.replay_key, sql_number(at)?],
        )?;
        tx.execute(
            "INSERT INTO sync_outbox(id,peer,packet) VALUES(?1,?2,?3)",
            params![batch_id, peer.device_id, json],
        )?;
        tx.execute("DELETE FROM sync_prepared WHERE id=?1", [&batch_id])?;
        tx.commit()?;
        Ok(batch_id)
    }
    pub fn sync_cancel_prepared(&self, batch_id: String) -> Result<()> {
        let s = self.lock()?;
        schema(&s)?;
        s.connection
            .execute("DELETE FROM sync_prepared WHERE id=?1", [batch_id])?;
        Ok(())
    }
    pub fn sync_export(&self, batch_id: String, path: String) -> Result<()> {
        let policy = self.policy("sync.send", None)?;
        let s = self.lock()?;
        schema(&s)?;
        let (peer_id, raw): (String, String) = s.connection.query_row(
            "SELECT peer,packet FROM sync_outbox WHERE id=?1",
            [&batch_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let peer = peer(&s, &peer_id)?;
        if !peer.enabled || peer.owner != policy.actor {
            return Err(invalid(
                "This device was disabled; queued exports are suspended",
            ));
        }
        let packet: Packet = strict_parse(raw.as_bytes())?;
        verify_receipt(
            packet.request_json.as_bytes(),
            packet.presentation_json.as_bytes(),
            &policy,
            &canonical(&packet.envelope.batch)?,
            packet.envelope.batch.revision,
            packet.approved_at,
        )?;
        portable::write_atomic(Path::new(&path), &packet, booknook_core::MAX_JSON_BYTES)?;
        s.connection
            .execute("UPDATE sync_outbox SET exported=1 WHERE id=?1", [batch_id])?;
        Ok(())
    }
    /// Removes only the retry copy after a human confirms delivery or cancels it.
    pub fn sync_remove_outbox(&self, batch_id: String) -> Result<()> {
        let s = self.lock()?;
        schema(&s)?;
        s.connection
            .execute("DELETE FROM sync_outbox WHERE id=?1", [batch_id])?;
        Ok(())
    }
    pub fn sync_receive(
        &self,
        path: String,
        source_device: String,
        key_hex: String,
    ) -> Result<String> {
        let policy = self.policy("sync.send", None)?;
        let key = key(&key_hex)?;
        let packet: Packet = strict_parse(&portable::read_bounded(Path::new(&path))?)?;
        sync::verify_packet(&packet, &policy, now()?)?;
        let batch = &packet.envelope.batch;
        let mut s = self.lock()?;
        crate::edit_journal::recover(&mut s)?;
        let local = device(&s)?;
        let peer = peer(&s, &source_device)?;
        check_peer(&peer, &key, &policy.actor)?;
        let opened = sync::open_packet(&packet, &key, &local, &policy, now()?)?;
        let c: Content = strict_parse(&opened.plaintext)?;
        if c.schema != "booknook.sync-content.v1"
            || c.source_device != source_device
            || c.state.revision != batch.revision
        {
            return Err(invalid("Sync source or revision mismatch"));
        }
        let packet_digest = opened.packet_digest;
        let previous: Option<String> = s
            .connection
            .query_row(
                "SELECT digest FROM sync_received WHERE id=?1",
                [&batch.id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(previous) = previous {
            if previous != packet_digest {
                return Err(invalid("Sync batch ID was reused"));
            }
            return encoded(&serde_json::json!({"duplicate":true,"conflicts":0,"state":s.load()?}));
        }
        let base: Option<String> = s
            .connection
            .query_row(
                "SELECT json FROM sync_bases WHERE peer=?1",
                [&source_device],
                |r| r.get(0),
            )
            .optional()?;
        let base: Option<State> = base.map(|raw| strict_parse(raw.as_bytes())).transpose()?;
        // Reordered older snapshots cannot replace the three-way merge baseline.
        if base.as_ref().is_some_and(|b| b.revision > c.state.revision) {
            return Err(invalid(
                "An older device snapshot arrived after a newer snapshot; request a current batch",
            ));
        }
        if base
            .as_ref()
            .is_some_and(|b| b.revision == c.state.revision && b != &c.state)
        {
            return Err(invalid(
                "Device reused a revision for different content; reconcile the source before sending another batch",
            ));
        }
        let current = s.load()?;
        let merged = reconcile::reconcile(base.as_ref(), &current, &c.state)?;
        import_assets(&s, &c)?;
        let json = encoded(&merged.state)?;
        let tx = s.connection.transaction()?;
        let count = tx.execute(
            "UPDATE desk SET revision=?1,json=?2 WHERE singleton=1 AND revision=?3",
            params![
                sql_number(merged.state.revision)?,
                json,
                sql_number(current.revision)?
            ],
        )?;
        if count != 1 {
            return Err(invalid("Local state changed during sync; retry"));
        }
        for conflict in &merged.conflicts {
            tx.execute(
                "INSERT OR IGNORE INTO sync_conflicts VALUES(?1,?2)",
                params![conflict.id, encoded(conflict)?],
            )?;
        }
        tx.execute(
            "INSERT INTO sync_received VALUES(?1,?2)",
            params![batch.id, packet_digest],
        )?;
        tx.execute("INSERT INTO sync_bases VALUES(?1,?2) ON CONFLICT(peer) DO UPDATE SET json=excluded.json",params![source_device,encoded(&c.state)?])?;
        tx.commit()?;
        encoded(
            &serde_json::json!({"duplicate":false,"conflicts":merged.conflicts.len(),"state":merged.state}),
        )
    }
    pub fn sync_resolve(&self, conflict_id: String, use_incoming: bool) -> Result<String> {
        let mut s = self.lock()?;
        schema(&s)?;
        let raw: String = s.connection.query_row(
            "SELECT json FROM sync_conflicts WHERE id=?1",
            [&conflict_id],
            |r| r.get(0),
        )?;
        let conflict: Conflict = strict_parse(raw.as_bytes())?;
        let state = s.load()?;
        let resolved = reconcile::resolve(&state, &conflict, use_incoming)?;
        let tx = s.connection.transaction()?;
        let count = tx.execute(
            "UPDATE desk SET revision=?1,json=?2 WHERE singleton=1 AND revision=?3",
            params![
                sql_number(resolved.revision)?,
                encoded(&resolved)?,
                sql_number(state.revision)?
            ],
        )?;
        if count != 1 {
            return Err(invalid("Local state changed during conflict review"));
        }
        tx.execute("DELETE FROM sync_conflicts WHERE id=?1", [conflict_id])?;
        tx.commit()?;
        encoded(&resolved)
    }
}
