// SPDX-License-Identifier: MIT OR Apache-2.0
use booknook_core::{
    model::State,
    provider_runtime::{
        CatalogInput, ProviderRequest, ProviderResponse, ProviderResult, SourceCatalogEntry,
    },
};
use booknook_native::{Desk, NativeError, ProviderHost, ProviderReply};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "booknook-provider-test-{}",
            booknook_core::new_id()
        )))
    }
    fn path(&self) -> String {
        self.0.to_string_lossy().into()
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[derive(Default)]
struct Capture {
    requests: Mutex<Vec<ProviderRequest>>,
    gate: Option<(tokio::sync::Notify, tokio::sync::Notify)>,
}
struct Host {
    capture: Arc<Capture>,
    bytes: Vec<u8>,
}
impl Host {
    fn new(capture: Arc<Capture>) -> Box<Self> {
        Box::new(Self {
            capture,
            bytes: vec![],
        })
    }
}
#[async_trait::async_trait]
impl ProviderHost for Host {
    fn descriptors_json(&self) -> Result<String, NativeError> {
        Ok(json!({"schema":"booknook.provider-registry.v1","providers":[{"id":"synthetic.library","source_namespace":"synthetic.library","capabilities":["catalog","content","recommendations","identity"],"available":true,"reason":null,"connection":{"schema":"castalia.provider-connection.v1","connection_id":"synthetic-account","provider_id":"synthetic.library","display_name":"Synthetic source","capabilities":["catalog","content","recommendations","identity"],"status":"available","credential":null}}]}).to_string())
    }
    async fn execute(&self, request_json: String) -> Result<ProviderReply, NativeError> {
        use booknook_core::provider_runtime::ProviderOperation;
        let request: ProviderRequest = booknook_core::strict_parse(request_json.as_bytes())?;
        self.capture.requests.lock().unwrap().push(request.clone());
        if let Some((started, release)) = &self.capture.gate {
            started.notify_one();
            release.notified().await;
        }
        let result = match &request.operation {
            ProviderOperation::Catalog { source_id } => ProviderResult::Catalog {
                catalog: CatalogInput::Normalized {
                    entries: vec![SourceCatalogEntry {
                        source_id: source_id.clone(),
                        title: "Library book".into(),
                        authors: vec!["Synthetic author".into()],
                        subjects: vec![],
                        edition_identifiers: vec![],
                        language: None,
                    }],
                },
            },
            ProviderOperation::Content {
                source_id,
                media_type,
            } => ProviderResult::Content {
                source_id: source_id.clone(),
                media_type: media_type.clone(),
                sha256: booknook_core::digest(&self.bytes),
            },
            ProviderOperation::Recommendations { input } => ProviderResult::Recommendations {
                recommendations: input
                    .candidates
                    .iter()
                    .map(|c| booknook_core::social::Recommendation {
                        book_id: c.book_id.clone(),
                        score: 1,
                        reasons: vec!["Synthetic catalog reason".into()],
                    })
                    .collect(),
            },
            ProviderOperation::Identity { entity_iri } => ProviderResult::Identity {
                reference: booknook_core::identity::DirectoryReference {
                    entity_iri: entity_iri.clone(),
                    kind: booknook_core::identity::DirectoryKind::Person,
                    display_name: Some("Public reader".into()),
                },
            },
            _ => {
                return Err(NativeError::Failure {
                    message: "Unsupported test operation".into(),
                });
            }
        };
        let bytes = if matches!(result, ProviderResult::Content { .. }) {
            self.bytes.clone()
        } else {
            vec![]
        };
        Ok(ProviderReply {
            response_json: serde_json::to_string(&ProviderResponse {
                schema: "booknook.provider-response.v1".into(),
                request_id: request.request_id,
                generation: request.generation,
                provider_id: request.provider_id,
                connection_id: request.connection_id,
                result,
            })?,
            content_bytes: bytes,
        })
    }
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}
fn snapshot(desk: &Desk) -> State {
    serde_json::from_str(&desk.snapshot().unwrap()).unwrap()
}
fn op() -> String {
    json!({"kind":"catalog","source_id":"selected-file"}).to_string()
}
#[test]
fn callback_read_is_nonmutating_then_commit_uses_real_sqlite_and_survives_reopen() {
    let directory = Temp::new();
    let capture = Arc::new(Capture::default());
    let desk = Desk::with_providers(directory.path(), Host::new(capture.clone()), 1).unwrap();
    let before = snapshot(&desk);
    let reply = runtime()
        .block_on(desk.provider_fetch("synthetic.library".into(), op()))
        .unwrap();
    assert_eq!(snapshot(&desk), before);
    assert_eq!(capture.requests.lock().unwrap().len(), 1);
    let committed = desk
        .provider_commit_catalog(reply.response_json.clone())
        .unwrap();
    let report: serde_json::Value = serde_json::from_str(&committed).unwrap();
    assert_eq!(report["result"]["inserted"], 1);
    let state = snapshot(&desk);
    let book = state.books.values().next().unwrap();
    assert_eq!(
        book.provenance[0].provider_connection_id.as_deref(),
        Some("synthetic-account")
    );
    let retry: serde_json::Value =
        serde_json::from_str(&desk.provider_commit_catalog(reply.response_json).unwrap()).unwrap();
    assert_eq!(retry["result"]["unchanged"], 1);
    drop(desk);
    assert_eq!(
        snapshot(&Desk::new(directory.path()).unwrap()).books,
        state.books
    );
}
#[test]
fn late_callback_and_old_execution_cannot_commit_after_host_replacement() {
    let directory = Temp::new();
    let capture = Arc::new(Capture {
        gate: Some((Default::default(), Default::default())),
        ..Default::default()
    });
    let desk =
        Arc::new(Desk::with_providers(directory.path(), Host::new(capture.clone()), 1).unwrap());
    runtime().block_on(async {
        let task_desk = desk.clone();
        let task = tokio::spawn(async move {
            task_desk
                .provider_fetch("synthetic.library".into(), op())
                .await
        });
        capture.gate.as_ref().unwrap().0.notified().await;
        desk.replace_provider_host(Host::new(Arc::new(Capture::default())), 2)
            .unwrap();
        capture.gate.as_ref().unwrap().1.notify_one();
        assert!(task.await.unwrap().is_err());
    });
    assert!(snapshot(&desk).books.is_empty());
    let read = runtime()
        .block_on(desk.provider_fetch("synthetic.library".into(), op()))
        .unwrap();
    desk.replace_provider_host(Host::new(Arc::new(Capture::default())), 3)
        .unwrap();
    assert!(desk.provider_commit_catalog(read.response_json).is_err());
    assert!(snapshot(&desk).books.is_empty());
}
#[test]
fn independent_window_settings_change_and_tampered_reads_are_rejected_atomically() {
    let directory = Temp::new();
    let desk =
        Desk::with_providers(directory.path(), Host::new(Arc::new(Capture::default())), 1).unwrap();
    let other = Desk::new(directory.path()).unwrap();
    let read = runtime()
        .block_on(desk.provider_fetch("synthetic.library".into(), op()))
        .unwrap();
    let mut tampered: serde_json::Value = serde_json::from_str(&read.response_json).unwrap();
    tampered["response"]["result"]["catalog"]["entries"][0]["title"] =
        json!("Substituted after review");
    assert!(desk.provider_commit_catalog(tampered.to_string()).is_err());
    let mut settings: serde_json::Value =
        serde_json::from_str(&other.connection_settings().unwrap()).unwrap();
    settings["nostr_relays"] = json!(["wss://relay.example"]);
    other
        .save_connection_settings(settings.to_string())
        .unwrap();
    assert!(desk.provider_commit_catalog(read.response_json).is_err());
    assert!(snapshot(&desk).books.is_empty());
    let fresh = runtime()
        .block_on(desk.provider_fetch("synthetic.library".into(), op()))
        .unwrap();
    desk.provider_commit_catalog(fresh.response_json).unwrap();
    assert_eq!(snapshot(&other).books.len(), 1);
}
#[test]
fn content_hash_validation_precedes_native_format_validation_and_state_commit() {
    let directory = Temp::new();
    let desk = Desk::with_providers(
        directory.path(),
        Box::new(Host {
            capture: Arc::new(Capture::default()),
            bytes: b"not a PDF".to_vec(),
        }),
        1,
    )
    .unwrap();
    let read = runtime()
        .block_on(desk.provider_fetch("synthetic.library".into(), op()))
        .unwrap();
    desk.provider_commit_catalog(read.response_json).unwrap();
    let before = snapshot(&desk);
    let book_id = before.books.keys().next().unwrap().clone();
    let read = runtime()
        .block_on(
            desk.provider_fetch(
                "synthetic.library".into(),
                json!({"kind":"content","source_id":"selected-pdf","media_type":"application/pdf"})
                    .to_string(),
            ),
        )
        .unwrap();
    assert!(
        desk.provider_commit_content(read.response_json, read.content_bytes, book_id)
            .is_err()
    );
    assert_eq!(snapshot(&desk), before);
}
#[test]
fn real_recommendation_and_identity_call_sites_use_minimized_provider_inputs() {
    let directory = Temp::new();
    let capture = Arc::new(Capture::default());
    let desk = Desk::with_providers(directory.path(), Host::new(capture.clone()), 1).unwrap();
    let result = runtime()
        .block_on(desk.provider_social_recommendations(
            "synthetic.library".into(),
            json!([{"book_id":"b","title":"A catalog book","subjects":[]}]).to_string(),
            "[]".into(),
        ))
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap()[0]["book_id"],
        "b"
    );
    let read = runtime()
        .block_on(desk.provider_fetch(
            "synthetic.library".into(),
            json!({"kind":"identity","entity_iri":"did:example:reader"}).to_string(),
        ))
        .unwrap();
    assert!(read.response_json.contains("did:example:reader"));
    let requests = capture.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let raw = serde_json::to_string(&requests[0]).unwrap();
    assert!(!raw.contains("notebooks"));
    assert!(!raw.contains("private_key"));
    assert!(snapshot(&desk).books.is_empty());
}
