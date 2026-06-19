//! The transaction `details` oneof as a typed [`TransactionDetails`] enum, one variant per
//! API message, plus the curated input structs and their mappings onto the generated types.
//!
//! Flat kinds are plain structs with non-`Option` required fields — a struct literal can't
//! omit them. The two deeply-nested kinds ([`TronTransaction`], [`StellarTransaction`]) use a
//! `::builder()…​.build()?` whose `Result` is the single place a missing required field is
//! reported, so the send path never fails for "you forgot a field".

use rust_decimal::Decimal;

use crate::error::{Result, UtilaError};
use crate::generated::types as g;
use crate::ids::{AssetId, NetworkId};

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

// ─────────────────────────── flat kinds ───────────────────────────

/// Transfer an asset from a source wallet to a destination address. `assetTransfer` is the
/// API's one **cross-chain** transfer primitive; the optional fields below are the per-chain
/// riders it carries (they only apply when the asset lives on the relevant chain) — distinct
/// from the per-network *transaction* kinds ([`EvmTransaction`], [`TronTransaction`], …).
#[derive(Debug, Clone)]
pub struct AssetTransfer {
    pub asset: AssetId,
    /// Source wallet resource name, e.g. `vaults/.../wallets/...`.
    pub source: String,
    /// Destination address or resource name.
    pub destination: String,
    /// Amount in DISPLAY units (whole tokens, e.g. `1.5`).
    pub amount: Decimal,
    pub memo: Option<String>,
    /// A sponsor (gas) wallet that pays the network fee — a *native sponsored transfer*
    /// (EVM / Solana / Sui / Aptos). `source` still sends the asset; `sponsor` only pays fees.
    pub sponsor: Option<String>,
    /// EVM-only: deduct the network fee from the transfer `amount` itself.
    pub pay_fee_from_amount: Option<bool>,
    /// Stellar-only: a typed on-chain memo for the transfer (distinct from the plain `memo`).
    pub stellar_memo: Option<StellarMemo>,
    /// XRPL-only: the destination tag (commonly required for exchange deposits).
    pub xrpl_destination_tag: Option<String>,
}

impl From<AssetTransfer> for g::V2AssetTransfer {
    fn from(t: AssetTransfer) -> Self {
        g::V2AssetTransfer {
            amount: t.amount.to_string(),
            asset: t.asset.as_str().to_string(),
            destination: t.destination,
            source: t.source,
            memo: t.memo,
            pay_fee_from_amount: t.pay_fee_from_amount,
            sponsor: t.sponsor,
            stellar_options: t.stellar_memo.map(|memo| g::AssetTransferStellarOptions {
                memo: Some(memo.into()),
            }),
            xrpl_options: t.xrpl_destination_tag.map(|destination_tag| {
                g::AssetTransferXrplOptions {
                    destination_tag: Some(destination_tag),
                }
            }),
        }
    }
}

/// Transfer one asset from a single source to many destinations in one transaction.
#[derive(Debug, Clone)]
pub struct BatchAssetTransfer {
    pub asset: AssetId,
    pub source: String,
    pub destinations: Vec<BatchDestination>,
}

/// One leg of a [`BatchAssetTransfer`].
#[derive(Debug, Clone)]
pub struct BatchDestination {
    pub destination: String,
    /// Amount in DISPLAY units.
    pub amount: Decimal,
    pub note: Option<String>,
}

impl From<BatchDestination> for g::BatchAssetTransferBatchTransferDestination {
    fn from(d: BatchDestination) -> Self {
        g::BatchAssetTransferBatchTransferDestination {
            amount: d.amount.to_string(),
            destination: d.destination,
            note: d.note,
        }
    }
}

impl From<BatchAssetTransfer> for g::V2BatchAssetTransfer {
    fn from(t: BatchAssetTransfer) -> Self {
        g::V2BatchAssetTransfer {
            asset: t.asset.as_str().to_string(),
            source: t.source,
            destinations: t.destinations.into_iter().map(Into::into).collect(),
        }
    }
}

/// A raw EVM transaction. Advanced knobs (`override_params`, EIP-7702
/// `authorization_details`) are not curated — reach for the generated type if you need them.
#[derive(Debug, Clone)]
pub struct EvmTransaction {
    pub from_address: String,
    pub network: NetworkId,
    pub to: Option<String>,
    /// Value in wei (base units), as a decimal string.
    pub value: Option<String>,
    /// Hex-encoded calldata.
    pub data: Option<String>,
    pub publish: Option<bool>,
}

