// SPDX-License-Identifier: MIT OR Apache-2.0
//! The public Rust client exchanges real HTTP with a bounded literal-loopback fixture.
//! Synthetic response records prove wire/validation behavior, never Amazon account access.
use booknook_connectors::amazon::CompanionClient;
use booknook_core::{Error, castalia_connections::CredentialStorage};
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
    time::timeout,
};

const TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const ASIN: &str = "B012345678";

struct Request {
    line: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}
enum ContentLength {
    Actual,
    Declared(usize),
    Omitted,
}

async fn server(
    status: &str,
    body: Vec<u8>,
    extra_headers: &str,
    length: ContentLength,
) -> (CompanionClient, JoinHandle<Request>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let length = match length {
        ContentLength::Actual => format!("Content-Length: {}\r\n", body.len()),
        ContentLength::Declared(length) => format!("Content-Length: {length}\r\n"),
        ContentLength::Omitted => String::new(),
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{length}Connection: close\r\n{extra_headers}\r\n"
    );
    let handle = tokio::spawn(async move {
        timeout(Duration::from_secs(5), async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let end = loop {
                let mut chunk = [0u8; 1024];
                let n = stream.read(&mut chunk).await.unwrap();
                assert!(n > 0, "request ended before headers");
                bytes.extend_from_slice(&chunk[..n]);
                assert!(bytes.len() <= 8192, "test request exceeded its bound");
                if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let head = String::from_utf8(bytes[..end].to_vec()).unwrap();
            let mut lines = head.split("\r\n");
            let line = lines.next().unwrap().to_owned();
            let headers: BTreeMap<String, String> = lines
                .filter(|line| !line.is_empty())
                .map(|line| {
                    let (name, value) = line.split_once(':').unwrap();
                    (name.to_ascii_lowercase(), value.trim().to_owned())
                })
                .collect();
            let length = headers
                .get("content-length")
                .map(|n| n.parse::<usize>().unwrap())
                .unwrap_or(0);
            assert!(length <= 4096);
            while bytes.len() < end + length {
                let mut chunk = [0u8; 1024];
                let n = stream.read(&mut chunk).await.unwrap();
                assert!(n > 0, "request ended before body");
                bytes.extend_from_slice(&chunk[..n]);
            }
            stream.write_all(response.as_bytes()).await.unwrap();
            // A bounded client may close immediately after rejecting the headers.
            let _ = stream.write_all(&body).await;
            let _ = stream.shutdown().await;
            Request {
                line,
                headers,
                body: bytes[end..end + length].to_vec(),
            }
        })
        .await
        .expect("loopback fixture must finish within five seconds")
    });
    (CompanionClient::new(&endpoint).unwrap(), handle)
}

async fn json_server(value: Value) -> (CompanionClient, JoinHandle<Request>) {
    server(
        "200 OK",
        serde_json::to_vec(&value).unwrap(),
        "",
        ContentLength::Actual,
    )
    .await
}

fn positions() -> Value {
    json!({"schema":"booknook.audible-positions.v1","observations":[{
        "asin":ASIN,"position_milliseconds":123000,"source_updated_at":100,
        "fetched_at":2000,"duration_milliseconds":300000,"edition_identifier":null,
        "alignment":"unmatched","metadata":{"title":"Reading café 🌿","authors":["A. Reader"]}
    }]})
}
fn registry() -> Value {
    json!([{
        "schema":"castalia.provider-connection.v1","connection_id":"synthetic.ca",
        "provider_id":"com.audible","display_name":"Audible (CA)",
        "capabilities":["audible.positions"],"status":"available",
        "credential":{"reference":"synthetic.credential","storage":"secure_local","portability":"reauthorization_required"}
    },{
        "schema":"castalia.provider-connection.v1","connection_id":"synthetic.kindle",
        "provider_id":"com.amazon.kindle","display_name":"Selected Kindle catalog",
        "capabilities":["kindle.catalog"],"status":"available","credential":null
    },{
        "schema":"castalia.provider-connection.v1","connection_id":"synthetic.unavailable",
        "provider_id":"example.source","display_name":"Unavailable source",
        "capabilities":[],"status":"unavailable","credential":null
    }])
}
fn authorized(request: &Request) {
    assert_eq!(
        request.headers.get("authorization"),
        Some(&format!("Bearer {TOKEN}"))
    );
    assert!(
        !request.line.contains(TOKEN),
        "authentication stays out of the URL"
    );
}

