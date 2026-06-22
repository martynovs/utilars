mod common;

use common::client;
use futures::TryStreamExt;
use rust_decimal::dec;
use serde_json::json;
use utilars::{Asset, NetworkId, VaultId};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn balances_resolve_or_leave_unresolved() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/abc:queryBalances"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "balances": [
                { "asset": "assets/native.ethereum-mainnet", "rawValue": "1000000000000000000", "frozenValue": "0" },
                { "asset": "assets/unknown.token", "rawValue": "5", "frozenValue": "0" }
            ]
        })))
        .mount(&server)
        .await;
    // batchGet resolves only the ETH asset (unknown one is omitted → degrades)
    Mock::given(method("GET"))
        .and(path("/v2/assets:batchGet"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "assets": [
                { "name": "assets/native.ethereum-mainnet", "symbol": "ETH", "decimals": 18 }
            ]
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let bals = client.balances().query(VaultId::new("abc")).await.unwrap();

    // resolved asset → Resolved variant gives decimals/symbol with no Option
    match &bals[0].asset {
        Asset::Resolved(a) => {
            assert_eq!(a.symbol, "ETH");
            assert_eq!(bals[0].amount.to_decimal(a.decimals).unwrap(), dec!(1));
        }
        Asset::Unresolved(id) => panic!("expected resolved, got unresolved {id}"),
    }
    // unresolved asset → Unresolved, exact base-unit amount still present
    match &bals[1].asset {
        Asset::Unresolved(id) => assert_eq!(id.as_str(), "unknown.token"),
        Asset::Resolved(a) => panic!("expected unresolved, got resolved {}", a.id),
    }
    assert_eq!(bals[1].amount.value(), 5);
}

// ===== balances tests =====
#[tokio::test]
async fn wallet_balances_send_enriches_and_sends_filters() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/abc/wallets/w1:queryBalances"))
        .and(wiremock::matchers::body_partial_json(json!({
            "filter": "asset(\"assets/native.ethereum-mainnet\")",
            "pageSize": 10,
            "pageToken": "tok"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "walletBalances": [
                { "wallet": "vaults/abc/wallets/w1", "asset": "assets/native.ethereum-mainnet", "rawValue": "2000000000000000000", "frozenValue": "0" },
                { "wallet": "vaults/abc/wallets/w1", "asset": "assets/unknown.token", "rawValue": "7" }
            ],
            "nextPageToken": "page2",
            "totalSize": 2
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/assets:batchGet"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "assets": [ { "name": "assets/native.ethereum-mainnet", "symbol": "ETH", "decimals": 18 } ]
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let page = client
        .balances()
        .query_wallet_balances(VaultId::new("abc"), utilars::WalletId::new("w1"))
        .filter("asset(\"assets/native.ethereum-mainnet\")")
        .page_size(10)
        .page_token("tok")
        .send()
        .await
        .unwrap();

    assert_eq!(page.next_page_token.as_deref(), Some("page2"));
    assert_eq!(page.total_size, 2);
    assert_eq!(page.balances[0].wallet.to_string(), "vaults/abc/wallets/w1");
    match &page.balances[0].asset {
        Asset::Resolved(a) => {
            assert_eq!(a.symbol, "ETH");
            assert_eq!(
                page.balances[0].amount.to_decimal(a.decimals).unwrap(),
                dec!(2)
            );
        }
        Asset::Unresolved(id) => panic!("expected resolved, got {id}"),
    }
    assert_eq!(page.balances[1].amount.value(), 7);
    match &page.balances[1].asset {
        Asset::Unresolved(id) => assert_eq!(id.as_str(), "unknown.token"),
        Asset::Resolved(a) => panic!("expected unresolved, got {}", a.id),
    }
}

#[tokio::test]
async fn wallet_balances_stream_walks_all_pages() {
    let server = MockServer::start().await;
    // page 1 (no pageToken) — served once, then exhausted
    Mock::given(method("POST"))
        .and(path("/v2/vaults/abc/wallets/w1:queryBalances"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "walletBalances": [
                { "wallet": "vaults/abc/wallets/w1", "asset": "assets/native.ethereum-mainnet", "rawValue": "1" }
            ],
            "nextPageToken": "page2"
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    // page 2 (pageToken=page2) — no further token
    Mock::given(method("POST"))
        .and(path("/v2/vaults/abc/wallets/w1:queryBalances"))
        .and(wiremock::matchers::body_partial_json(json!({ "pageToken": "page2" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "walletBalances": [
                { "wallet": "vaults/abc/wallets/w1", "asset": "assets/native.ethereum-mainnet", "rawValue": "2" }
            ]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/assets:batchGet"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "assets": [ { "name": "assets/native.ethereum-mainnet", "symbol": "ETH", "decimals": 18 } ]
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let all: Vec<_> = client
        .balances()
        .query_wallet_balances(VaultId::new("abc"), utilars::WalletId::new("w1"))
        .stream()
        .try_collect()
        .await
        .unwrap();

    assert_eq!(all.len(), 2);
    assert_eq!(all[0].amount.value(), 1);
    assert_eq!(all[1].amount.value(), 2);
}

#[tokio::test]
async fn wallet_address_balances_send_enriches() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/abc/wallets/w1/addresses/a1:queryBalances"))
        .and(wiremock::matchers::body_partial_json(json!({ "pageSize": 25 })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "walletAddressBalances": [
                { "walletAddress": "vaults/abc/wallets/w1/addresses/a1", "asset": "assets/native.ethereum-mainnet", "rawValue": "1000000000000000000", "frozenValue": "0" }
            ],
            "totalSize": 1
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/assets:batchGet"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "assets": [ { "name": "assets/native.ethereum-mainnet", "symbol": "ETH", "decimals": 18 } ]
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let page = client
        .balances()
        .query_wallet_address_balances(
            VaultId::new("abc"),
            utilars::WalletId::new("w1"),
            utilars::AddressId::new("a1"),
        )
        .page_size(25)
        .send()
        .await
        .unwrap();

    assert_eq!(page.total_size, 1);
    assert_eq!(
        page.balances[0].wallet_address.to_string(),
        "vaults/abc/wallets/w1/addresses/a1"
    );
    match &page.balances[0].asset {
        Asset::Resolved(a) => assert_eq!(a.symbol, "ETH"),
        Asset::Unresolved(id) => panic!("expected resolved, got {id}"),
    }
}

#[tokio::test]
async fn wallet_address_balances_stream_walks_all_pages() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/abc/wallets/-/addresses/-:queryBalances"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "walletAddressBalances": [
                { "walletAddress": "vaults/abc/wallets/w1/addresses/a1", "asset": "assets/unknown.token", "rawValue": "3" }
            ],
            "nextPageToken": "page2"
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/abc/wallets/-/addresses/-:queryBalances"))
        .and(wiremock::matchers::body_partial_json(json!({ "pageToken": "page2" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "walletAddressBalances": [
                { "walletAddress": "vaults/abc/wallets/w2/addresses/a2", "asset": "assets/unknown.token", "rawValue": "4" }
            ]
        })))
        .mount(&server)
        .await;
    // batchGet resolves nothing — assets stay Unresolved (degrade gracefully)
    Mock::given(method("GET"))
        .and(path("/v2/assets:batchGet"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "assets": [] })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let all: Vec<_> = client
        .balances()
        .query_wallet_address_balances(
            VaultId::new("abc"),
            utilars::WalletId::new("-"),
            utilars::AddressId::new("-"),
        )
        .stream()
        .try_collect()
        .await
        .unwrap();

    assert_eq!(all.len(), 2);
    assert_eq!(all[0].amount.value(), 3);
    assert_eq!(all[1].amount.value(), 4);
    assert!(matches!(all[1].asset, Asset::Unresolved(_)));
}

#[tokio::test]
async fn wallet_utxos_send_parses_fields() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/abc/wallets/w1:queryUTXOs"))
        .and(wiremock::matchers::body_partial_json(json!({
            "network": "bitcoin-mainnet",
            "pageSize": 5
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "utxos": [
                {
                    "walletAddress": "vaults/abc/wallets/w1/addresses/a1",
                    "network": "networks/bitcoin-mainnet",
                    "txHash": "deadbeef",
                    "vout": 2,
                    "value": "0.5",
                    "state": "AVAILABLE",
                    "scriptType": "P2WPKH",
                    "confirmations": 6
                }
            ],
            "nextPageToken": "page2",
            "totalSize": 1
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let page = client
        .balances()
        .query_wallet_utxos(
            VaultId::new("abc"),
            utilars::WalletId::new("w1"),
            NetworkId::new("bitcoin-mainnet"),
        )
        .page_size(5)
        .send()
        .await
        .unwrap();

    assert_eq!(page.next_page_token.as_deref(), Some("page2"));
    assert_eq!(page.total_size, 1);
    let u = &page.utxos[0];
    assert_eq!(
        u.wallet_address.to_string(),
        "vaults/abc/wallets/w1/addresses/a1"
    );
    assert_eq!(
        u.network.as_ref().unwrap().as_str(),
        "networks/bitcoin-mainnet"
    );
    assert_eq!(u.tx_hash.as_deref(), Some("deadbeef"));
    assert_eq!(u.vout, 2);
    assert_eq!(u.value, Some(dec!(0.5)));
    assert_eq!(u.state, Some(utilars::UtxoState::Available));
    assert_eq!(u.script_type.as_deref(), Some("P2WPKH"));
    assert_eq!(u.confirmations, 6);
}

#[tokio::test]
async fn wallet_utxos_stream_walks_all_pages() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/abc/wallets/w1:queryUTXOs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "utxos": [ { "walletAddress": "vaults/abc/wallets/w1/addresses/a1", "value": "0.1" } ],
            "nextPageToken": "page2"
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/abc/wallets/w1:queryUTXOs"))
        .and(wiremock::matchers::body_partial_json(
            json!({ "pageToken": "page2" }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "utxos": [ { "walletAddress": "vaults/abc/wallets/w1/addresses/a2", "value": "0.2" } ]
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let all: Vec<_> = client
        .balances()
        .query_wallet_utxos(
            VaultId::new("abc"),
            utilars::WalletId::new("w1"),
            NetworkId::new("bitcoin-mainnet"),
        )
        .stream()
        .try_collect()
        .await
        .unwrap();

    assert_eq!(all.len(), 2);
    assert_eq!(all[0].value, Some(dec!(0.1)));
    assert_eq!(all[1].value, Some(dec!(0.2)));
}

#[tokio::test]
async fn refresh_asset_address_balance_enriches() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/abc:refreshAssetAddressBalance"))
        .and(wiremock::matchers::body_partial_json(json!({
            "asset": "assets/x",
            "address": "0xabc"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "balance": { "asset": "assets/x", "rawValue": "42", "frozenValue": "0" }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/assets:batchGet"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "assets": [ { "name": "assets/x", "symbol": "TOK", "decimals": 6 } ]
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let bal = client
        .balances()
        .refresh_asset_address_balance(VaultId::new("abc"), utilars::AssetId::new("x"), "0xabc")
        .await
        .unwrap();

    assert_eq!(bal.amount.value(), 42);
    match &bal.asset {
        Asset::Resolved(a) => assert_eq!(a.symbol, "TOK"),
        Asset::Unresolved(id) => panic!("expected resolved, got {id}"),
    }
}

// Exercises the long-tail optional builder setters (no network — builders are dropped
// unsent), so every setter line is covered without bespoke mock plumbing.
#[test]
fn optional_builder_setters_are_exercised() {
    let client = client("http://localhost");
    let _ = client
        .wallets()
        .list_addresses(VaultId::new("v"), utilars::WalletId::new("w"))
        .page_token("x");
    let _ = client
        .balances()
        .query_wallet_utxos(
            VaultId::new("v"),
            utilars::WalletId::new("w"),
            NetworkId::new("n"),
        )
        .filter("f")
        .order_by("o")
        .skip(1)
        .page_token("p");
    let _ = client
        .balances()
        .query_wallet_address_balances(
            VaultId::new("v"),
            utilars::WalletId::new("w"),
            utilars::AddressId::new("a"),
        )
        .filter("f")
        .page_token("p");
}

#[tokio::test]
async fn refresh_missing_balance_is_typed_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/abc:refreshAssetAddressBalance"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    let client = client(&server.uri());
    let err = client
        .balances()
        .refresh_asset_address_balance(VaultId::new("abc"), utilars::AssetId::new("assets/x"), "0x")
        .await
        .unwrap_err();
    assert!(matches!(err, utilars::ApiError::Api { code: -1, .. }));
}
