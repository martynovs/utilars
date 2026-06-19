mod common;

use common::client;
use futures::{StreamExt, TryStreamExt};
use serde_json::json;
use utilars::VaultId;
use wiremock::matchers::{method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn vaults_list_authenticates_and_parses() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "vaults": [{ "name": "vaults/abc", "displayName": "Treasury" }],
            "nextPageToken": "page2",
            "totalSize": 1
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let page = client
        .vaults()
        .list()
        .page_size(50)
        .page_token("tok")
        .send()
        .await
        .unwrap();

    assert_eq!(page.vaults[0].name, "vaults/abc");
    assert_eq!(page.next_page_token.as_deref(), Some("page2"));

    let reqs = server.received_requests().await.unwrap();
    let auth = reqs[0]
        .headers
        .get("authorization")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(auth.starts_with("Bearer "), "got: {auth}");
}

#[tokio::test]
async fn vaults_stream_walks_all_pages() {
    let server = MockServer::start().await;
    // page 1 (no pageToken) → nextPageToken=page2
    Mock::given(method("GET"))
        .and(path("/v2/vaults"))
        .and(query_param_is_missing("pageToken"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "vaults": [{ "name": "vaults/abc", "displayName": "A" }],
            "nextPageToken": "page2"
        })))
        .mount(&server)
        .await;
    // page 2 (pageToken=page2) → no further token
    Mock::given(method("GET"))
        .and(path("/v2/vaults"))
        .and(query_param("pageToken", "page2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "vaults": [{ "name": "vaults/def", "displayName": "B" }]
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let names: Vec<String> = client
        .vaults()
        .stream()
        .map_ok(|v| v.name)
        .try_collect()
        .await
        .unwrap();

    assert_eq!(names, vec!["vaults/abc", "vaults/def"]);
}

#[tokio::test]
async fn vaults_stream_empty_yields_nothing() {
    let server = MockServer::start().await;
    // A single empty page, no nextPageToken → the stream ends immediately.
    Mock::given(method("GET"))
        .and(path("/v2/vaults"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "vaults": [] })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let all: Vec<_> = client.vaults().stream().try_collect().await.unwrap();
    assert!(all.is_empty());
}

#[tokio::test]
async fn vaults_stream_surfaces_mid_stream_error() {
    let server = MockServer::start().await;
    // page 1 yields an item + a nextPageToken …
    Mock::given(method("GET"))
        .and(path("/v2/vaults"))
        .and(query_param_is_missing("pageToken"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "vaults": [{ "name": "vaults/abc", "displayName": "A" }],
            "nextPageToken": "page2"
        })))
        .mount(&server)
        .await;
    // … but fetching page 2 fails. The error must surface *after* page-1 items, not swallow them.
    Mock::given(method("GET"))
        .and(path("/v2/vaults"))
        .and(query_param("pageToken", "page2"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({
            "code": 14, "message": "unavailable"
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    // Collect every yielded item (Ok and Err) to assert ordering.
    let results: Vec<_> = client.vaults().stream().collect().await;
    assert!(
        matches!(results.as_slice(), [Ok(v), Err(_)] if v.name == "vaults/abc"),
        "expected [Ok(abc), Err(_)], got {results:?}"
    );
}

#[tokio::test]
async fn vaults_get_by_typed_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/abc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "vault": { "name": "vaults/abc", "displayName": "Treasury" }
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let vault = client.vaults().get(VaultId::new("abc")).await.unwrap();
    assert_eq!(vault.name, "vaults/abc");
}

#[tokio::test]
async fn api_error_is_typed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/missing"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "code": 5,
            "message": "vault not found",
            "details": [
                { "@type": "type.googleapis.com/google.rpc.ErrorInfo", "reason": "NOT_FOUND" }
            ]
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let err = client
        .vaults()
        .get(VaultId::new("missing"))
        .await
        .unwrap_err();
    match err {
        utilars::UtilaError::Api {
            code,
            message,
            details,
        } => {
            assert_eq!(code, 5);
            assert_eq!(message, "vault not found");
            // The gRPC status `details` array is surfaced verbatim.
            assert_eq!(details.len(), 1);
            let first = details.first().expect("one detail entry");
            assert_eq!(
                first.get("reason").and_then(|v| v.as_str()),
                Some("NOT_FOUND")
            );
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}

#[tokio::test]
async fn vault_get_missing_field_is_typed_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/novault"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    let client = client(&server.uri());
    let err = client
        .vaults()
        .get(VaultId::new("novault"))
        .await
        .unwrap_err();
    assert!(matches!(err, utilars::UtilaError::Api { code: -1, .. }));
}