#[tokio::test]
async fn explicit_marketplaces_and_legacy_us_have_distinct_compatible_wire_requests() {
    let asins = [ASIN.to_string()];
    for marketplace in ["us", "uk", "de", "ca", "au"] {
        let (client, request) = json_server(positions()).await;
        let observations = client
            .audible_positions_in_marketplace(TOKEN, marketplace, &asins, 2000)
            .await
            .unwrap();
        assert_eq!(observations[0].source_updated_at, 100);
        assert_eq!(observations[0].fetched_at, 2000);
        assert!(observations[0].stale(2000));
        assert_eq!(observations[0].alignment, "unmatched");
        let request = request.await.unwrap();
        authorized(&request);
        assert_eq!(request.line, "POST /v1/audible/positions HTTP/1.1");
        assert_eq!(
            serde_json::from_slice::<Value>(&request.body).unwrap(),
            json!({"asins":[ASIN],"marketplace":marketplace})
        );
    }
    let (client, request) = json_server(positions()).await;
    client.audible_positions(TOKEN, &asins, 2000).await.unwrap();
    let request = request.await.unwrap();
    authorized(&request);
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body).unwrap(),
        json!({"asins":[ASIN]})
    );
}

#[tokio::test]
async fn provider_registry_uses_authenticated_marketplace_route_and_shared_types() {
    for marketplace in ["us", "uk", "de", "ca", "au"] {
        let (client, request) = json_server(registry()).await;
        let connections = client
            .provider_connections(TOKEN, marketplace)
            .await
            .unwrap();
        assert_eq!(connections.len(), 3);
        assert!(connections[1].credential.is_none());
        assert!(connections[2].credential.is_none());
        assert_eq!(
            connections[0].credential.as_ref().unwrap().storage,
            CredentialStorage::SecureLocal
        );
        assert_eq!(serde_json::to_value(connections).unwrap(), registry());
        let request = request.await.unwrap();
        authorized(&request);
        assert_eq!(
            request.line,
            format!("GET /v1/provider-connections/{marketplace} HTTP/1.1")
        );
        assert!(request.body.is_empty());
    }
}

#[tokio::test]
async fn malformed_or_ambiguous_descriptors_fail_closed_without_echoing_source_material() {
    let mut invalid = Vec::new();
    for (field, value) in [
        ("schema", json!("castalia.provider-connection.v2")),
        ("display_name", json!("bad\u{202e}label")),
        ("access_token", json!("synthetic-private-response-marker")),
        (
            "capabilities",
            json!(["audible.positions", "audible.positions"]),
        ),
    ] {
        let mut response = registry();
        response[0][field] = value;
        invalid.push(serde_json::to_vec(&response).unwrap());
    }
    let mut response = registry();
    response[0]["credential"]["storage"] = json!("synthetic-private-response-marker");
    invalid.push(serde_json::to_vec(&response).unwrap());
    let mut response = registry();
    response[0].as_object_mut().unwrap().remove("credential");
    invalid.push(serde_json::to_vec(&response).unwrap());
    invalid.push(serde_json::to_vec(&vec![registry()[0].clone(); 2]).unwrap());
    let too_many: Vec<_> = (0..65)
        .map(|index| {
            let mut connection = registry()[0].clone();
            connection["connection_id"] = json!(format!("synthetic.connection-{index}"));
            connection
        })
        .collect();
    invalid.push(serde_json::to_vec(&too_many).unwrap());
    invalid.push(
        serde_json::to_string(&registry())
            .unwrap()
            .replacen("\"status\":", "\"status\":\"unavailable\",\"status\":", 1)
            .into_bytes(),
    );
    for body in invalid {
        let (client, request) = server("200 OK", body, "", ContentLength::Actual).await;
        let error = client.provider_connections(TOKEN, "ca").await.unwrap_err();
        assert!(
            !error
                .to_string()
                .contains("synthetic-private-response-marker")
        );
        assert!(matches!(error, Error::Invalid(_)));
        authorized(&request.await.unwrap());
    }
}

