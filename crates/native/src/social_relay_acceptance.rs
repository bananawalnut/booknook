// SPDX-License-Identifier: MIT OR Apache-2.0
//! Opt-in interoperability against an independently built nostr-rs-relay process.
//! Only this test maps a logical WSS endpoint to real plaintext loopback WebSockets.
use super::*;
use booknook_core::settings::{CastaliaSettings, ConnectionSettings};
use nostr_sdk::pool::transport::{
    error::TransportError,
    websocket::{DefaultWebsocketTransport, WebSocketSink, WebSocketStream, WebSocketTransport},
};
use std::{
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};
mod synthetic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/social_receipts.rs"
    ));
}

#[derive(Debug)]
struct LoopbackSocket {
    port: u16,
}
impl WebSocketTransport for LoopbackSocket {
    fn support_ping(&self) -> bool {
        true
    }
    fn connect<'a>(
        &'a self,
        url: &'a Url,
        mode: &'a ConnectionMode,
        timeout: Duration,
    ) -> nostr_sdk::util::BoxedFuture<
        'a,
        std::result::Result<(WebSocketSink, WebSocketStream), TransportError>,
    > {
        Box::pin(async move {
            assert_eq!(url.scheme(), "wss");
            assert_eq!(url.host_str(), Some("127.0.0.1"));
            assert_eq!(url.port(), Some(self.port));
            let mut local = url.clone();
            local.set_scheme("ws").unwrap();
            DefaultWebsocketTransport
                .connect(&local, mode, timeout)
                .await
        })
    }
}
struct RelayProcess {
    child: Option<Child>,
    root: PathBuf,
    binary: PathBuf,
    port: u16,
}
impl RelayProcess {
    fn new(binary: PathBuf) -> Self {
        let root =
            std::env::temp_dir().join(format!("booknook-real-relay-{}", booknook_core::new_id()));
        std::fs::create_dir_all(root.join("relay-db")).unwrap();
        let socket = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        drop(socket);
        std::fs::write(root.join("config.toml"), format!("[network]\naddress=\"127.0.0.1\"\nport={port}\n[verified_users]\nmode=\"disabled\"\n[limits]\nmax_event_bytes=131072\nmax_ws_message_bytes=131072\n")).unwrap();
        Self {
            child: None,
            root,
            binary,
            port,
        }
    }
    async fn start(&mut self) {
        assert!(self.child.is_none());
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.root.join("relay.log"))
            .unwrap();
        self.child = Some(
            Command::new(&self.binary)
                .arg("--config")
                .arg(self.root.join("config.toml"))
                .arg("--db")
                .arg(self.root.join("relay-db"))
                .current_dir(&self.root)
                .env("RUST_LOG", "warn")
                .stdout(Stdio::from(log.try_clone().unwrap()))
                .stderr(Stdio::from(log))
                .spawn()
                .unwrap(),
        );
        for _ in 0..100 {
            if std::net::TcpStream::connect(("127.0.0.1", self.port)).is_ok() {
                return;
            }
            if let Some(exit) = self.child.as_mut().unwrap().try_wait().unwrap() {
                panic!(
                    "Independent relay exited {exit}: {}",
                    std::fs::read_to_string(self.root.join("relay.log")).unwrap()
                );
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("Independent relay did not start");
    }
    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            child.kill().unwrap();
            child.wait().unwrap();
        }
    }
    fn url(&self) -> String {
        format!("wss://127.0.0.1:{}", self.port)
    }
    async fn transport(&self) -> SocialRelayTransport {
        SocialRelayTransport::connect_with_transport(
            &[self.url()],
            LoopbackSocket { port: self.port },
        )
        .await
        .unwrap()
    }
}
impl Drop for RelayProcess {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn open(path: &Path, seed: u8, relay: &str) -> Desk {
    let desk = Desk::new(path.to_string_lossy().into()).unwrap();
    let (_, _, policy) = synthetic::receipt(b"test configuration", ACTION, 0, 100, seed);
    let pin = &policy.pins[0];
    let settings = ConnectionSettings {
        castalia: Some(CastaliaSettings {
            issuer: pin.issuer.clone(),
            key_id: pin.key_id.clone(),
            issuer_public_key: pin.public_key.clone(),
            audience: pin.audience.clone(),
            terminal_caller: policy.expected_caller.id,
            holder_public_key: policy.actor,
            policy_digest: policy.policy_digest,
            credential_endpoint: "http://127.0.0.1:65500/credential".into(),
        }),
        nostr_relays: vec![relay.into()],
        ..Default::default()
    };
    desk.save_connection_settings(serde_json::to_string(&settings).unwrap())
        .unwrap();
    desk
}
fn bind(desk: &Desk, seed: u8, keys: &Keys) -> String {
    let raw = desk
        .social_prepare_binding(keys.secret_key().to_secret_bytes().to_vec())
        .unwrap();
    let b: KeyBinding = serde_json::from_str(&raw).unwrap();
    let (request, presentation, _) = synthetic::receipt(
        &canonical(&b).unwrap(),
        "nostr.bind",
        b.version,
        now().unwrap(),
        seed,
    );
    desk.social_approve_binding(raw, request, presentation)
        .unwrap()
}
fn approve(desk: &Desk, seed: u8, action: SocialAction) -> SocialEvent {
    let action_name = serde_json::to_value(&action).unwrap()["op"]
        .as_str()
        .unwrap()
        .to_owned();
    let raw = desk
        .social_prepare(serde_json::to_string(&action).unwrap())
        .unwrap_or_else(|error| panic!("Reader {seed} preparing {action_name}: {error:?}"));
    let event: SocialEvent = serde_json::from_str(&raw).unwrap();
    let (request, presentation, _) = synthetic::proof(&event, seed);
    desk.social_approve(raw, request, presentation)
        .unwrap_or_else(|error| panic!("Reader {seed} approving {action_name}: {error:?}"));
    event
}
fn state(desk: &Desk) -> SocialState {
    let snapshot: serde_json::Value =
        serde_json::from_str(&desk.social_snapshot(false).unwrap()).unwrap();
    serde_json::from_value(snapshot["state"].clone()).unwrap()
}
async fn exchange(desk: &Desk, relay: &RelayProcess, keys: &Keys) -> serde_json::Value {
    let raw = tokio::time::timeout(
        Duration::from_secs(90),
        desk.exchange_connected(relay.transport().await, keys.clone(), true),
    )
    .await
    .expect("Exchange must remain bounded")
    .unwrap();
    serde_json::from_str(&raw).unwrap()
}

fn discussion(group_id: &str, body: &str) -> SocialAction {
    SocialAction::Publish {
        group_id: Some(group_id.into()),
        post: social::Post {
            kind: social::PostKind::Discussion,
            book: None,
            body: body.into(),
            stars: None,
            reading_status: None,
            spoiler: true,
            reply_to: None,
        },
    }
}

async fn private_group_native_journey(relay: &RelayProcess) {
    let paths = [2u8, 3, 4].map(|seed| relay.root.join(format!("private-reader-{seed}")));
    let desks = [2u8, 3, 4].map(|seed| open(&paths[usize::from(seed - 2)], seed, &relay.url()));
    let keys = [2u8, 3, 4].map(|seed| Keys::parse(&format!("{seed:02x}").repeat(32)).unwrap());
    let identities: Vec<_> = (0..3)
        .map(|i| bind(&desks[i], i as u8 + 2, &keys[i]))
        .collect();
    for (i, desk) in desks.iter().enumerate() {
        for (j, identity) in identities.iter().enumerate() {
            if i != j {
                desk.social_import_binding(identity.clone()).unwrap();
            }
        }
    }
    let group_id = new_owned_id(&synthetic::actor(2)).unwrap();
    approve(
        &desks[0],
        2,
        SocialAction::CreateGroup {
            id: group_id.clone(),
            name: "Synthetic three-reader private circle".into(),
            description: "Only the explicitly admitted readers may discuss this book".into(),
            private: true,
        },
    );
    assert_eq!(exchange(&desks[0], relay, &keys[0]).await["sent"], 1);
    let invite_ids = [booknook_core::new_id(), booknook_core::new_id()];
    for i in 1..3 {
        approve(
            &desks[0],
            2,
            SocialAction::Invite {
                id: invite_ids[i - 1].clone(),
                group: state(&desks[0]).groups[&group_id].clone(),
                invitee: synthetic::actor(i as u8 + 2),
                expires_at: now().unwrap() + 600,
            },
        );
    }
    assert_eq!(exchange(&desks[0], relay, &keys[0]).await["sent"], 4);
    for i in 1..3 {
        let fetched = exchange(&desks[i], relay, &keys[i]).await;
        assert_eq!(
            fetched["received"], 1,
            "Each invitee receives only their invitation: {fetched}"
        );
        let before = state(&desks[i]);
        assert!(before.invitations.contains_key(&invite_ids[i - 1]));
        assert!(!before.invitations.contains_key(&invite_ids[2 - i]));
        approve(
            &desks[i],
            i as u8 + 2,
            SocialAction::AcceptInvitation {
                invitation_id: invite_ids[i - 1].clone(),
                admission_consent: None,
            },
        );
        assert_eq!(exchange(&desks[i], relay, &keys[i]).await["sent"], 2);
    }
    let accepted = exchange(&desks[0], relay, &keys[0]).await;
    assert_eq!(
        accepted["received"], 2,
        "Owner must receive two independent signed acceptances: {accepted}"
    );
    let admission = approve(
        &desks[0],
        2,
        SocialAction::SetMembers {
            group_id: group_id.clone(),
            members: [
                synthetic::actor(2),
                synthetic::actor(3),
                synthetic::actor(4),
            ]
            .into(),
            moderators: Default::default(),
            admission_proofs: Vec::new(),
        },
    );
    assert!(
        matches!(&admission.action, SocialAction::SetMembers { admission_proofs, .. } if admission_proofs.len() == 2)
    );
    assert_eq!(exchange(&desks[0], relay, &keys[0]).await["sent"], 3);
    for i in 1..3 {
        let fetched = exchange(&desks[i], relay, &keys[i]).await;
        assert_eq!(
            fetched["received"], 1,
            "Transferred proofs must work through native receive: {fetched}"
        );
        assert_eq!(state(&desks[i]).groups[&group_id].members.len(), 3);
        assert!(
            !state(&desks[i])
                .invitations
                .contains_key(&invite_ids[2 - i])
        );
    }
    let first = approve(
        &desks[1],
        3,
        discussion(&group_id, "PRIVATE-FIRST: three admitted readers"),
    );
    assert_eq!(exchange(&desks[1], relay, &keys[1]).await["sent"], 3);
    for i in [0, 2] {
        let fetched = exchange(&desks[i], relay, &keys[i]).await;
        assert_eq!(
            fetched["received"], 1,
            "Admitted reader receives exact discussion: {fetched}"
        );
        assert_eq!(
            state(&desks[i]).publications[&first.id],
            state(&desks[1]).publications[&first.id]
        );
    }
    let prior = state(&desks[2]).publications.len();
    assert_eq!(exchange(&desks[2], relay, &keys[2]).await["received"], 0);
    assert_eq!(state(&desks[2]).publications.len(), prior);

    // This record was approved for epoch 1 but never sent. The owner then knows
    // the new membership locally, so its old audience must not be retried.
    let stale = approve(
        &desks[0],
        2,
        discussion(&group_id, "PRIVATE-STALE: approved but never delivered"),
    );
    let removal = approve(
        &desks[0],
        2,
        SocialAction::SetMembers {
            group_id: group_id.clone(),
            members: [synthetic::actor(2), synthetic::actor(3)].into(),
            moderators: Default::default(),
            admission_proofs: Vec::new(),
        },
    );
    let removed = exchange(&desks[0], relay, &keys[0]).await;
    assert_eq!(
        removed["sent"], 3,
        "Only the membership change should send, including notice to the removed reader: {removed}"
    );
    assert!(!removed["messages"].as_array().unwrap().is_empty());
    assert!(state(&desks[0]).outbox[&stale.id].acknowledged.is_empty());
    for i in 1..3 {
        exchange(&desks[i], relay, &keys[i]).await;
        let current = state(&desks[i]);
        assert!(current.publications.contains_key(&removal.id));
        assert_eq!(current.groups[&group_id].members.len(), 2);
        assert!(!current.publications.contains_key(&stale.id));
    }
    assert!(
        desks[2]
            .social_prepare(
                serde_json::to_string(&discussion(&group_id, "Removed reader must be denied"))
                    .unwrap()
            )
            .is_err()
    );
    assert!(social::feed(&state(&desks[2]), &synthetic::actor(4), true).is_empty());
    let later = approve(
        &desks[1],
        3,
        discussion(&group_id, "PRIVATE-LATER: two current readers only"),
    );
    assert_eq!(exchange(&desks[1], relay, &keys[1]).await["sent"], 2);
    exchange(&desks[0], relay, &keys[0]).await;
    exchange(&desks[2], relay, &keys[2]).await;
    assert!(state(&desks[0]).publications.contains_key(&later.id));
    assert!(!state(&desks[2]).publications.contains_key(&later.id));
    assert_eq!(exchange(&desks[1], relay, &keys[1]).await["received"], 0);

    // Inspect the independent relay's actual stored envelopes, not prepared
    // in-memory records. No private plaintext or stale delivery may be present.
    let observer = relay.transport().await;
    let captured = observer.fetch(Vec::new(), None, 0).await.unwrap();
    observer.disconnect().await;
    let private: Vec<_> = captured
        .iter()
        .filter(|event| {
            event
                .tags
                .identifier()
                .is_some_and(|id| id.starts_with("booknook:private:"))
        })
        .collect();
    for envelope in &private {
        assert_eq!(envelope.tags.public_keys().count(), 1);
        let raw = envelope.as_json();
        for secret in [
            "PRIVATE-FIRST",
            "PRIVATE-STALE",
            "PRIVATE-LATER",
            "Synthetic three-reader private circle",
            group_id.as_str(),
        ] {
            assert!(
                !raw.contains(secret),
                "Private application content leaked into relay data"
            );
        }
    }
    for seed in [2, 3, 4] {
        let address = |id: &str| {
            format!(
                "booknook:private:{}",
                booknook_core::digest(format!("{id}:{}", synthetic::actor(seed)).as_bytes())
            )
        };
        assert!(
            !private
                .iter()
                .any(|event| event.tags.identifier() == Some(address(&stale.id).as_str()))
        );
        let delivered: Vec<_> = private
            .iter()
            .filter(|event| event.tags.identifier() == Some(address(&later.id).as_str()))
            .collect();
        assert_eq!(
            delivered.len(),
            usize::from(seed != 4),
            "Post-removal relay recipient set is exact"
        );
    }
    drop(desks);
    let reopened = Desk::new(paths[0].to_string_lossy().into()).unwrap();
    assert!(state(&reopened).outbox.contains_key(&stale.id));
    let retried = exchange(&reopened, relay, &keys[0]).await;
    assert_eq!(
        retried["sent"], 0,
        "Reopening must not make the stale audience eligible: {retried}"
    );
    assert!(state(&reopened).outbox[&stale.id].acknowledged.is_empty());
    println!(
        "Independent relay private groups: three native desks, isolated invitations, transferable batch admission, encrypted discussions, deduplication, member removal, exact post-removal recipients and durable stale-audience retry denial passed."
    );
}

#[test]
#[ignore = "requires BOOKNOOK_RELAY_BINARY pointing at the pinned independent relay; localhost sockets only"]
fn independent_relay_roundtrip_outage_and_durable_retry() {
    let binary = PathBuf::from(
        std::env::var_os("BOOKNOOK_RELAY_BINARY").expect("Set BOOKNOOK_RELAY_BINARY"),
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let mut relay = RelayProcess::new(binary);
        relay.start().await;
        let first_path = relay.root.join("reader-one");
        let first = open(&first_path, 2, &relay.url());
        let second = open(&relay.root.join("reader-two"), 3, &relay.url());
        let first_keys = Keys::parse(&"02".repeat(32)).unwrap();
        let second_keys = Keys::parse(&"03".repeat(32)).unwrap();
        let first_identity = bind(&first, 2, &first_keys);
        let second_identity = bind(&second, 3, &second_keys);
        first.social_import_binding(second_identity).unwrap();
        second.social_import_binding(first_identity).unwrap();
        let request = approve(&first, 2, SocialAction::Friendship { peer: synthetic::actor(3), requested: true });
        let sent = exchange(&first, &relay, &first_keys).await;
        assert_eq!(sent["sent"], 2, "Both addressed records need a real relay OK: {sent}");
        assert!(state(&first).outbox.is_empty());
        let fetched = exchange(&second, &relay, &second_keys).await;
        assert_eq!(fetched["received"], 1, "Recipient must receive the authenticated request: {fetched}");
        assert_eq!(state(&second).publications[&request.id], state(&first).publications[&request.id]);
        approve(&second, 3, SocialAction::Friendship { peer: synthetic::actor(2), requested: true });
        assert_eq!(exchange(&second, &relay, &second_keys).await["sent"], 2);
        exchange(&first, &relay, &first_keys).await;
        assert!(friends(&state(&first), &synthetic::actor(2), &synthetic::actor(3)));
        let prior = state(&second).publications.len();
        assert_eq!(exchange(&second, &relay, &second_keys).await["received"], 0);
        assert_eq!(state(&second).publications.len(), prior);
        let observer = relay.transport().await;
        let captured = observer.fetch(Vec::new(), None, 0).await.unwrap();
        observer.disconnect().await;
        let private: Vec<_> = captured.iter().filter(|e| e.tags.identifier().is_some_and(|id| id.starts_with("booknook:private:"))).collect();
        assert_eq!(private.len(), 4);
        for envelope in private {
            assert_eq!(envelope.tags.public_keys().count(), 1);
            assert!(!envelope.content.contains("friendship"));
            assert!(!envelope.content.contains(&synthetic::actor(2)));
            assert!(!envelope.content.contains(&synthetic::actor(3)));
        }
        relay.stop();
        let group = approve(&first, 2, SocialAction::CreateGroup { id: new_owned_id(&synthetic::actor(2)).unwrap(), name: "Offline private group".into(), description: "Synthetic pending content".into(), private: true });
        let failed = exchange(&first, &relay, &first_keys).await;
        assert_eq!(failed["sent"], 0);
        assert!(!failed["messages"].as_array().unwrap().is_empty());
        assert!(state(&first).outbox.contains_key(&group.id));
        drop(first);
        let reopened = Desk::new(first_path.to_string_lossy().into()).unwrap();
        assert!(state(&reopened).outbox.contains_key(&group.id));
        relay.start().await;
        let retried = exchange(&reopened, &relay, &first_keys).await;
        assert_eq!(retried["sent"], 1, "Restarted relay must acknowledge durable retry: {retried}");
        assert!(state(&reopened).outbox.is_empty());
        assert_eq!(exchange(&reopened, &relay, &first_keys).await["sent"], 0);
        let observer = relay.transport().await;
        assert!(observer.fetch(Vec::new(), None, 0).await.unwrap().len() >= 7, "Relay database must survive its process restart");
        observer.disconnect().await;
        println!("Independent relay: real ACK/fetch, addressed NIP-44 ciphertext, mutual friendship, duplicates, outage, desk reopen and relay restart retry passed. Local WS only; TLS and external identity provisioning remain unqualified.");
        relay.stop();
        // A separate relay database prevents earlier synthetic identities with
        // the same holder/key but different validity windows from contaminating
        // the independent private-group qualification.
        let mut private_relay = RelayProcess::new(relay.binary.clone());
        private_relay.start().await;
        private_group_native_journey(&private_relay).await;
    });
}

