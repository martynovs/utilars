mod common;

use common::client;
use futures::TryStreamExt;
use rust_decimal::dec;
use serde_json::json;
use utilars::{AssetTransfer, NetworkId, Priority, TransactionDetails, UserRef, VaultId};
use wiremock::matchers::{method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn initiate_sends_exactly_one_detail_and_idempotency_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/abc/transactions:initiate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "transaction": { "name": "vaults/abc/transactions/t1", "state": "AWAITING_APPROVAL" }
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let out = client
        .transactions()
        .asset_transfer(
            VaultId::new("abc"),
            AssetTransfer {
                asset: "native.ethereum-mainnet".into(),
                source: "vaults/abc/wallets/w1".into(),
                destination: "0xabc".into(),
                amount: dec!(1.5),
                memo: None,
                sponsor: None,
                pay_fee_from_amount: None,
                stellar_memo: None,
                xrpl_destination_tag: None,
            },
        )
        .priority(Priority::High)
        .note("payroll")
        .external_id("ext-1")
        .validate_only(false)
        .run_simulation(false)
        .send()
        .await
        .unwrap();

    assert!(!out.request_id.is_empty());
    assert_eq!(
        out.transaction.unwrap().name.to_string(),
        "vaults/abc/transactions/t1"
    );

    // body had exactly one detail variant + an auto-generated requestId + HIGH priority
    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    let details = body["details"].as_object().unwrap();
    assert_eq!(details.len(), 1);
    assert!(details.contains_key("assetTransfer"));
    assert_eq!(body["details"]["assetTransfer"]["amount"], "1.5");
    assert_eq!(body["priority"], "HIGH");
    assert_eq!(body["requestId"], out.request_id);
}

#[tokio::test]
async fn initiate_request_id_override_is_used() {
    let server = MockServer::start().await;
    let fixed = "4ba6b3b1-ee91-4dcf-a4b3-e4487d7f0f46";
    Mock::given(method("POST"))
        .and(path("/v2/vaults/abc/transactions:initiate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "transaction": null })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let out = client
        .transactions()
        .asset_transfer(
            VaultId::new("abc"),
            AssetTransfer {
                asset: "x".into(),
                source: "s".into(),
                destination: "d".into(),
                amount: dec!(1),
                memo: None,
                sponsor: None,
                pay_fee_from_amount: None,
                stellar_memo: None,
                xrpl_destination_tag: None,
            },
        )
        .request_id(fixed)
        .send()
        .await
        .unwrap();

    assert_eq!(out.request_id, fixed);
}

// ===== transactions tests =====
#[tokio::test]
async fn transactions_get_curates_core_fields() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/abc/transactions/tx1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "transaction": {
                "name": "vaults/abc/transactions/tx1",
                "network": "networks/ethereum-mainnet",
                "state": "MINED",
                "type": "TRANSACTION",
                "subType": "NATIVE_TRANSFER",
                "direction": "OUTGOING",
                "hash": "0xdead",
                "spam": false,
                "designatedSigners": ["users/me@example.com"],
                "transfers": [{
                    "asset": "assets/native.ethereum-mainnet",
                    "amount": "1.5",
                    "sourceAddress": { "value": "0xaaa" },
                    "destinationAddress": { "value": "0xbbb" }
                }],
                "request": {
                    "name": "vaults/abc/transactionRequests/r1",
                    "externalId": "order-1",
                    "initiator": "users/me@example.com",
                    "origin": "WALLET_CONNECT"
                }
            }
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let tx = client
        .transactions()
        .get(VaultId::new("abc"), utilars::TransactionId::new("tx1"))
        .await
        .unwrap();

    assert_eq!(tx.name.to_string(), "vaults/abc/transactions/tx1");
    assert_eq!(tx.state, Some(utilars::TransactionState::Mined));
    assert_eq!(tx.kind, Some(utilars::TransactionKind::Transaction));
    assert_eq!(tx.sub_type.as_deref(), Some("NATIVE_TRANSFER"));
    assert_eq!(tx.direction.as_deref(), Some("OUTGOING"));
    assert_eq!(
        tx.network.as_ref().map(NetworkId::as_str),
        Some("networks/ethereum-mainnet")
    );
    assert_eq!(tx.designated_signers.len(), 1);
    assert_eq!(tx.designated_signers[0].to_string(), "users/me@example.com");
    assert_eq!(tx.transfers.len(), 1);
    assert_eq!(
        tx.transfers[0].destination_address.as_deref(),
        Some("0xbbb")
    );
    let req = tx.request.unwrap();
    assert_eq!(req.external_id.as_deref(), Some("order-1"));
    assert_eq!(req.origin.as_deref(), Some("WALLET_CONNECT"));
}

#[tokio::test]
async fn transactions_list_send_with_filter_and_pagination() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/abc/transactions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "transactions": [{ "name": "vaults/abc/transactions/tx1", "state": "SIGNED" }],
            "nextPageToken": "page2",
            "totalSize": 1
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let page = client
        .transactions()
        .list(VaultId::new("abc"))
        .filter("state(SIGNED)")
        .order_by("create_time desc")
        .page_size(25)
        .page_token("tok")
        .send()
        .await
        .unwrap();

    assert_eq!(
        page.transactions[0].name.to_string(),
        "vaults/abc/transactions/tx1"
    );
    assert_eq!(
        page.transactions[0].state,
        Some(utilars::TransactionState::Signed)
    );
    assert_eq!(page.next_page_token.as_deref(), Some("page2"));
    assert_eq!(page.total_size, 1);

    let reqs = server.received_requests().await.unwrap();
    let url = reqs[0].url.to_string();
    assert!(url.contains("filter="), "got: {url}");
    assert!(url.contains("pageSize=25"), "got: {url}");
    assert!(url.contains("pageToken=tok"), "got: {url}");
}