impl From<EvmTransaction> for g::Apiv2EvmTransaction {
    fn from(t: EvmTransaction) -> Self {
        g::Apiv2EvmTransaction {
            authorization_details: None,
            data: t.data,
            from_address: t.from_address,
            network: t.network.as_str().to_string(),
            override_params: None,
            publish: t.publish,
            to_address: t.to,
            value: t.value,
        }
    }
}

/// An EVM `personal_sign` request. Exactly one of `message` / `message_hex` is meaningful.
#[derive(Debug, Clone)]
pub struct EvmPersonalSign {
    pub from_address: String,
    pub message: Option<String>,
    pub message_hex: Option<String>,
}

impl From<EvmPersonalSign> for g::V2EvmPersonalSign {
    fn from(t: EvmPersonalSign) -> Self {
        g::V2EvmPersonalSign {
            from_address: t.from_address,
            message: t.message,
            message_hex: t.message_hex,
        }
    }
}

/// An EVM `eth_signTypedData_v4` request (`message` is the JSON typed-data document).
#[derive(Debug, Clone)]
pub struct EvmTypedData {
    pub from_address: String,
    pub message: String,
}

impl From<EvmTypedData> for g::V2EvmSignTypedDataV4 {
    fn from(t: EvmTypedData) -> Self {
        g::V2EvmSignTypedDataV4 {
            from_address: t.from_address,
            message: t.message,
        }
    }
}

/// An EIP-7702 account-delegation authorization.
#[derive(Debug, Clone)]
pub struct EvmAccountDelegation {
    pub from_address: String,
    pub contract_address: String,
    pub chain_id: Option<String>,
    pub nonce: Option<String>,
    pub offset_nonce: Option<bool>,
}

impl From<EvmAccountDelegation> for g::Apiv2EvmAccountDelegation {
    fn from(t: EvmAccountDelegation) -> Self {
        g::Apiv2EvmAccountDelegation {
            chain_id: t.chain_id,
            contract_address: t.contract_address,
            from_address: t.from_address,
            nonce: t.nonce,
            offset_nonce: t.offset_nonce,
        }
    }
}

/// Withdraw funds from a connected exchange account.
#[derive(Debug, Clone)]
pub struct ExchangeWithdrawal {
    pub asset: AssetId,
    /// Amount in DISPLAY units.
    pub amount: Decimal,
    pub source: String,
    pub destination: String,
    pub destination_network: String,
    pub pay_fee_from_amount: Option<bool>,
}

impl From<ExchangeWithdrawal> for g::Apiv2ExchangeWithdrawal {
    fn from(t: ExchangeWithdrawal) -> Self {
        g::Apiv2ExchangeWithdrawal {
            amount: t.amount.to_string(),
            asset: t.asset.as_str().to_string(),
            destination: t.destination,
            destination_network: t.destination_network,
            pay_fee_from_amount: t.pay_fee_from_amount,
            source: t.source,
        }
    }
}

/// A pre-serialized Solana transaction.
#[derive(Debug, Clone)]
pub struct SolanaRaw {
    pub network: NetworkId,
    /// Base64-encoded serialized transaction.
    pub raw_transaction: String,
    pub publish: Option<bool>,
    pub replace_blockhash: Option<bool>,
    pub try_replace_blockhash: Option<bool>,
}

impl From<SolanaRaw> for g::V2SolanaSerializedTransaction {
    fn from(t: SolanaRaw) -> Self {
        g::V2SolanaSerializedTransaction {
            network: t.network.as_str().to_string(),
            publish: t.publish,
            raw_transaction: t.raw_transaction,
            replace_blockhash: t.replace_blockhash,
            try_replace_blockhash: t.try_replace_blockhash,
        }
    }
}

/// A pre-built Stellar transaction envelope (XDR).
#[derive(Debug, Clone)]
pub struct StellarRaw {
    pub network: NetworkId,
    pub source_address: String,
    /// Base64 XDR transaction envelope.
    pub xdr_envelope: String,
    pub publish: Option<bool>,
    pub use_latest_sequence_number: Option<bool>,
}

impl From<StellarRaw> for g::V2StellarRawTransaction {
    fn from(t: StellarRaw) -> Self {
        g::V2StellarRawTransaction {
            network: t.network.as_str().to_string(),
            publish: t.publish,
            source_address: t.source_address,
            use_latest_sequence_number: t.use_latest_sequence_number,
            xdr_envelope: t.xdr_envelope,
        }
    }
}

