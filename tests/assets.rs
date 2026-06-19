mod common;

use common::client;
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ===== assets tests =====
#[tokio::test]
async fn assets_get_resolves_single() {
    let server = MockServer::start().await;
    // AssetId carries the full `assets/...` name; the Get path param is only the segment,
    // so the request path must be the prefix-stripped id.
    Mock::given(method("GET"))
        .and(path("/v2/assets/native.ethereum-mainnet"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "asset": {
                "name": "assets/native.ethereum-mainnet",
                "displayName": "Ethereum",
                "symbol": "ETH",
                "decimals": 18,
                "type": "NATIVE_CURRENCY"
            }
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let asset = client
        .assets()
        .get(utilars::AssetId::new("assets/native.ethereum-mainnet"))
        .await
        .unwrap();

    assert_eq!(asset.id.as_str(), "assets/native.ethereum-mainnet");
    assert_eq!(asset.decimals, 18);
    assert_eq!(asset.symbol, "ETH");

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
async fn assets_get_missing_asset_is_api_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/assets/native.ethereum-mainnet"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let err = client
        .assets()
        .get(utilars::AssetId::new("native.ethereum-mainnet"))
        .await
        .unwrap_err();
    assert!(
        matches!(err, utilars::UtilaError::Api { code: -1, .. }),
        "got: {err:?}"
    );
}

#[tokio::test]
async fn assets_get_for_vault_resolves() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/abc/assets/imported.usdc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "asset": {
                "name": "assets/imported.usdc",
                "symbol": "USDC",
                "decimals": 6,
                "type": "TOKEN"
            }
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let asset = client
        .assets()
        .get_for_vault(
            utilars::VaultId::new("abc"),
            utilars::AssetId::new("assets/imported.usdc"),
        )
        .await
        .unwrap();

    assert_eq!(asset.id.as_str(), "assets/imported.usdc");
    assert_eq!(asset.decimals, 6);
    assert_eq!(asset.symbol, "USDC");
}

#[tokio::test]
async fn assets_batch_get_returns_all() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/assets:batchGet"))
        .and(query_param("names", "assets/native.ethereum-mainnet"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "assets": [
                { "name": "assets/native.ethereum-mainnet", "symbol": "ETH", "decimals": 18 },
                { "name": "assets/imported.usdc", "symbol": "USDC", "decimals": 6 }
            ]
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let assets = client
        .assets()
        .batch_get([
            utilars::AssetId::new("assets/native.ethereum-mainnet"),
            utilars::AssetId::new("assets/imported.usdc"),
        ])
        .await
        .unwrap();

    assert_eq!(assets.len(), 2);
    assert_eq!(assets[0].id.as_str(), "assets/native.ethereum-mainnet");
    assert_eq!(assets[0].symbol, "ETH");
    assert_eq!(assets[1].id.as_str(), "assets/imported.usdc");
    assert_eq!(assets[1].decimals, 6);
}

#[tokio::test]
async fn assets_get_for_vault_missing_is_typed_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/v1/assets/x"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    let client = client(&server.uri());
    let err = client
        .assets()
        .get_for_vault(
            utilars::VaultId::new("v1"),
            utilars::AssetId::new("assets/x"),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, utilars::UtilaError::Api { code: -1, .. }));
}