#[tokio::test]
async fn invalid_marketplace_token_and_asin_are_rejected_before_network_io() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client =
        CompanionClient::new(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    for marketplace in ["", "CA", "fr", "../us", "ca?token=synthetic"] {
        assert!(matches!(
            client.provider_connections(TOKEN, marketplace).await,
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            client
                .audible_positions_in_marketplace(TOKEN, marketplace, &[ASIN.into()], 2000)
                .await,
            Err(Error::Invalid(_))
        ));
    }
    assert!(matches!(
        client.provider_connections("invalid", "ca").await,
        Err(Error::Denied(_))
    ));
    assert!(matches!(
        client
            .audible_positions_in_marketplace("invalid", "ca", &[ASIN.into()], 2000)
            .await,
        Err(Error::Denied(_))
    ));
    assert!(matches!(
        client
            .audible_positions_in_marketplace(TOKEN, "ca", &["not-an-asin".into()], 2000)
            .await,
        Err(Error::Invalid(_))
    ));
    assert!(
        timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn denied_oversized_and_redirected_registries_never_become_connection_success() {
    let (client, request) = server(
        "401 Unauthorized",
        br#"{"error":"synthetic-private-response-marker"}"#.to_vec(),
        "",
        ContentLength::Actual,
    )
    .await;
    let error = client.provider_connections(TOKEN, "ca").await.unwrap_err();
    assert!(matches!(error, Error::Denied(_)));
    assert!(
        !error
            .to_string()
            .contains("synthetic-private-response-marker")
    );
    authorized(&request.await.unwrap());
    let mut oversized_valid_json = vec![b' '; 1024 * 1024];
    oversized_valid_json.extend_from_slice(b"[]");
    for (body, declared_length) in [
        (b"[]".to_vec(), ContentLength::Declared(1024 * 1024 + 1)),
        (oversized_valid_json, ContentLength::Omitted),
    ] {
        let (client, request) = server("200 OK", body, "", declared_length).await;
        let error = client.provider_connections(TOKEN, "ca").await.unwrap_err();
        assert!(
            matches!(&error, Error::Invalid(message) if message == "Companion response too large")
        );
        authorized(&request.await.unwrap());
    }
    let redirect = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let header = format!(
        "Location: http://{}/unexpected\r\n",
        redirect.local_addr().unwrap()
    );
    let (client, request) =
        server("302 Found", b"[]".to_vec(), &header, ContentLength::Actual).await;
    assert!(matches!(
        client.provider_connections(TOKEN, "ca").await,
        Err(Error::Unavailable(_))
    ));
    authorized(&request.await.unwrap());
    assert!(
        timeout(Duration::from_millis(50), redirect.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn explicit_marketplace_preserves_position_identity_time_and_alignment_rejection() {
    for (field, value) in [
        ("asin", json!("B099999999")),
        ("source_updated_at", json!(4000)),
        ("alignment", json!("confirmed")),
    ] {
        let mut response = positions();
        response["observations"][0][field] = value;
        let (client, request) = json_server(response).await;
        assert!(matches!(
            client
                .audible_positions_in_marketplace(TOKEN, "uk", &[ASIN.into()], 2000)
                .await,
            Err(Error::Invalid(_))
        ));
        authorized(&request.await.unwrap());
    }
}
