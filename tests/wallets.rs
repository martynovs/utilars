mod common;

use common::client;
use futures::TryStreamExt;
use serde_json::json;
use utilars::{AddressId, NetworkId, VaultId, WalletId};
use wiremock::matchers::{method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ===== wallets tests =====
#[tokio::test]
async fn wallets_list_authenticates_parses_and_paginates() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/v1/wallets"))
        .and(query_param("pageSize", "25"))
        .and(query_param("pageToken", "tok"))
        .and(query_param("showArchived", "true"))
        .and(query_param("filter", "external"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "wallets": [{
                "name": "vaults/v1/wallets/w1",
                "displayName": "Hot",
                "networks": ["networks/ethereum-mainnet", "networks/bitcoin-mainnet"],
                "external": true
            }],
            "nextPageToken": "page2",
            "totalSize": 1
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let page = client
        .wallets()
        .list(VaultId::new("v1"))
        .filter("external")
        .show_archived(true)
        .page_size(25)
        .page_token("tok")
        .send()
        .await
        .unwrap();

    assert_eq!(page.wallets[0].name.to_string(), "vaults/v1/wallets/w1");
    assert_eq!(page.wallets[0].display_name.as_deref(), Some("Hot"));
    assert_eq!(page.wallets[0].networks.len(), 2);
    assert_eq!(page.wallets[0].networks[0].as_str(), "ethereum-mainnet");
    assert!(page.wallets[0].external);
    assert_eq!(page.next_page_token.as_deref(), Some("page2"));
    assert_eq!(page.total_size, 1);

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
async fn wallets_stream_walks_all_pages() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/v1/wallets"))
        .and(query_param_is_missing("pageToken"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "wallets": [{ "name": "vaults/v1/wallets/w1", "displayName": "A", "networks": [] }],
            "nextPageToken": "page2"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/v1/wallets"))
        .and(query_param("pageToken", "page2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "wallets": [{ "name": "vaults/v1/wallets/w2", "displayName": "B", "networks": [] }]
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let names: Vec<String> = client
        .wallets()
        .list(VaultId::new("v1"))
        .stream()
        .map_ok(|w| w.name.to_string())
        .try_collect()
        .await
        .unwrap();
    assert_eq!(names, vec!["vaults/v1/wallets/w1", "vaults/v1/wallets/w2"]);
}

#[tokio::test]
async fn wallets_get_parses() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/v1/wallets/w1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "wallet": { "name": "vaults/v1/wallets/w1", "displayName": "Cold", "archived": true, "networks": [] }
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let wallet = client
        .wallets()
        .get(VaultId::new("v1"), utilars::WalletId::new("w1"))
        .await
        .unwrap();
    assert_eq!(wallet.name.to_string(), "vaults/v1/wallets/w1");
    assert_eq!(wallet.display_name.as_deref(), Some("Cold"));
    assert!(wallet.archived);
}

#[tokio::test]
async fn wallets_get_missing_field_errors() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/v1/wallets/w1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let err = client
        .wallets()
        .get(VaultId::new("v1"), utilars::WalletId::new("w1"))
        .await
        .unwrap_err();
    assert!(format!("{err}").contains("wallet missing"), "got: {err}");
}

#[tokio::test]
async fn wallets_create_sends_body_and_parses() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/v1/wallets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "wallet": { "name": "vaults/v1/wallets/new", "displayName": "Fresh", "networks": [] }
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let wallet = client
        .wallets()
        .create(VaultId::new("v1"), "Fresh")
        .network(NetworkId::new("ethereum-mainnet"))
        .external(false)
        .send()
        .await
        .unwrap();
    assert_eq!(wallet.name.to_string(), "vaults/v1/wallets/new");

    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = reqs[0].body_json().unwrap();
    assert_eq!(body["displayName"], "Fresh");
    assert_eq!(body["networks"][0], "networks/ethereum-mainnet");
    assert_eq!(body["external"], false);
}

#[tokio::test]
async fn wallets_archive_and_unarchive_send_allow_missing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/v1/wallets/w1:archive"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/v1/wallets/w1:unarchive"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    client
        .wallets()
        .archive(VaultId::new("v1"), utilars::WalletId::new("w1"))
        .allow_missing(true)
        .send()
        .await
        .unwrap();
    client
        .wallets()
        .unarchive(VaultId::new("v1"), utilars::WalletId::new("w1"))
        .send()
        .await
        .unwrap();

    let reqs = server.received_requests().await.unwrap();
    let archive_body: serde_json::Value = reqs[0].body_json().unwrap();
    assert_eq!(archive_body["allowMissing"], true);
}

#[tokio::test]
async fn wallets_batch_archive_and_unarchive_send_names() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/v1/wallets:batchArchive"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/v1/wallets:batchUnarchive"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    client
        .wallets()
        .batch_archive(VaultId::new("v1"), &[WalletId::new("w1")])
        .allow_missing(true)
        .send()
        .await
        .unwrap();
    client
        .wallets()
        .batch_unarchive(VaultId::new("v1"), &[WalletId::new("w2")])
        .send()
        .await
        .unwrap();

    let reqs = server.received_requests().await.unwrap();
    let archive_body: serde_json::Value = reqs[0].body_json().unwrap();
    assert_eq!(archive_body["names"][0], "vaults/v1/wallets/w1");
    assert_eq!(archive_body["allowMissing"], true);
    let unarchive_body: serde_json::Value = reqs[1].body_json().unwrap();
    assert_eq!(unarchive_body["names"][0], "vaults/v1/wallets/w2");
}

#[tokio::test]
async fn wallets_batch_get_parses() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/v1/wallets:batchGet"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "wallets": [
                { "name": "vaults/v1/wallets/w1", "displayName": "One", "networks": [] },
                { "name": "vaults/v1/wallets/w2", "displayName": "Two", "networks": [] }
            ]
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let wallets = client
        .wallets()
        .batch_get(
            VaultId::new("v1"),
            &[WalletId::new("w1"), WalletId::new("w2")],
        )
        .await
        .unwrap();
    assert_eq!(wallets.len(), 2);
    assert_eq!(wallets[1].display_name.as_deref(), Some("Two"));
}

