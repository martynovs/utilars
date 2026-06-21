//! The transaction `details` oneof as a typed [`TransactionDetails`] enum, one variant per
//! API message, plus the shared transfer-endpoint / sponsor types and the mappings onto the
//! generated `details` envelopes.
//!
//! The per-kind input structs live in sibling modules ([`super::evm`], [`super::tron`],
//! [`super::stellar`], …); flat kinds are plain structs with non-`Option` required fields, while
//! the two deeply-nested kinds ([`super::TronTransaction`], [`super::StellarTransaction`]) use a
//! `::builder()…​.build()?` whose `Result` is the single place a missing required field is
//! reported, so the send path never fails for "you forgot a field".

use crate::generated::types as g;
use crate::resource::{AddressId, GasStationId, VaultId, WalletId};
use crate::resource::{GasStationRef, WalletAddressRef, WalletRef};

use super::evm::{EvmAccountDelegation, EvmPersonalSign, EvmTransaction, EvmTypedData};
use super::solana::SolanaRaw;
use super::stellar::{StellarRaw, StellarTransaction};
use super::sui::SuiRaw;
use super::transfer::{AssetTransfer, BatchAssetTransfer, ExchangeWithdrawal};
use super::tron::{TronTransaction, TronTriggerSmartContract};
use super::xrpl::XrplRaw;

/// Where a transfer's asset moves from or to — the `source` / `destination` of a transfer. The
/// API accepts a raw on-chain address **or** a Utila resource name; this is the typed form. Build
/// from a string with `.into()` (resource names classify; anything else is an [`Address`]).
///
/// A gas station is **not** a transfer endpoint (it pays fees, it doesn't send/receive) — see
/// [`Sponsor`].
///
/// [`Address`]: TransferEndpoint::Address
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TransferEndpoint {
    /// A raw on-chain address (`0x…`, `bc1…`).
    Address(String),
    /// A Utila wallet.
    Wallet(WalletRef),
    /// A specific wallet address.
    WalletAddress(WalletAddressRef),
}

impl TransferEndpoint {
    /// The wire string the API expects.
    pub(crate) fn to_wire(&self) -> String {
        match self {
            Self::Address(a) => a.clone(),
            Self::Wallet(w) => w.to_string(),
            Self::WalletAddress(w) => w.to_string(),
        }
    }
}

impl From<&str> for TransferEndpoint {
    fn from(s: &str) -> Self {
        match s.split('/').collect::<Vec<_>>().as_slice() {
            ["vaults", v, "wallets", w, "addresses", a] => Self::WalletAddress(WalletAddressRef {
                vault: VaultId::new(*v),
                wallet: WalletId::new(*w),
                address: AddressId::new(*a),
            }),
            ["vaults", v, "wallets", w] => Self::Wallet(WalletRef {
                vault: VaultId::new(*v),
                wallet: WalletId::new(*w),
            }),
            _ => Self::Address(s.to_owned()),
        }
    }
}

impl From<String> for TransferEndpoint {
    fn from(s: String) -> Self {
        Self::from(s.as_str())
    }
}

/// Who pays a transfer's network fee (a *sponsored transfer*): any [`TransferEndpoint`] with a
/// balance, or a dedicated gas station. Build from a string with `.into()` (a `gasStations/…`
/// resource name becomes [`Sponsor::GasStation`]; everything else classifies as a
/// [`TransferEndpoint`]).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Sponsor {
    /// A wallet / wallet address / raw address that pays the fee.
    Endpoint(TransferEndpoint),
    /// A dedicated gas station.
    GasStation(GasStationRef),
}

impl Sponsor {
    pub(crate) fn to_wire(&self) -> String {
        match self {
            Self::Endpoint(e) => e.to_wire(),
            Self::GasStation(g) => g.to_string(),
        }
    }
}

impl From<&str> for Sponsor {
    fn from(s: &str) -> Self {
        match s.split('/').collect::<Vec<_>>().as_slice() {
            ["vaults", v, "gasStations", g] => Self::GasStation(GasStationRef {
                vault: VaultId::new(*v),
                gas_station: GasStationId::new(*g),
            }),
            _ => Self::Endpoint(TransferEndpoint::from(s)),
        }
    }
}