/// Replaces the real IO port while preserving the same signed-envelope acceptance path.
/// No socket or account is involved; the adapter can fail or inject forged relay records.
#[derive(Default)]
struct MemoryProviderBus {
    events: std::sync::Mutex<Vec<serde_json::Value>>,
    outage: std::sync::atomic::AtomicBool,
    calls: std::sync::atomic::AtomicUsize,
}
struct MemoryProvider(std::sync::Arc<MemoryProviderBus>);

/// Changes trusted local state while the first private send awaits its ACK.
/// Uses real signed/encrypted native envelopes; no socket or credentials.
struct RacingProvider {
    bus: std::sync::Arc<MemoryProviderBus>,
    after_first_send: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
}
#[async_trait::async_trait]
impl crate::ProviderHost for RacingProvider {
    fn descriptors_json(&self) -> Result<String> {
        MemoryProvider(self.bus.clone()).descriptors_json()
    }
    async fn execute(&self, raw: String) -> Result<crate::ProviderReply> {
        use booknook_core::provider_runtime::{ProviderOperation, ProviderRequest};
        let request: ProviderRequest = booknook_core::strict_parse(raw.as_bytes())?;
        let private_send = matches!(&request.operation, ProviderOperation::SocialPublish { event }
            if event["tags"].as_array().unwrap().iter().any(|tag| tag[0] == "p"));
        let result = MemoryProvider(self.bus.clone()).execute(raw).await?;
        if private_send {
            tokio::task::yield_now().await;
            let change = self.after_first_send.lock().unwrap().take();
            if let Some(change) = change {
                change();
            }
        }
        Ok(result)
    }
}