#[tokio::test]
async fn wallet_addresses_list_and_stream() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/v1/wallets/w1/addresses"))
        .and(query_param_is_missing("pageToken"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "walletAddresses": [{
                "name": "vaults/v1/wallets/w1/addresses/a1",
                "address": "bc1qexample",
                "network": "networks/bitcoin-mainnet",
                "format": "BITCOIN_P2WPKH",
                "type": "DEPOSIT"
            }],
            "nextPageToken": "p2",
            "totalSize": 2
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/v1/wallets/w1/addresses"))
        .and(query_param("pageToken", "p2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "walletAddresses": [{
                "name": "vaults/v1/wallets/w1/addresses/a2",
                "network": "networks/bitcoin-mainnet"
            }]
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let page = client
        .wallets()
        .list_addresses(VaultId::new("v1"), utilars::WalletId::new("w1"))
        .page_size(10)
        .send()
        .await
        .unwrap();
    assert_eq!(page.addresses[0].address.as_deref(), Some("bc1qexample"));
    assert_eq!(page.addresses[0].network.as_str(), "bitcoin-mainnet");
    assert_eq!(page.addresses[0].format.as_deref(), Some("BITCOIN_P2WPKH"));
    assert_eq!(page.addresses[0].kind.as_deref(), Some("DEPOSIT"));

    let names: Vec<String> = client
        .wallets()
        .list_addresses(VaultId::new("v1"), utilars::WalletId::new("w1"))
        .stream()
        .map_ok(|a| a.name.to_string())
        .try_collect()
        .await
        .unwrap();
    assert_eq!(
        names,
        vec![
            "vaults/v1/wallets/w1/addresses/a1",
            "vaults/v1/wallets/w1/addresses/a2"
        ]
    );
}

#[tokio::test]
async fn wallet_address_get_parses() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/v1/wallets/w1/addresses/a1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "walletAddress": {
                "name": "vaults/v1/wallets/w1/addresses/a1",
                "address": "0xabc",
                "network": "networks/ethereum-mainnet",
                "note": "savings"
            }
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let addr = client
        .wallets()
        .get_address(
            VaultId::new("v1"),
            utilars::WalletId::new("w1"),
            utilars::AddressId::new("a1"),
        )
        .await
        .unwrap();
    assert_eq!(addr.address.as_deref(), Some("0xabc"));
    assert_eq!(addr.note.as_deref(), Some("savings"));
}

#[tokio::test]
async fn wallet_address_create_sends_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/v1/wallets/w1/addresses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "walletAddress": {
                "name": "vaults/v1/wallets/w1/addresses/new",
                "address": "0xnew",
                "network": "networks/ethereum-mainnet"
            }
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let addr = client
        .wallets()
        .create_address(
            VaultId::new("v1"),
            utilars::WalletId::new("w1"),
            NetworkId::new("ethereum-mainnet"),
        )
        .display_name("Deposit 1")
        .note("primary")
        .send()
        .await
        .unwrap();
    assert_eq!(addr.name.to_string(), "vaults/v1/wallets/w1/addresses/new");

    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = reqs[0].body_json().unwrap();
    assert_eq!(body["network"], "networks/ethereum-mainnet");
    assert_eq!(body["displayName"], "Deposit 1");
    assert_eq!(body["note"], "primary");
}

#[tokio::test]
async fn wallet_addresses_batch_get_parses() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/v1/wallets/w1/addresses:batchGet"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "walletAddresses": [
                { "name": "vaults/v1/wallets/w1/addresses/a1", "network": "networks/bitcoin-mainnet" },
                { "name": "vaults/v1/wallets/w1/addresses/a2", "network": "networks/bitcoin-mainnet" }
            ]
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let addrs = client
        .wallets()
        .batch_get_addresses(
            VaultId::new("v1"),
            WalletId::new("w1"),
            &[AddressId::new("a1"), AddressId::new("a2")],
        )
        .await
        .unwrap();
    assert_eq!(addrs.len(), 2);
    assert_eq!(
        addrs[0].name.to_string(),
        "vaults/v1/wallets/w1/addresses/a1"
    );
}

#[tokio::test]
async fn wallet_address_get_missing_is_typed_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/v1/wallets/w1/addresses/missing"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    let client = client(&server.uri());
    let err = client
        .wallets()
        .get_address(
            VaultId::new("v1"),
            utilars::WalletId::new("w1"),
            utilars::AddressId::new("missing"),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, utilars::ApiError::Api { code: -1, .. }));
}

#[tokio::test]
async fn wallet_create_missing_wallet_is_typed_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/v1/wallets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    let client = client(&server.uri());
    let err = client
        .wallets()
        .create(VaultId::new("v1"), "X")
        .send()
        .await
        .unwrap_err();
    assert!(matches!(err, utilars::ApiError::Api { code: -1, .. }));
}

#[tokio::test]
async fn wallet_create_address_missing_is_typed_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/v1/wallets/w1/addresses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    let client = client(&server.uri());
    let err = client
        .wallets()
        .create_address(
            VaultId::new("v1"),
            utilars::WalletId::new("w1"),
            NetworkId::new("networks/x"),
        )
        .send()
        .await
        .unwrap_err();
    assert!(matches!(err, utilars::ApiError::Api { code: -1, .. }));
}
