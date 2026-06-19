mod common;

use common::client;
use futures::TryStreamExt;
use serde_json::json;
use utilars::{NetworkId, VaultId};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn networks_list_and_stream() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/networks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "networks": [
                { "name": "networks/ethereum-mainnet", "displayName": "Ethereum",
                  "nativeAsset": "assets/native.ethereum-mainnet", "status": "ACTIVE",
                  "testnet": false, "custom": false },
                { "name": "networks/sepolia", "displayName": "Sepolia", "testnet": true }
            ]
        })))
        .mount(&server)
        .await;
    let client = client(&server.uri());

    let page = client.networks().list().page_size(50).send().await.unwrap();
    assert_eq!(page.networks[0].name, "networks/ethereum-mainnet");
    assert_eq!(page.networks[0].status.as_deref(), Some("ACTIVE"));
    assert_eq!(
        page.networks[0].native_asset.as_ref().unwrap().as_str(),
        "assets/native.ethereum-mainnet"
    );
    assert!(page.networks[1].testnet);

    let all: Vec<String> = client
        .networks()
        .stream()
        .map_ok(|n| n.name)
        .try_collect()
        .await
        .unwrap();
    assert_eq!(all, vec!["networks/ethereum-mainnet", "networks/sepolia"]);
}

#[tokio::test]
async fn networks_for_vault_list_and_stream() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/abc/networks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "networks": [{ "name": "networks/ethereum-mainnet", "status": "ACTIVE" }]
        })))
        .mount(&server)
        .await;
    let client = client(&server.uri());

    let page = client
        .networks()
        .list_for_vault(VaultId::new("abc"))
        .page_size(10)
        .page_token("tok")
        .send()
        .await
        .unwrap();
    assert_eq!(page.networks.len(), 1);

    let all: Vec<_> = client
        .networks()
        .stream_for_vault(VaultId::new("abc"))
        .try_collect()
        .await
        .unwrap();
    assert_eq!(all.len(), 1);
}

#[tokio::test]
async fn networks_get_and_batch_contract() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/networks/ethereum-mainnet"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "network": { "name": "networks/ethereum-mainnet", "displayName": "Ethereum", "status": "ACTIVE" }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/networks/ethereum-mainnet/batchContracts/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "networks/ethereum-mainnet/batchContracts/x",
            "address": "0xabc", "network": "networks/ethereum-mainnet"
        })))
        .mount(&server)
        .await;
    let client = client(&server.uri());

    let net = client
        .networks()
        .get(NetworkId::new("ethereum-mainnet"))
        .await
        .unwrap();
    assert_eq!(net.display_name.as_deref(), Some("Ethereum"));

    let bc = client
        .networks()
        .latest_batch_contract(NetworkId::new("ethereum-mainnet"))
        .await
        .unwrap();
    assert_eq!(bc.address.as_deref(), Some("0xabc"));
    assert_eq!(bc.network.unwrap().as_str(), "networks/ethereum-mainnet");
}

#[tokio::test]
async fn networks_get_missing_field_is_typed_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/networks/none"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    let client = client(&server.uri());
    let err = client
        .networks()
        .get(NetworkId::new("none"))
        .await
        .unwrap_err();
    assert!(matches!(err, utilars::UtilaError::Api { code: -1, .. }));
}

#[tokio::test]
async fn networks_stream_walks_multiple_pages() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/networks"))
        .and(wiremock::matchers::query_param_is_missing("pageToken"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "networks": [{ "name": "networks/a", "displayName": "A" }],
            "nextPageToken": "p2"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/networks"))
        .and(wiremock::matchers::query_param("pageToken", "p2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "networks": [{ "name": "networks/b", "displayName": "B" }]
        })))
        .mount(&server)
        .await;
    let client = client(&server.uri());
    let all: Vec<String> = client
        .networks()
        .stream()
        .map_ok(|n| n.name)
        .try_collect()
        .await
        .unwrap();
    assert_eq!(all, vec!["networks/a", "networks/b"]);
}