#[derive(Clone, Copy, Debug)]
enum SendRace {
    RevokeBinding,
    ReplaceBinding,
    ChangeTrust,
    CancelDelivery,
}

fn assert_send_race(race: SendRace) {
    use std::sync::Arc;
    let root = std::env::temp_dir().join(format!("booknook-send-race-{}", booknook_core::new_id()));
    let sender = Arc::new(open(
        &root.join("sender"),
        2,
        "wss://synthetic-relay.example/",
    ));
    let peer = open(&root.join("peer"), 3, "wss://synthetic-relay.example/");
    let keys = [2u8, 3].map(|seed| Keys::parse(&format!("{seed:02x}").repeat(32)).unwrap());
    bind(&sender, 2, &keys[0]);
    let peer_binding = bind(&peer, 3, &keys[1]);
    sender.social_import_binding(peer_binding).unwrap();
    let event = approve(
        &sender,
        2,
        SocialAction::Friendship {
            peer: synthetic::actor(3),
            requested: true,
        },
    );
    let Delivery::Direct { recipients } = &event.delivery else {
        panic!("Expected private recipients")
    };
    assert_eq!(recipients.len(), 2);
    let first_recipient = recipients.first().unwrap().clone();
    let last_recipient = recipients.last().unwrap().clone();
    let changing_sender_key = last_recipient == synthetic::actor(2);
    let original_settings = sender.connection_settings().unwrap();
    let replacement = if last_recipient == synthetic::actor(2) {
        let raw: BindingDocument = serde_json::from_str(
            &sender
                .social_export_binding(last_recipient.clone())
                .unwrap(),
        )
        .unwrap();
        let mut binding = raw.binding;
        binding.version += 1;
        binding.nostr_public_key = Keys::parse(&"04".repeat(32)).unwrap().public_key().to_hex();
        let (request, presentation, _) = synthetic::receipt(
            &canonical(&binding).unwrap(),
            "nostr.bind",
            binding.version,
            now().unwrap(),
            2,
        );
        serde_json::to_string(&BindingDocument {
            schema: raw.schema,
            binding,
            approved_at: now().unwrap(),
            request_json: request,
            presentation_json: presentation,
        })
        .unwrap()
    } else {
        bind(&peer, 3, &Keys::parse(&"04".repeat(32)).unwrap())
    };
    let replacement_for_retry = replacement.clone();
    let weak = Arc::downgrade(&sender);
    let id = event.id.clone();
    let bus = Arc::new(MemoryProviderBus::default());
    sender
        .replace_provider_host(
            Box::new(RacingProvider {
                bus: bus.clone(),
                after_first_send: std::sync::Mutex::new(Some(Box::new(move || {
                    let sender = weak.upgrade().unwrap();
                    match race {
                        SendRace::RevokeBinding => {
                            sender.social_revoke_binding(last_recipient).unwrap()
                        }
                        SendRace::ReplaceBinding => {
                            sender.social_import_binding(replacement).unwrap()
                        }
                        SendRace::ChangeTrust => {
                            let mut settings: ConnectionSettings =
                                serde_json::from_str(&sender.connection_settings().unwrap())
                                    .unwrap();
                            settings.castalia.as_mut().unwrap().policy_digest = "ab".repeat(32);
                            sender
                                .save_connection_settings(serde_json::to_string(&settings).unwrap())
                                .unwrap();
                        }
                        SendRace::CancelDelivery => sender
                            .social_local(
                                serde_json::to_string(&LocalSocialCommand::CancelDelivery {
                                    event_id: id,
                                })
                                .unwrap(),
                            )
                            .unwrap(),
                    }
                }))),
            }),
            1,
        )
        .unwrap();
    let result = sender.provider_social_exchange(
        "test.transport".into(),
        keys[0].secret_key().to_secret_bytes().to_vec(),
        false,
    );
    let private_count = bus
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| e["tags"].as_array().unwrap().iter().any(|t| t[0] == "p"))
        .count();
    assert_eq!(
        private_count, 1,
        "{race:?}: a later private envelope escaped after trusted state changed"
    );
    let report: serde_json::Value =
        serde_json::from_str(&result.expect("A stale send must stop cleanly")).unwrap();
    assert!(!report["messages"].as_array().unwrap().is_empty());
    let current = state(&sender);
    match race {
        SendRace::CancelDelivery => assert!(
            !current.outbox.contains_key(&event.id),
            "ACK resurrected cancelled delivery"
        ),
        SendRace::ChangeTrust => assert!(
            current.outbox[&event.id].acknowledged.is_empty(),
            "Provider rejected stale response; it cannot be claimed as an accepted ACK"
        ),
        _ => {
            assert_eq!(current.outbox[&event.id].acknowledged.len(), 1);
            assert!(
                current.outbox[&event.id]
                    .acknowledged
                    .contains_key(&first_recipient),
                "Real first ACK was lost"
            );
        }
    }
    drop(sender);
    let reopened = Desk::new(root.join("sender").to_string_lossy().into()).unwrap();
    assert_eq!(
        state(&reopened).outbox,
        current.outbox,
        "Durable ACK/remaining queue changed on reopen"
    );
    let mut retry_keys = Keys::parse(&"02".repeat(32)).unwrap();
    match race {
        SendRace::RevokeBinding => reopened
            .social_import_binding(replacement_for_retry)
            .unwrap(),
        SendRace::ChangeTrust => reopened
            .save_connection_settings(original_settings)
            .unwrap(),
        _ => (),
    }
    if changing_sender_key && matches!(race, SendRace::RevokeBinding | SendRace::ReplaceBinding) {
        retry_keys = Keys::parse(&"04".repeat(32)).unwrap();
    }
    reopened
        .replace_provider_host(Box::new(MemoryProvider(bus.clone())), 1)
        .unwrap();
    let retry: serde_json::Value = serde_json::from_str(
        &reopened
            .provider_social_exchange(
                "test.transport".into(),
                retry_keys.secret_key().to_secret_bytes().to_vec(),
                false,
            )
            .unwrap(),
    )
    .unwrap();
    let remaining = match race {
        SendRace::CancelDelivery => 0,
        SendRace::ChangeTrust => 2,
        _ => 1,
    };
    assert_eq!(
        retry["sent"], remaining,
        "A deliberate retry must preserve earlier confirmed recipient ACKs"
    );
    assert!(!state(&reopened).outbox.contains_key(&event.id));
    drop(reopened);
    drop(peer);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn sending_rechecks_revoked_binding_between_recipient_acks() {
    assert_send_race(SendRace::RevokeBinding);
}
#[test]
fn sending_rechecks_replaced_binding_between_recipient_acks() {
    assert_send_race(SendRace::ReplaceBinding);
}
#[test]
fn sending_rechecks_issuer_policy_between_recipient_acks() {
    assert_send_race(SendRace::ChangeTrust);
}
#[test]
fn sending_ack_never_resurrects_a_cancelled_outbox() {
    assert_send_race(SendRace::CancelDelivery);
}
#[test]
fn sending_rechecks_binding_expiry_at_dispatch_time() {
    let root =
        std::env::temp_dir().join(format!("booknook-send-expiry-{}", booknook_core::new_id()));
    let desk = open(&root, 2, "wss://synthetic-relay.example/");
    bind(&desk, 2, &Keys::parse(&"02".repeat(32)).unwrap());
    let event = approve(
        &desk,
        2,
        SocialAction::CreateGroup {
            id: new_owned_id(&synthetic::actor(2)).unwrap(),
            name: "Synthetic group".into(),
            description: "Expiry regression".into(),
            private: true,
        },
    );
    let (bindings, publication) = {
        let store = desk.lock().unwrap();
        (
            documents(&store.connection).unwrap(),
            load(&store.connection).unwrap().outbox[&event.id]
                .publication
                .clone(),
        )
    };
    desk.validate_prepared_delivery(&publication, &bindings, now().unwrap())
        .unwrap();
    assert!(
        desk.validate_prepared_delivery(&publication, &bindings, bindings[0].0.binding.valid_until)
            .is_err(),
        "A binding expiring during relay IO must stop the next dispatch"
    );
    drop(desk);
    std::fs::remove_dir_all(root).unwrap();
}
#[async_trait::async_trait]
impl crate::ProviderHost for MemoryProvider {
    fn descriptors_json(&self) -> Result<String> {
        Ok(json!({"schema":"booknook.provider-registry.v1","providers":[{"id":"test.transport","source_namespace":"test.transport","capabilities":["social_transport"],"available":true,"reason":null,"connection":null}]}).to_string())
    }
    async fn execute(&self, raw: String) -> Result<crate::ProviderReply> {
        use booknook_core::provider_runtime::{
            ProviderOperation, ProviderRequest, ProviderResponse, ProviderResult,
        };
        use std::sync::atomic::Ordering;
        self.0.calls.fetch_add(1, Ordering::SeqCst);
        if self.0.outage.load(Ordering::SeqCst) {
            return Err(
                booknook_core::Error::Unavailable("Synthetic adapter outage".into()).into(),
            );
        }
        let request: ProviderRequest = booknook_core::strict_parse(raw.as_bytes())?;
        let result = match &request.operation {
            ProviderOperation::SocialPublish { event } => {
                let parsed = Event::from_json(serde_json::to_vec(event)?).unwrap();
                parsed
                    .verify()
                    .expect("Native sends an actual signed envelope");
                let mut events = self.0.events.lock().unwrap();
                if !events.iter().any(|e| e["id"] == event["id"]) {
                    events.push(event.clone());
                }
                ProviderResult::SocialPublish {
                    event_id: parsed.id.to_hex(),
                    relays: vec!["wss://synthetic-relay.example/".into()],
                }
            }
            ProviderOperation::SocialFetch { limit, .. } => ProviderResult::SocialFetch {
                events: self
                    .0
                    .events
                    .lock()
                    .unwrap()
                    .iter()
                    .take(usize::from(*limit))
                    .cloned()
                    .collect(),
            },
            _ => {
                return Err(
                    booknook_core::Error::Invalid("Unexpected transport operation".into()).into(),
                );
            }
        };
        Ok(crate::ProviderReply {
            response_json: serde_json::to_string(&ProviderResponse {
                schema: "booknook.provider-response.v1".into(),
                request_id: request.request_id,
                generation: request.generation,
                provider_id: request.provider_id,
                connection_id: request.connection_id,
                result,
            })?,
            content_bytes: vec![],
        })
    }
}
#[test]
fn injected_social_transport_preserves_authorization_outage_replay_and_forgery_checks() {
    use std::sync::{Arc, atomic::Ordering};
    let root = std::env::temp_dir().join(format!(
        "booknook-injected-social-{}",
        booknook_core::new_id()
    ));
    let bus = Arc::new(MemoryProviderBus::default());
    let desks = [2u8, 3].map(|seed| {
        open(
            &root.join(format!("reader-{seed}")),
            seed,
            "wss://synthetic-relay.example/",
        )
    });
    for desk in &desks {
        desk.replace_provider_host(Box::new(MemoryProvider(bus.clone())), 1)
            .unwrap();
    }
    let keys = [2u8, 3].map(|seed| Keys::parse(&format!("{seed:02x}").repeat(32)).unwrap());
    let secret = |index: usize| keys[index].secret_key().to_secret_bytes().to_vec();
    assert!(
        desks[0]
            .provider_social_exchange("test.transport".into(), secret(0), true)
            .is_err()
    );
    assert_eq!(
        bus.calls.load(Ordering::SeqCst),
        0,
        "No provider call before Castalia key binding"
    );
    let identity0 = bind(&desks[0], 2, &keys[0]);
    let identity1 = bind(&desks[1], 3, &keys[1]);
    desks[0].social_import_binding(identity1).unwrap();
    desks[1].social_import_binding(identity0).unwrap();
    let event = approve(
        &desks[0],
        2,
        SocialAction::Publish {
            group_id: None,
            post: Post {
                kind: PostKind::Review,
                book: Some(BookReference {
                    id: "urn:isbn:synthetic".into(),
                    title: "Synthetic book".into(),
                    author: "Writer".into(),
                }),
                body: "Only explicitly approved words".into(),
                stars: Some(5),
                reading_status: None,
                spoiler: false,
                reply_to: None,
            },
        },
    );
    bus.outage.store(true, Ordering::SeqCst);
    let report: serde_json::Value = serde_json::from_str(
        &desks[0]
            .provider_social_exchange("test.transport".into(), secret(0), true)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(report["sent"], 0);
    assert!(state(&desks[0]).outbox[&event.id].acknowledged.is_empty());
    bus.outage.store(false, Ordering::SeqCst);
    let report: serde_json::Value = serde_json::from_str(
        &desks[0]
            .provider_social_exchange("test.transport".into(), secret(0), true)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(report["sent"], 1);
    assert!(
        !state(&desks[0]).outbox.contains_key(&event.id),
        "Acknowledged public delivery leaves the pending outbox"
    );
    desks[1]
        .provider_social_exchange("test.transport".into(), secret(1), true)
        .unwrap();
    assert!(state(&desks[1]).publications.contains_key(&event.id));
    let before = state(&desks[1]).publications.clone();
    let replay: serde_json::Value = serde_json::from_str(
        &desks[1]
            .provider_social_exchange("test.transport".into(), secret(1), true)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(replay["received"], 0);
    assert_eq!(state(&desks[1]).publications, before);
    let mut forged = bus
        .events
        .lock()
        .unwrap()
        .iter()
        .find(|e| {
            e["content"]
                .as_str()
                .is_some_and(|s| s.contains("Only explicitly approved words"))
        })
        .unwrap()
        .clone();
    forged["id"] = json!("f".repeat(64));
    forged["content"] = json!("Forged provider content");
    bus.events.lock().unwrap().push(forged);
    let report: serde_json::Value = serde_json::from_str(
        &desks[1]
            .provider_social_exchange("test.transport".into(), secret(1), true)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(report["received"], 0);
    assert_eq!(state(&desks[1]).publications, before);
    let pending: i64 = desks[1]
        .lock()
        .unwrap()
        .connection
        .query_row("SELECT count(*) FROM social_pending", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        pending, 0,
        "Forged transport data never enters verified pending state"
    );
    // A valid Nostr signature still cannot authorize changed Castalia-approved content.
    let original = bus
        .events
        .lock()
        .unwrap()
        .iter()
        .find(|e| {
            e["content"]
                .as_str()
                .is_some_and(|s| s.contains("Only explicitly approved words"))
        })
        .unwrap()
        .clone();
    let original = Event::from_json(serde_json::to_vec(&original).unwrap()).unwrap();
    let mut payload: serde_json::Value = serde_json::from_str(&original.content).unwrap();
    payload["event"]["action"]["post"]["body"] = json!("Unapproved alteration");
    let signed = EventBuilder::new(original.kind, payload.to_string())
        .tags(original.tags.clone())
        .custom_created_at(original.created_at)
        .sign_with_keys(&keys[0])
        .unwrap();
    signed.verify().unwrap();
    bus.events
        .lock()
        .unwrap()
        .push(serde_json::to_value(signed).unwrap());
    desks[1]
        .provider_social_exchange("test.transport".into(), secret(1), true)
        .unwrap();
    assert_eq!(state(&desks[1]).publications, before);
    drop(desks);
    std::fs::remove_dir_all(root).unwrap();
}