/// A pre-serialized Sui transaction (BCS bytes).
#[derive(Debug, Clone)]
pub struct SuiRaw {
    pub network: NetworkId,
    pub sender: String,
    /// Base64-encoded BCS transaction bytes.
    pub tx_bcs_bytes: String,
    pub publish: Option<bool>,
}

impl From<SuiRaw> for g::V2SuiRawTransaction {
    fn from(t: SuiRaw) -> Self {
        g::V2SuiRawTransaction {
            network: t.network.as_str().to_string(),
            publish: t.publish,
            sender: t.sender,
            tx_bcs_bytes: t.tx_bcs_bytes,
        }
    }
}

/// A raw XRPL transaction (JSON transaction object).
#[derive(Debug, Clone)]
pub struct XrplRaw {
    pub network: NetworkId,
    pub sender: String,
    pub json_transaction_data: serde_json::Map<String, serde_json::Value>,
    pub publish: Option<bool>,
}

impl From<XrplRaw> for g::V2XrplRawTransaction {
    fn from(t: XrplRaw) -> Self {
        g::V2XrplRawTransaction {
            json_transaction_data: t.json_transaction_data,
            network: t.network.as_str().to_string(),
            publish: t.publish,
            sender: t.sender,
        }
    }
}

/// Call a Tron smart contract directly (the top-level detail kind, which carries its own
/// `network`). For a contract call *inside* a staking [`TronTransaction`], see
/// [`TronContractCall`].
#[derive(Debug, Clone)]
pub struct TronTriggerSmartContract {
    pub network: NetworkId,
    pub owner_address: String,
    pub contract_address: String,
    pub data: Option<String>,
    pub call_value: Option<String>,
}

impl From<TronTriggerSmartContract> for g::V2TronTriggerSmartContract {
    fn from(t: TronTriggerSmartContract) -> Self {
        g::V2TronTriggerSmartContract {
            call_value: t.call_value,
            contract_address: t.contract_address,
            data: t.data,
            network: t.network.as_str().to_string(),
            owner_address: t.owner_address,
        }
    }
}

// ─────────────────────────── Tron (nested) ───────────────────────────

/// A Tron transaction: a `network` plus exactly one staking/resource [`TronAction`].
/// Build it with [`TronTransaction::builder`].
#[derive(Debug, Clone)]
pub struct TronTransaction {
    pub network: NetworkId,
    pub action: TronAction,
    pub publish: Option<bool>,
}

impl TronTransaction {
    #[must_use]
    pub fn builder() -> TronTransactionBuilder {
        TronTransactionBuilder::default()
    }
}

/// Builder for [`TronTransaction`]; `build()` reports a missing `network`/`action`.
#[derive(Default)]
pub struct TronTransactionBuilder {
    network: Option<NetworkId>,
    action: Option<TronAction>,
    publish: Option<bool>,
}

impl TronTransactionBuilder {
    pub fn network(mut self, network: impl Into<NetworkId>) -> Self {
        self.network = Some(network.into());
        self
    }
    pub fn action(mut self, action: TronAction) -> Self {
        self.action = Some(action);
        self
    }
    pub fn publish(mut self, publish: bool) -> Self {
        self.publish = Some(publish);
        self
    }
    pub fn build(self) -> Result<TronTransaction> {
        Ok(TronTransaction {
            network: self
                .network
                .ok_or_else(|| UtilaError::Config("TronTransaction: network is required".into()))?,
            action: self
                .action
                .ok_or_else(|| UtilaError::Config("TronTransaction: action is required".into()))?,
            publish: self.publish,
        })
    }
}

/// The single resource-management action a [`TronTransaction`] performs.
#[derive(Debug, Clone)]
pub enum TronAction {
    FreezeBalanceV2(TronFreeze),
    UnfreezeBalanceV2(TronFreeze),
    DelegateResource(TronDelegate),
    UndelegateResource(TronUndelegate),
    VoteWitness(Vec<TronVote>),
    WithdrawBalance { owner_address: String },
    WithdrawExpireUnfreeze { owner_address: String },
    CancelAllUnfreezeV2 { owner_address: String },
    TriggerSmartContract(TronContractCall),
}

/// The Tron resource a stake targets.
#[derive(Debug, Clone, Copy)]
pub enum TronResource {
    Bandwidth,
    Energy,
}