#[tokio::test]
async fn transactions_stream_walks_all_pages() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/abc/transactions"))
        .and(query_param_is_missing("pageToken"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "transactions": [{ "name": "vaults/abc/transactions/tx1" }],
            "nextPageToken": "page2"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/abc/transactions"))
        .and(query_param("pageToken", "page2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "transactions": [{ "name": "vaults/abc/transactions/tx2" }]
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let names: Vec<String> = client
        .transactions()
        .list(VaultId::new("abc"))
        .stream()
        .map_ok(|t| t.name.to_string())
        .try_collect()
        .await
        .unwrap();
    assert_eq!(
        names,
        vec![
            "vaults/abc/transactions/tx1".to_string(),
            "vaults/abc/transactions/tx2".to_string()
        ]
    );
}

#[tokio::test]
async fn transactions_batch_get_returns_all() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/abc/transactions:batchGet"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "transactions": [
                { "name": "vaults/abc/transactions/tx1" },
                { "name": "vaults/abc/transactions/tx2" }
            ]
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let txs = client
        .transactions()
        .batch_get(
            VaultId::new("abc"),
            &[
                utilars::TransactionId::new("tx1"),
                utilars::TransactionId::new("tx2"),
            ],
        )
        .await
        .unwrap();
    assert_eq!(txs.len(), 2);
    assert_eq!(txs[1].name.to_string(), "vaults/abc/transactions/tx2");
}

#[tokio::test]
async fn transactions_cancel_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/abc/transactions/tx1:cancel"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    let client = client(&server.uri());
    client
        .transactions()
        .cancel(VaultId::new("abc"), utilars::TransactionId::new("tx1"))
        .await
        .unwrap();
}

#[tokio::test]
async fn transactions_publish_returns_transaction() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/abc/transactions/tx1:publish"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "transaction": { "name": "vaults/abc/transactions/tx1", "state": "PUBLISHED" }
        })))
        .mount(&server)
        .await;
    let client = client(&server.uri());
    let tx = client
        .transactions()
        .publish(VaultId::new("abc"), utilars::TransactionId::new("tx1"))
        .await
        .unwrap();
    assert_eq!(tx.state, Some(utilars::TransactionState::Published));
}

#[tokio::test]
async fn transactions_replace_with_note_sends_type() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/abc/transactions/tx1:replace"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "transaction": { "name": "vaults/abc/transactions/tx2", "state": "AWAITING_SIGNATURE" }
        })))
        .mount(&server)
        .await;
    let client = client(&server.uri());
    let tx = client
        .transactions()
        .replace(
            VaultId::new("abc"),
            utilars::TransactionId::new("tx1"),
            utilars::ReplacementType::Accelerate,
        )
        .note("bump the fee")
        .designated_signers(vec![UserRef::new("me@example.com")])
        .send()
        .await
        .unwrap();
    assert_eq!(tx.name.to_string(), "vaults/abc/transactions/tx2");

    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["type"], "ACCELERATE");
    assert_eq!(body["note"], "bump the fee");
    assert_eq!(body["designatedSigners"][0], "users/me@example.com");
}

#[tokio::test]
async fn transactions_vote_returns_state_and_sends_vote() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/abc/transactionRequests/r1:vote"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "transactionState": "AWAITING_SIGNATURE"
        })))
        .mount(&server)
        .await;
    let client = client(&server.uri());
    let state = client
        .transactions()
        .vote(VaultId::new("abc"), "r1", utilars::Vote::Approve)
        .await
        .unwrap();
    assert_eq!(state.as_deref(), Some("AWAITING_SIGNATURE"));

    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["vote"], "APPROVE");
}

