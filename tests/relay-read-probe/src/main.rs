// SPDX-License-Identifier: MIT OR Apache-2.0
//! Opt-in read-only acceptance probe. No keys, signer, EVENT, AUTH, publication,
//! user configuration, user records, certificate overrides or alternate transport.
use booknook_connectors::nostr_transport::SocialRelayTransport;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tracing::{
    Event, Metadata, Subscriber,
    field::{Field, Visit},
    span::{Attributes, Id, Record},
};

#[derive(Default)]
struct Observed {
    connected: bool,
    sent: BTreeMap<String, usize>,
    received: BTreeMap<String, usize>,
    errors: usize,
}
struct Watch(Arc<Mutex<Observed>>);
#[derive(Default)]
struct Message(String);
impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}
// Observe only SDK protocol message types, never retain or print frame contents.
impl Subscriber for Watch {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target().starts_with("nostr_relay_pool")
    }
    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }
    fn record(&self, _: &Id, _: &Record<'_>) {}
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}
    fn event(&self, event: &Event<'_>) {
        let mut text = Message::default();
        event.record(&mut text);
        let mut state = self.0.lock().unwrap();
        if text.0.starts_with("Connected to '") {
            state.connected = true;
        }
        if *event.metadata().level() == tracing::Level::ERROR {
            state.errors += 1;
        }
        for (prefix, outgoing) in [("Sending '[", true), ("Received '[", false)] {
            if text.0.starts_with(prefix) {
                let json_start = prefix.len() - 1;
                let suffix = if outgoing { "' to '" } else { "' from '" };
                if let Some(json_end) = text.0.rfind(suffix) {
                    if let Ok(value) =
                        serde_json::from_str::<serde_json::Value>(&text.0[json_start..json_end])
                    {
                        if let Some(kind) = value.get(0).and_then(|v| v.as_str()) {
                            let counts = if outgoing {
                                &mut state.sent
                            } else {
                                &mut state.received
                            };
                            *counts.entry(kind.to_owned()).or_default() += 1;
                        }
                    }
                }
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 || args[1] != "--read-only-public-relay" {
        eprintln!(
            "Opt in with --read-only-public-relay wss://PUBLIC_RELAY. This sends one public read subscription, no identity or publication."
        );
        std::process::exit(2);
    }
    let relay = &args[2];
    if !relay.starts_with("wss://") || relay.contains(['@', '?', '#']) {
        eprintln!("Use a public wss:// relay URL without credentials, query or fragment.");
        std::process::exit(2);
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let observed = Arc::new(Mutex::new(Observed::default()));
    tracing::subscriber::set_global_default(Watch(observed.clone())).unwrap();
    let started = Instant::now();
    let mut outcome = "not_started";
    let mut count = None;
    let operation = async {
        match SocialRelayTransport::connect(&[relay.to_owned()]).await {
            Ok(transport) => {
                // connect() starts its task asynchronously. Wait for the actual
                // default transport's successful connection log before reading.
                let wait = Instant::now();
                while !observed.lock().unwrap().connected && wait.elapsed() < Duration::from_secs(8)
                {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                if observed.lock().unwrap().connected {
                    // A one-second public time window; no person or user key is queried.
                    match transport.fetch_page(vec![], None, now, Some(now)).await {
                        Ok(events) => {
                            count = Some(events.len());
                            outcome = "fetch_returned";
                        }
                        Err(_) => outcome = "fetch_error",
                    }
                } else {
                    outcome = "connection_unconfirmed";
                }
                transport.disconnect().await;
            }
            Err(_) => outcome = "connect_error",
        }
    };
    if tokio::time::timeout(Duration::from_secs(25), operation)
        .await
        .is_err()
    {
        outcome = "overall_timeout";
    }
    let state = observed.lock().unwrap();
    let success = outcome == "fetch_returned"
        && state.errors == 0
        && state.connected
        && state.sent.get("REQ") == Some(&1)
        && state.sent.get("CLOSE") == Some(&1)
        && state
            .sent
            .keys()
            .all(|kind| kind == "REQ" || kind == "CLOSE")
        && state.received.get("EOSE").is_some_and(|n| *n > 0)
        && !state.sent.contains_key("EVENT")
        && !state.sent.contains_key("AUTH");
    println!(
        "{}",
        serde_json::json!({
            "schema": "booknook.read-only-relay-probe.v1", "relay": relay,
            "started_at_unix": now, "duration_ms": started.elapsed().as_millis(),
            "transport": "SocialRelayTransport::connect / DefaultWebsocketTransport",
            "query": {"kinds":[30078],"#t":["booknook"],"since":now,"until":now,"limit":500},
            "signer": "none", "outcome":outcome, "connected":state.connected,
            "outbound_message_kinds":state.sent,"inbound_message_kinds":state.received,
            "sdk_error_count":state.errors,"returned_event_count":count,"passed":success,
            "scope":"Remote default TLS/WebSocket handshake plus read-only REQ/EOSE only; no publication, identity, consent, private delivery or whole-social qualification."
        })
    );
    std::process::exit(if success { 0 } else { 1 });
}