impl From<TronResource> for g::Apiv2TronTransactionTronResourceEnum {
    fn from(r: TronResource) -> Self {
        match r {
            TronResource::Bandwidth => Self::Bandwidth,
            TronResource::Energy => Self::Energy,
        }
    }
}

/// Freeze (or unfreeze) TRX for bandwidth/energy.
#[derive(Debug, Clone)]
pub struct TronFreeze {
    /// Amount of TRX, in SUN.
    pub amount: String,
    pub owner_address: String,
    pub resource: TronResource,
}

impl From<TronFreeze> for g::Apiv2TronTransactionFreezeBalanceV2 {
    fn from(t: TronFreeze) -> Self {
        g::Apiv2TronTransactionFreezeBalanceV2 {
            amount: t.amount,
            owner_address: t.owner_address,
            resource: t.resource.into(),
        }
    }
}

impl From<TronFreeze> for g::Apiv2TronTransactionUnfreezeBalanceV2 {
    fn from(t: TronFreeze) -> Self {
        g::Apiv2TronTransactionUnfreezeBalanceV2 {
            amount: t.amount,
            owner_address: t.owner_address,
            resource: t.resource.into(),
        }
    }
}

/// Delegate frozen resource to another address.
#[derive(Debug, Clone)]
pub struct TronDelegate {
    pub amount: String,
    pub owner_address: String,
    pub receiver_address: String,
    pub resource: TronResource,
    pub lock: Option<bool>,
    pub lock_period: Option<String>,
}

impl From<TronDelegate> for g::Apiv2TronTransactionDelegateResource {
    fn from(t: TronDelegate) -> Self {
        g::Apiv2TronTransactionDelegateResource {
            amount: t.amount,
            lock: t.lock,
            lock_period: t.lock_period,
            owner_address: t.owner_address,
            receiver_address: t.receiver_address,
            resource: t.resource.into(),
        }
    }
}

/// Reclaim previously delegated resource.
#[derive(Debug, Clone)]
pub struct TronUndelegate {
    pub amount: String,
    pub owner_address: String,
    pub receiver_address: String,
    pub resource: TronResource,
}

impl From<TronUndelegate> for g::Apiv2TronTransactionUnDelegateResource {
    fn from(t: TronUndelegate) -> Self {
        g::Apiv2TronTransactionUnDelegateResource {
            amount: t.amount,
            owner_address: t.owner_address,
            receiver_address: t.receiver_address,
            resource: t.resource.into(),
        }
    }
}

/// A single super-representative vote.
#[derive(Debug, Clone)]
pub struct TronVote {
    pub vote_address: String,
    pub vote_count: String,
}

impl From<TronVote> for g::Apiv2TronTransactionVoteWitnessVote {
    fn from(t: TronVote) -> Self {
        g::Apiv2TronTransactionVoteWitnessVote {
            vote_address: t.vote_address,
            vote_count: t.vote_count,
        }
    }
}

/// A contract call performed inside a [`TronTransaction`] (no `network` — it is on the parent).
#[derive(Debug, Clone)]
pub struct TronContractCall {
    pub owner_address: String,
    pub contract_address: String,
    pub data: Option<String>,
    pub call_value: Option<String>,
}

impl From<TronContractCall> for g::Apiv2TronTransactionTriggerSmartContract {
    fn from(t: TronContractCall) -> Self {
        g::Apiv2TronTransactionTriggerSmartContract {
            call_value: t.call_value,
            contract_address: t.contract_address,
            data: t.data,
            owner_address: t.owner_address,
        }
    }
}

