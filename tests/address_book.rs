mod common;

use common::client;
use futures::TryStreamExt;
use serde_json::json;
use utilars::{NetworkId, VaultId};
use wiremock::matchers::{method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ===== address_book tests =====
#[tokio::test]
async fn address_book_list_passes_filter_and_parses() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/v1/addressBookEntries"))
        .and(query_param(
            "filter",
            "network(\"networks/ethereum-mainnet\")",
        ))
        .and(query_param("orderBy", "create_time desc"))
        .and(query_param("pageSize", "25"))
        .and(query_param("pageToken", "tok"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "addressBookEntries": [{
                "name": "vaults/v1/addressBookEntries/e1",
                "displayName": "Cold Storage",
                "address": "0xabc",
                "network": "networks/ethereum-mainnet",
                "note": "treasury",
                "tracked": true,
                "associatedExternalWallet": "vaults/v1/wallets/w1",
                "creator": "users/u1",
                "createTime": "2023-08-29T18:04:00Z"
            }],
            "nextPageToken": "page2",
            "totalSize": 1
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let page = client
        .address_book()
        .list(VaultId::new("v1"))
        .filter("network(\"networks/ethereum-mainnet\")")
        .order_by("create_time desc")
        .page_size(25)
        .page_token("tok")
        .send()
        .await
        .unwrap();

    assert_eq!(page.total_size, 1);
    assert_eq!(page.next_page_token.as_deref(), Some("page2"));
    let e = &page.entries[0];
    assert_eq!(e.name.to_string(), "vaults/v1/addressBookEntries/e1");
    assert_eq!(e.display_name.as_deref(), Some("Cold Storage"));
    assert_eq!(e.address, "0xabc");
    assert_eq!(
        e.network.as_ref().map(NetworkId::as_str),
        Some("ethereum-mainnet")
    );
    assert_eq!(e.note.as_deref(), Some("treasury"));
    assert!(e.tracked);
    assert_eq!(
        e.associated_wallet
            .as_ref()
            .map(ToString::to_string)
            .as_deref(),
        Some("vaults/v1/wallets/w1")
    );
    assert_eq!(
        e.creator.as_ref().map(ToString::to_string).as_deref(),
        Some("users/u1")
    );
    assert!(e.create_time.is_some());
}

#[tokio::test]
async fn address_book_list_omits_unset_params() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/v1/addressBookEntries"))
        .and(query_param_is_missing("filter"))
        .and(query_param_is_missing("orderBy"))
        .and(query_param_is_missing("pageSize"))
        .and(query_param_is_missing("pageToken"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "addressBookEntries": [{
                "name": "vaults/v1/addressBookEntries/e1",
                "displayName": "A",
                "address": "0xabc",
                "network": "networks/ethereum-mainnet"
            }]
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let page = client
        .address_book()
        .list(VaultId::new("v1"))
        .send()
        .await
        .unwrap();
    assert_eq!(page.entries.len(), 1);
    // unset note/tracked/creator collapse to defaults
    assert!(!page.entries[0].tracked);
    assert!(page.entries[0].note.is_none());
}

#[tokio::test]
async fn address_book_stream_walks_all_pages() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/v1/addressBookEntries"))
        .and(query_param_is_missing("pageToken"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "addressBookEntries": [{
                "name": "vaults/v1/addressBookEntries/e1",
                "displayName": "A", "address": "0x1", "network": "networks/ethereum-mainnet"
            }],
            "nextPageToken": "page2"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/v1/addressBookEntries"))
        .and(query_param("pageToken", "page2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "addressBookEntries": [{
                "name": "vaults/v1/addressBookEntries/e2",
                "displayName": "B", "address": "0x2", "network": "networks/ethereum-mainnet"
            }]
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let names: Vec<String> = client
        .address_book()
        .stream(VaultId::new("v1"))
        .map_ok(|e| e.name.to_string())
        .try_collect()
        .await
        .unwrap();
    assert_eq!(
        names,
        vec![
            "vaults/v1/addressBookEntries/e1".to_string(),
            "vaults/v1/addressBookEntries/e2".to_string()
        ]
    );
}

#[tokio::test]
async fn address_book_get_many_batches_by_name() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/vaults/v1/addressBookEntries:batchGet"))
        .and(query_param("names", "vaults/v1/addressBookEntries/e1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "addressBookEntries": [{
                "name": "vaults/v1/addressBookEntries/e1",
                "displayName": "A", "address": "0x1", "network": "networks/ethereum-mainnet"
            }]
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let entries = client
        .address_book()
        .get_many(
            VaultId::new("v1"),
            &[utilars::AddressBookEntryId::new("e1")],
        )
        .await
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].name.to_string(),
        "vaults/v1/addressBookEntries/e1"
    );
}

#[tokio::test]
async fn address_book_batch_create_returns_action() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/v1/addressBookEntries:batchCreate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "vaultAction": {
                "name": "vaults/v1/actions/a1",
                "status": "PENDING",
                "createTime": "2023-08-29T18:04:00Z",
                "expireTime": "2023-08-30T18:04:00Z"
            }
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let action = client
        .address_book()
        .batch_create(
            VaultId::new("v1"),
            vec![utilars::NewAddressBookEntry {
                address: "0x1".into(),
                display_name: "A".into(),
                network: NetworkId::new("networks/ethereum-mainnet"),
                note: None,
            }],
        )
        .await
        .unwrap();
    assert_eq!(action.name.to_string(), "vaults/v1/actions/a1");
    assert_eq!(action.status, Some(utilars::VaultActionStatus::Pending));
    assert!(action.expire_time.is_some());
}

#[tokio::test]
async fn address_book_batch_create_missing_action_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/v1/addressBookEntries:batchCreate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let err = client
        .address_book()
        .batch_create(
            VaultId::new("v1"),
            vec![utilars::NewAddressBookEntry {
                address: "0x1".into(),
                display_name: "A".into(),
                network: NetworkId::new("networks/ethereum-mainnet"),
                note: None,
            }],
        )
        .await
        .unwrap_err();
    assert!(matches!(err, utilars::ApiError::Api { code: -1, .. }));
}

#[tokio::test]
async fn address_book_batch_create_unsigned_returns_action() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/v1/addressBookEntries:unsignedBatchCreate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "vaultAction": { "name": "vaults/v1/actions/a2", "status": "AWAITING_SIGNATURE" }
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let action = client
        .address_book()
        .batch_create_unsigned(
            VaultId::new("v1"),
            vec![utilars::NewAddressBookEntry {
                address: "0x1".into(),
                display_name: "A".into(),
                network: NetworkId::new("networks/ethereum-mainnet"),
                note: Some("n".into()),
            }],
        )
        .await
        .unwrap();
    assert_eq!(action.name.to_string(), "vaults/v1/actions/a2");
    assert_eq!(
        action.status,
        Some(utilars::VaultActionStatus::AwaitingSignature)
    );
}

#[tokio::test]
async fn address_book_batch_add_to_group_returns_action() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/vaults/v1/addressBookEntries:batchAdd"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "vaultAction": { "name": "vaults/v1/actions/a3", "status": "PENDING" }
        })))
        .mount(&server)
        .await;

    let client = client(&server.uri());
    let action = client
        .address_book()
        .batch_add_to_group(
            VaultId::new("v1"),
            utilars::AddressBookEntryGroupId::new("g1"),
            &[utilars::AddressBookEntryId::new("e1")],
        )
        .await
        .unwrap();
    assert_eq!(action.name.to_string(), "vaults/v1/actions/a3");
}