#[tokio::test]
async fn transactions_latest_simulation_curates_balance_changes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/abc/transactions/tx1/simulations/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "transactionSimulation": {
                "name": "vaults/abc/transactionSimulations/s1",
                "addressBalanceChanges": [{
                    "address": { "value": "0xaaa" },
                    "balanceChanges": [{
                        "asset": "assets/native.ethereum-mainnet",
                        "amount": "-1.5",
                        "negative": true
                    }]
                }]
            }
        })))
        .mount(&server)
        .await;
    let client = client(&server.uri());
    let sim = client
        .transactions()
        .latest_simulation(VaultId::new("abc"), utilars::TransactionId::new("tx1"))
        .await
        .unwrap();
    assert_eq!(sim.name.to_string(), "vaults/abc/transactionSimulations/s1");
    assert_eq!(sim.balance_changes[0].address.as_deref(), Some("0xaaa"));
    assert!(sim.balance_changes[0].changes[0].negative);
    assert_eq!(
        sim.balance_changes[0].changes[0].amount.as_deref(),
        Some("-1.5")
    );
}

#[tokio::test]
async fn transactions_aml_screening_curates_provider() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/abc/transactions/tx1/amlScreening"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "amlScreening": { "provider": "CHAINALYSIS", "rawResponses": ["{}"] }
        })))
        .mount(&server)
        .await;
    let client = client(&server.uri());
    let aml = client
        .transactions()
        .aml_screening(VaultId::new("abc"), utilars::TransactionId::new("tx1"))
        .await
        .unwrap();
    assert_eq!(aml.provider.as_deref(), Some("CHAINALYSIS"));
    assert_eq!(aml.raw_responses, vec!["{}".to_string()]);
}

#[tokio::test]
async fn transactions_estimate_fee_with_priority() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/abc/transactions:estimateFee"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "totalFee": "0.0021",
            "gasless": false,
            "convertedTotalFee": { "amount": "5.10", "currencyCode": "USD" },
            "evmFee": { "gasPrice": "20000000000", "gasUsed": "21000" },
            "tronFee": { "bandwidthUsed": "100", "energyUsed": "5000", "totalSunUsed": "1100000" }
        })))
        .mount(&server)
        .await;
    let client = client(&server.uri());
    let est = client
        .transactions()
        .estimate_fee(
            VaultId::new("abc"),
            TransactionDetails::AssetTransfer(AssetTransfer {
                asset: utilars::AssetId::new("native.ethereum-mainnet"),
                source: "vaults/abc/wallets/w1".into(),
                destination: "0xbbb".into(),
                amount: dec!(1.5),
                memo: None,
                sponsor: None,
                pay_fee_from_amount: None,
                stellar_memo: None,
                xrpl_destination_tag: None,
            }),
        )
        .priority(Priority::High)
        .send()
        .await
        .unwrap();
    assert_eq!(est.total_fee.as_deref(), Some("0.0021"));
    assert!(!est.gasless);
    assert_eq!(est.converted_total_fee, Some(dec!(5.10)));
    assert_eq!(est.evm_fee.unwrap().gas_used.as_deref(), Some("21000"));
    assert_eq!(
        est.tron_fee.unwrap().total_sun_used.as_deref(),
        Some("1100000")
    );

    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["priority"], "HIGH");
    assert!(body["details"]["assetTransfer"]["amount"].is_string());
}

#[tokio::test]
async fn transaction_get_missing_field_is_typed_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/abc/transactions/tx404"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    let client = client(&server.uri());
    let err = client
        .transactions()
        .get(VaultId::new("abc"), utilars::TransactionId::new("tx404"))
        .await
        .unwrap_err();
    assert!(matches!(err, utilars::ApiError::Api { code: -1, .. }));
}

#[tokio::test]
async fn transactions_stream_honors_starting_page_token() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/abc/transactions"))
        .and(query_param("pageToken", "start"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "transactions": [{ "name": "vaults/abc/transactions/tx9" }]
        })))
        .mount(&server)
        .await;
    let client = client(&server.uri());
    let names: Vec<String> = client
        .transactions()
        .list(VaultId::new("abc"))
        .page_token("start")
        .stream()
        .map_ok(|t| t.name.to_string())
        .try_collect()
        .await
        .unwrap();
    assert_eq!(names, vec!["vaults/abc/transactions/tx9".to_string()]);
}