impl From<TronTransaction> for g::Apiv2TronTransaction {
    fn from(t: TronTransaction) -> Self {
        // `Apiv2TronTransaction` doesn't derive `Default` in the types module, so spell out
        // the all-`None` action fields; exactly one is set from the curated `action` below.
        let mut out = g::Apiv2TronTransaction {
            network: t.network.as_str().to_string(),
            publish: t.publish,
            cancel_all_unfreeze_v2: None,
            delegate_resource: None,
            freeze_balance_v2: None,
            trigger_smart_contract: None,
            undelegate_resource: None,
            unfreeze_balance_v2: None,
            vote_witness: None,
            withdraw_balance: None,
            withdraw_expire_unfreeze: None,
        };
        match t.action {
            TronAction::FreezeBalanceV2(f) => out.freeze_balance_v2 = Some(f.into()),
            TronAction::UnfreezeBalanceV2(f) => out.unfreeze_balance_v2 = Some(f.into()),
            TronAction::DelegateResource(d) => out.delegate_resource = Some(d.into()),
            TronAction::UndelegateResource(d) => out.undelegate_resource = Some(d.into()),
            TronAction::VoteWitness(votes) => {
                out.vote_witness = Some(g::Apiv2TronTransactionVoteWitness {
                    owner_address: votes
                        .first()
                        .map(|v| v.vote_address.clone())
                        .unwrap_or_default(),
                    votes: votes.into_iter().map(Into::into).collect(),
                });
            }
            TronAction::WithdrawBalance { owner_address } => {
                out.withdraw_balance =
                    Some(g::Apiv2TronTransactionWithdrawBalance { owner_address });
            }
            TronAction::WithdrawExpireUnfreeze { owner_address } => {
                out.withdraw_expire_unfreeze =
                    Some(g::Apiv2TronTransactionWithdrawExpireUnfreeze { owner_address });
            }
            TronAction::CancelAllUnfreezeV2 { owner_address } => {
                out.cancel_all_unfreeze_v2 =
                    Some(g::Apiv2TronTransactionCancelAllUnfreezeV2 { owner_address });
            }
            TronAction::TriggerSmartContract(c) => out.trigger_smart_contract = Some(c.into()),
        }
        out
    }
}

// ─────────────────────────── Stellar (nested) ───────────────────────────

/// A structured Stellar transaction. Build it with [`StellarTransaction::builder`]; exotic
/// operations the API doesn't model here go through [`StellarOpBody::Raw`].
#[derive(Debug, Clone)]
pub struct StellarTransaction {
    pub network: NetworkId,
    pub source_address: String,
    pub operations: Vec<StellarOperation>,
    pub memo: Option<StellarMemo>,
    pub fee: Option<String>,
    pub time_bounds: Option<StellarTimeBounds>,
    pub publish: Option<bool>,
}

impl StellarTransaction {
    #[must_use]
    pub fn builder() -> StellarTransactionBuilder {
        StellarTransactionBuilder::default()
    }
}

/// Builder for [`StellarTransaction`]; `build()` reports a missing `network`/`source_address`.
#[derive(Default)]
pub struct StellarTransactionBuilder {
    network: Option<NetworkId>,
    source_address: Option<String>,
    operations: Vec<StellarOperation>,
    memo: Option<StellarMemo>,
    fee: Option<String>,
    time_bounds: Option<StellarTimeBounds>,
    publish: Option<bool>,
}

impl StellarTransactionBuilder {
    pub fn network(mut self, network: impl Into<NetworkId>) -> Self {
        self.network = Some(network.into());
        self
    }
    pub fn source_address(mut self, address: impl Into<String>) -> Self {
        self.source_address = Some(address.into());
        self
    }
    /// Append one operation (call repeatedly to build the list).
    pub fn operation(mut self, op: StellarOperation) -> Self {
        self.operations.push(op);
        self
    }
    pub fn memo(mut self, memo: StellarMemo) -> Self {
        self.memo = Some(memo);
        self
    }
    pub fn fee(mut self, fee: impl Into<String>) -> Self {
        self.fee = Some(fee.into());
        self
    }
    pub fn time_bounds(mut self, bounds: StellarTimeBounds) -> Self {
        self.time_bounds = Some(bounds);
        self
    }
    pub fn publish(mut self, publish: bool) -> Self {
        self.publish = Some(publish);
        self
    }
    pub fn build(self) -> Result<StellarTransaction> {
        Ok(StellarTransaction {
            network: self.network.ok_or_else(|| {
                UtilaError::Config("StellarTransaction: network is required".into())
            })?,
            source_address: self.source_address.ok_or_else(|| {
                UtilaError::Config("StellarTransaction: source_address is required".into())
            })?,
            operations: self.operations,
            memo: self.memo,
            fee: self.fee,
            time_bounds: self.time_bounds,
            publish: self.publish,
        })
    }
}

/// One Stellar operation: a body plus an optional per-operation source account.
#[derive(Debug, Clone)]
pub struct StellarOperation {
    pub body: StellarOpBody,
    pub source_account: Option<String>,
}

impl StellarOperation {
    /// A body with no per-operation source account override.
    #[must_use]
    pub fn new(body: StellarOpBody) -> Self {
        Self {
            body,
            source_account: None,
        }
    }
}