impl From<String> for Sponsor {
    fn from(s: String) -> Self {
        Self::from(s.as_str())
    }
}

/// The transaction to create — exactly one variant (the API's `details` oneof). Modeled as
/// an enum so zero or multiple detail types are unrepresentable. Construct it through the
/// per-kind methods on [`crate::Transactions`] (`transactions().tron(..)`, `.evm(..)`, …)
/// rather than building the enum by hand.
#[derive(Debug, Clone)]
pub enum TransactionDetails {
    AssetTransfer(AssetTransfer),
    BatchAssetTransfer(BatchAssetTransfer),
    Evm(EvmTransaction),
    EvmPersonalSign(EvmPersonalSign),
    EvmTypedData(EvmTypedData),
    EvmAccountDelegation(EvmAccountDelegation),
    ExchangeWithdrawal(ExchangeWithdrawal),
    SolanaRaw(SolanaRaw),
    Stellar(StellarTransaction),
    StellarRaw(StellarRaw),
    SuiRaw(SuiRaw),
    Tron(TronTransaction),
    TronTriggerContract(TronTriggerSmartContract),
    XrplRaw(XrplRaw),
}

/// Map the typed [`TransactionDetails`] onto the generated all-`Option` initiate `details`
/// struct, setting exactly one field so it serializes to a single-key object.
pub(crate) fn map_details(details: TransactionDetails) -> g::V2InitiateTransactionRequestDetails {
    let mut out = g::V2InitiateTransactionRequestDetails::default();
    match details {
        TransactionDetails::AssetTransfer(t) => out.asset_transfer = Some(t.into()),
        TransactionDetails::BatchAssetTransfer(t) => out.asset_batch_transfer = Some(t.into()),
        TransactionDetails::Evm(t) => out.evm_transaction = Some(t.into()),
        TransactionDetails::EvmPersonalSign(t) => out.evm_personal_sign = Some(t.into()),
        TransactionDetails::EvmTypedData(t) => out.evm_sign_typed_data_v4 = Some(t.into()),
        TransactionDetails::EvmAccountDelegation(t) => out.evm_account_delegation = Some(t.into()),
        TransactionDetails::ExchangeWithdrawal(t) => out.exchange_withdrawal = Some(t.into()),
        TransactionDetails::SolanaRaw(t) => out.solana_serialized_transaction = Some(t.into()),
        TransactionDetails::Stellar(t) => out.stellar_transaction = Some(t.into()),
        TransactionDetails::StellarRaw(t) => out.stellar_raw_transaction = Some(t.into()),
        TransactionDetails::SuiRaw(t) => out.sui_raw_transaction = Some(t.into()),
        TransactionDetails::Tron(t) => out.tron_transaction = Some(t.into()),
        TransactionDetails::TronTriggerContract(t) => {
            out.tron_trigger_smart_contract = Some(t.into());
        }
        TransactionDetails::XrplRaw(t) => out.xrpl_raw_transaction = Some(t.into()),
    }
    out
}