/// A Stellar operation body. Common operations are curated; anything else (SEP-41, etc.)
/// uses [`StellarOpBody::Raw`] (the API's built-in raw-operation escape).
#[derive(Debug, Clone)]
pub enum StellarOpBody {
    Payment {
        asset: String,
        destination_address: String,
        /// Amount in stroops (raw units).
        raw_amount: String,
    },
    CreateAccount {
        destination_address: String,
        raw_starting_balance: String,
    },
    ChangeTrust {
        asset: String,
        limit: Option<String>,
    },
    /// A raw, base64-encoded operation XDR for ops not modeled above.
    Raw(String),
}

impl From<StellarOpBody> for g::OperationBody {
    fn from(b: StellarOpBody) -> Self {
        let mut out = g::OperationBody {
            change_trust: None,
            create_account: None,
            payment: None,
            raw: None,
            sep41_payment: None,
            sep41_token_approval: None,
        };
        match b {
            StellarOpBody::Payment {
                asset,
                destination_address,
                raw_amount,
            } => {
                out.payment = Some(g::OperationBodyPayment {
                    asset,
                    destination_address,
                    raw_amount,
                });
            }
            StellarOpBody::CreateAccount {
                destination_address,
                raw_starting_balance,
            } => {
                out.create_account = Some(g::OperationBodyCreateAccount {
                    destination_address,
                    raw_starting_balance,
                });
            }
            StellarOpBody::ChangeTrust { asset, limit } => {
                out.change_trust = Some(g::OperationBodyChangeTrust { asset, limit });
            }
            StellarOpBody::Raw(xdr) => out.raw = Some(xdr),
        }
        out
    }
}

impl From<StellarOperation> for g::V2StellarTransactionOperation {
    fn from(op: StellarOperation) -> Self {
        g::V2StellarTransactionOperation {
            body: op.body.into(),
            source_account_address: op.source_account,
        }
    }
}

/// A Stellar memo.
#[derive(Debug, Clone)]
pub struct StellarMemo {
    pub memo_type: StellarMemoType,
    pub data: String,
}

/// The Stellar memo type.
#[derive(Debug, Clone, Copy)]
pub enum StellarMemoType {
    Text,
    Id,
    Hash,
    Return,
}

impl From<StellarMemoType> for g::Apiv2StellarTransactionMemoTypeEnum {
    fn from(t: StellarMemoType) -> Self {
        match t {
            StellarMemoType::Text => Self::Text,
            StellarMemoType::Id => Self::Id,
            StellarMemoType::Hash => Self::Hash,
            StellarMemoType::Return => Self::Return,
        }
    }
}

impl From<StellarMemo> for g::Apiv2StellarTransactionMemo {
    fn from(m: StellarMemo) -> Self {
        g::Apiv2StellarTransactionMemo {
            data: m.data,
            type_: m.memo_type.into(),
        }
    }
}

/// Stellar transaction validity window (Unix seconds).
#[derive(Debug, Clone)]
pub struct StellarTimeBounds {
    pub max_unix_time: String,
    pub min_unix_time: Option<String>,
}

impl From<StellarTimeBounds> for g::Apiv2StellarTimeBounds {
    fn from(t: StellarTimeBounds) -> Self {
        g::Apiv2StellarTimeBounds {
            max_unix_time: t.max_unix_time,
            min_unix_time: t.min_unix_time,
        }
    }
}

impl From<StellarTransaction> for g::Apiv2StellarTransaction {
    fn from(t: StellarTransaction) -> Self {
        g::Apiv2StellarTransaction {
            fee: t.fee,
            memo: t.memo.map(Into::into),
            network: t.network.as_str().to_string(),
            operations: t.operations.into_iter().map(Into::into).collect(),
            publish: t.publish,
            source_address: t.source_address,
            time_bounds: t.time_bounds.map(Into::into),
        }
    }
}

// ─────────────────────────── oneof envelopes ───────────────────────────

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
        assert!(matches!(no_net, Err(UtilaError::Config(_))));
        let no_action = TronTransaction::builder().network("tron-mainnet").build();
        assert!(matches!(no_action, Err(UtilaError::Config(_))));
    }

    #[test]
    fn stellar_builder_requires_network_and_source() {
        let no_net = StellarTransaction::builder().source_address("G").build();
        assert!(matches!(no_net, Err(UtilaError::Config(_))));
        let no_src = StellarTransaction::builder()
            .network("stellar-mainnet")
            .build();
        assert!(matches!(no_src, Err(UtilaError::Config(_))));
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