/// Map [`TransactionDetails`] onto the fee-estimation `details` oneof. Only the kinds the
/// estimate endpoint accepts are wired; others leave the struct empty (the server rejects).
pub(crate) fn map_estimate_details(
    details: TransactionDetails,
) -> g::V2EstimateTransactionFeeRequestDetails {
    let mut out = g::V2EstimateTransactionFeeRequestDetails::default();
    match details {
        TransactionDetails::AssetTransfer(t) => out.asset_transfer = Some(t.into()),
        TransactionDetails::BatchAssetTransfer(t) => out.asset_batch_transfer = Some(t.into()),
        TransactionDetails::Evm(t) => out.evm_transaction = Some(t.into()),
        TransactionDetails::Tron(t) => out.tron_transaction = Some(t.into()),
        TransactionDetails::TronTriggerContract(t) => {
            out.tron_trigger_smart_contract = Some(t.into());
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ApiError;
    use crate::resource::{
        AddressId, GasStationId, GasStationRef, VaultId, WalletAddressRef, WalletId, WalletRef,
    };
    use crate::transactions::{
        BatchDestination, StellarMemo, StellarMemoType, StellarOpBody, StellarOperation,
        StellarTimeBounds, TronAction, TronContractCall, TronDelegate, TronFreeze, TronResource,
        TronUndelegate, TronVote,
    };
    use rust_decimal::dec;
    use serde_json::Value;

    /// Map a detail to JSON and return its single top-level key + value (asserting exactly
    /// one variant was set — the oneof invariant).
    fn one_detail(d: TransactionDetails) -> (String, Value) {
        let v = serde_json::to_value(map_details(d)).expect("serialize details");
        let obj = v.as_object().expect("details is an object");
        assert_eq!(obj.len(), 1, "exactly one detail variant set: {obj:?}");
        let (k, val) = obj.iter().next().expect("one entry");
        (k.clone(), val.clone())
    }

    fn keys(v: &Value) -> Vec<String> {
        v.as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default()
    }

    #[test]
    fn transfer_endpoint_classifies_and_round_trips() {
        let cases = [
            ("0xabc", TransferEndpoint::Address("0xabc".into())),
            (
                "vaults/v/wallets/w",
                TransferEndpoint::Wallet(WalletRef {
                    vault: VaultId::new("v"),
                    wallet: WalletId::new("w"),
                }),
            ),
            (
                "vaults/v/wallets/w/addresses/a",
                TransferEndpoint::WalletAddress(WalletAddressRef {
                    vault: VaultId::new("v"),
                    wallet: WalletId::new("w"),
                    address: AddressId::new("a"),
                }),
            ),
        ];
        for (wire, expected) in cases {
            assert_eq!(TransferEndpoint::from(wire), expected, "{wire}");
            assert_eq!(expected.to_wire(), wire);
        }
        // From<String> path.
        assert_eq!(
            TransferEndpoint::from("0xabc".to_string()),
            TransferEndpoint::Address("0xabc".into())
        );
    }

    #[test]
    fn sponsor_classifies_gas_station_and_endpoints() {
        let gas = Sponsor::from("vaults/v/gasStations/g");
        assert_eq!(
            gas,
            Sponsor::GasStation(GasStationRef {
                vault: VaultId::new("v"),
                gas_station: GasStationId::new("g"),
            })
        );
        assert_eq!(gas.to_wire(), "vaults/v/gasStations/g");
        // A non-gas-station classifies as an endpoint.
        let wallet = Sponsor::from("vaults/v/wallets/w".to_string());
        assert_eq!(
            wallet,
            Sponsor::Endpoint(TransferEndpoint::Wallet(WalletRef {
                vault: VaultId::new("v"),
                wallet: WalletId::new("w"),
            }))
        );
        assert_eq!(wallet.to_wire(), "vaults/v/wallets/w");
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "flat table of all flat detail kinds; splitting hurts readability"
    )]
    fn flat_kinds_map_to_their_single_field() {
        let cases: Vec<(TransactionDetails, &str)> = vec![
            (
                TransactionDetails::AssetTransfer(AssetTransfer {
                    asset: "assets/x".into(),
                    source: "s".into(),
                    destination: "d".into(),
                    amount: dec!(1.5),
                    memo: Some("m".into()),
                    sponsor: None,
                    pay_fee_from_amount: None,
                    stellar_memo: None,
                    xrpl_destination_tag: None,
                }),
                "assetTransfer",
            ),
            (
                TransactionDetails::BatchAssetTransfer(BatchAssetTransfer {
                    asset: "assets/x".into(),
                    source: "s".into(),
                    destinations: vec![BatchDestination {
                        destination: "d".into(),
                        amount: dec!(2),
                        note: None,
                    }],
                }),
                "assetBatchTransfer",
            ),
            (
                TransactionDetails::Evm(EvmTransaction {
                    from_address: "0xa".into(),
                    network: "ethereum-mainnet".into(),
                    to: Some("0xb".into()),
                    value: Some("1".into()),
                    data: None,
                    publish: Some(true),
                }),
                "evmTransaction",
            ),
            (
                TransactionDetails::EvmPersonalSign(EvmPersonalSign {
                    from_address: "0xa".into(),
                    message: Some("hi".into()),
                    message_hex: None,
                }),
                "evmPersonalSign",
            ),
            (
                TransactionDetails::EvmTypedData(EvmTypedData {
                    from_address: "0xa".into(),
                    message: "{}".into(),
                }),
                "evmSignTypedDataV4",
            ),
            (
                TransactionDetails::EvmAccountDelegation(EvmAccountDelegation {
                    from_address: "0xa".into(),
                    contract_address: "0xc".into(),
                    chain_id: Some("1".into()),
                    nonce: None,
                    offset_nonce: Some(false),
                }),
                "evmAccountDelegation",
            ),
            (
                TransactionDetails::ExchangeWithdrawal(ExchangeWithdrawal {
                    asset: "assets/x".into(),
                    amount: dec!(3),
                    source: "s".into(),
                    destination: "d".into(),
                    destination_network: "n".into(),
                    pay_fee_from_amount: Some(true),
                }),
                "exchangeWithdrawal",
            ),
            (
                TransactionDetails::SolanaRaw(SolanaRaw {
                    network: "solana-mainnet".into(),
                    raw_transaction: "deadbeef".into(),
                    publish: None,
                    replace_blockhash: Some(true),
                    try_replace_blockhash: None,
                }),
                "solanaSerializedTransaction",
            ),
            (
                TransactionDetails::StellarRaw(StellarRaw {
                    network: "stellar-mainnet".into(),
                    source_address: "G...".into(),
                    xdr_envelope: "AAAA".into(),
                    publish: None,
                    use_latest_sequence_number: Some(true),
                }),
                "stellarRawTransaction",
            ),
            (
                TransactionDetails::SuiRaw(SuiRaw {
                    network: "sui-mainnet".into(),
                    sender: "0xs".into(),
                    tx_bcs_bytes: "AAAA".into(),
                    publish: None,
                }),
                "suiRawTransaction",
            ),
            (
                TransactionDetails::TronTriggerContract(TronTriggerSmartContract {
                    network: "tron-mainnet".into(),
                    owner_address: "T...".into(),
                    contract_address: "T...c".into(),
                    data: Some("0x".into()),
                    call_value: None,
                }),
                "tronTriggerSmartContract",
            ),
            (
                TransactionDetails::XrplRaw(XrplRaw {
                    network: "xrpl-mainnet".into(),
                    sender: "r...".into(),
                    json_transaction_data: serde_json::Map::new(),
                    publish: None,
                }),
                "xrplRawTransaction",
            ),
        ];
        for (detail, expected) in cases {
            let (key, _) = one_detail(detail);
            assert_eq!(key, expected);
        }
    }

    #[test]
    fn asset_transfer_carries_sponsor_and_chain_options() {
        let (_key, value) = one_detail(TransactionDetails::AssetTransfer(AssetTransfer {
            asset: "assets/x".into(),
            source: "s".into(),
            destination: "d".into(),
            amount: dec!(1),
            memo: None,
            sponsor: Some("vaults/v/wallets/gas".into()),
            pay_fee_from_amount: Some(true),
            stellar_memo: Some(StellarMemo {
                memo_type: StellarMemoType::Text,
                data: "hi".into(),
            }),
            xrpl_destination_tag: Some("12345".into()),
        }));
        assert_eq!(
            value.get("sponsor").and_then(|v| v.as_str()),
            Some("vaults/v/wallets/gas")
        );
        assert_eq!(
            value
                .get("payFeeFromAmount")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert!(
            value
                .get("stellarOptions")
                .and_then(|o| o.get("memo"))
                .is_some(),
            "stellarOptions.memo present: {value:?}"
        );
        assert_eq!(
            value
                .get("xrplOptions")
                .and_then(|o| o.get("destinationTag"))
                .and_then(|v| v.as_str()),
            Some("12345")
        );
    }

    #[test]
    fn tron_actions_each_set_one_sub_field() {
        let freeze = TronFreeze {
            amount: "1000000".into(),
            owner_address: "T...".into(),
            resource: TronResource::Energy,
        };
        let cases: Vec<(TronAction, &str)> = vec![
            (
                TronAction::FreezeBalanceV2(freeze.clone()),
                "freezeBalanceV2",
            ),
            (
                TronAction::UnfreezeBalanceV2(freeze.clone()),
                "unfreezeBalanceV2",
            ),
            (
                TronAction::DelegateResource(TronDelegate {
                    amount: "1".into(),
                    owner_address: "T...".into(),
                    receiver_address: "T...r".into(),
                    resource: TronResource::Bandwidth,
                    lock: Some(true),
                    lock_period: Some("100".into()),
                }),
                "delegateResource",
            ),
            (
                TronAction::UndelegateResource(TronUndelegate {
                    amount: "1".into(),
                    owner_address: "T...".into(),
                    receiver_address: "T...r".into(),
                    resource: TronResource::Bandwidth,
                }),
                "undelegateResource",
            ),
            (
                TronAction::VoteWitness(vec![TronVote {
                    vote_address: "T...w".into(),
                    vote_count: "5".into(),
                }]),
                "voteWitness",
            ),
            (
                TronAction::WithdrawBalance {
                    owner_address: "T...".into(),
                },
                "withdrawBalance",
            ),
            (
                TronAction::WithdrawExpireUnfreeze {
                    owner_address: "T...".into(),
                },
                "withdrawExpireUnfreeze",
            ),
            (
                TronAction::CancelAllUnfreezeV2 {
                    owner_address: "T...".into(),
                },
                "cancelAllUnfreezeV2",
            ),
            (
                TronAction::TriggerSmartContract(TronContractCall {
                    owner_address: "T...".into(),
                    contract_address: "T...c".into(),
                    data: None,
                    call_value: Some("0".into()),
                }),
                "triggerSmartContract",
            ),
        ];
        for (action, expected) in cases {
            let tron = TronTransaction::builder()
                .network("tron-mainnet")
                .action(action)
                .publish(true)
                .build()
                .expect("valid tron tx");
            let (key, value) = one_detail(TransactionDetails::Tron(tron));
            assert_eq!(key, "tronTransaction");
            assert!(
                keys(&value).contains(&expected.to_string()),
                "tron action {expected} present in {value:?}"
            );
        }
    }

    #[test]
    fn stellar_operation_bodies_and_memo_map() {
        let bodies: Vec<(StellarOpBody, &str)> = vec![
            (
                StellarOpBody::Payment {
                    asset: "native".into(),
                    destination_address: "G...d".into(),
                    raw_amount: "100".into(),
                },
                "payment",
            ),
            (
                StellarOpBody::CreateAccount {
                    destination_address: "G...d".into(),
                    raw_starting_balance: "100".into(),
                },
                "createAccount",
            ),
            (
                StellarOpBody::ChangeTrust {
                    asset: "USDC".into(),
                    limit: Some("1000".into()),
                },
                "changeTrust",
            ),
            (StellarOpBody::Raw("AAAA".into()), "raw"),
        ];
        for (body, expected) in bodies {
            let tx = StellarTransaction::builder()
                .network("stellar-mainnet")
                .source_address("G...s")
                .operation(StellarOperation::new(body))
                .memo(StellarMemo {
                    memo_type: StellarMemoType::Text,
                    data: "hi".into(),
                })
                .fee("100")
                .time_bounds(StellarTimeBounds {
                    max_unix_time: "999".into(),
                    min_unix_time: Some("0".into()),
                })
                .publish(false)
                .build()
                .expect("valid stellar tx");
            let (key, value) = one_detail(TransactionDetails::Stellar(tx));
            assert_eq!(key, "stellarTransaction");
            let op = value
                .get("operations")
                .and_then(|o| o.as_array())
                .and_then(|a| a.first())
                .and_then(|o| o.get("body"))
                .expect("operation body present");
            assert!(
                keys(op).contains(&expected.to_string()),
                "op body {expected} present in {op:?}"
            );
            assert!(value.get("memo").is_some(), "memo present");
        }
    }

    #[test]
    fn stellar_memo_types_all_map() {
        for ty in [
            StellarMemoType::Text,
            StellarMemoType::Id,
            StellarMemoType::Hash,
            StellarMemoType::Return,
        ] {
            let memo: g::Apiv2StellarTransactionMemo = StellarMemo {
                memo_type: ty,
                data: "d".into(),
            }
            .into();
            // Each maps to a non-null memo type; serialization must succeed.
            serde_json::to_value(&memo).expect("serialize memo");
        }
    }

    #[test]
    fn tron_resource_both_variants_map() {
        let b: g::Apiv2TronTransactionTronResourceEnum = TronResource::Bandwidth.into();
        let e: g::Apiv2TronTransactionTronResourceEnum = TronResource::Energy.into();
        assert!(matches!(
            b,
            g::Apiv2TronTransactionTronResourceEnum::Bandwidth
        ));
        assert!(matches!(e, g::Apiv2TronTransactionTronResourceEnum::Energy));
    }

    #[test]
    fn tron_builder_requires_network_and_action() {
        let no_net = TronTransaction::builder()
            .action(TronAction::WithdrawBalance {
                owner_address: "T".into(),
            })
            .build();
        assert!(matches!(no_net, Err(ApiError::Config(_))));
        let no_action = TronTransaction::builder().network("tron-mainnet").build();
        assert!(matches!(no_action, Err(ApiError::Config(_))));
    }

    #[test]
    fn stellar_builder_requires_network_and_source() {
        let no_net = StellarTransaction::builder().source_address("G").build();
        assert!(matches!(no_net, Err(ApiError::Config(_))));
        let no_src = StellarTransaction::builder()
            .network("stellar-mainnet")
            .build();
        assert!(matches!(no_src, Err(ApiError::Config(_))));
    }

    #[test]
    fn estimate_details_maps_supported_and_skips_others() {
        // supported: asset transfer → assetTransfer
        let v = serde_json::to_value(map_estimate_details(TransactionDetails::AssetTransfer(
            AssetTransfer {
                asset: "assets/x".into(),
                source: "s".into(),
                destination: "d".into(),
                amount: dec!(1),
                memo: None,
                sponsor: None,
                pay_fee_from_amount: None,
                stellar_memo: None,
                xrpl_destination_tag: None,
            },
        )))
        .unwrap();
        assert_eq!(keys(&v), vec!["assetTransfer".to_string()]);

        // supported: evm, tron, tron trigger, batch
        for d in [
            TransactionDetails::Evm(EvmTransaction {
                from_address: "0xa".into(),
                network: "ethereum-mainnet".into(),
                to: None,
                value: None,
                data: None,
                publish: None,
            }),
            TransactionDetails::Tron(
                TronTransaction::builder()
                    .network("tron-mainnet")
                    .action(TronAction::WithdrawBalance {
                        owner_address: "T".into(),
                    })
                    .build()
                    .unwrap(),
            ),
            TransactionDetails::TronTriggerContract(TronTriggerSmartContract {
                network: "tron-mainnet".into(),
                owner_address: "T".into(),
                contract_address: "Tc".into(),
                data: None,
                call_value: None,
            }),
            TransactionDetails::BatchAssetTransfer(BatchAssetTransfer {
                asset: "assets/x".into(),
                source: "s".into(),
                destinations: vec![],
            }),
        ] {
            let v = serde_json::to_value(map_estimate_details(d)).unwrap();
            assert_eq!(keys(&v).len(), 1);
        }

        // unsupported: solana raw → empty (server rejects)
        let v = serde_json::to_value(map_estimate_details(TransactionDetails::SolanaRaw(
            SolanaRaw {
                network: "solana-mainnet".into(),
                raw_transaction: "x".into(),
                publish: None,
                replace_blockhash: None,
                try_replace_blockhash: None,
            },
        )))
        .unwrap();
        assert!(keys(&v).is_empty(), "unsupported kind leaves details empty");
    }
}
