//! The `transactions()` group surface: the transaction model, the `Transactions` accessor, and
//! its read/action builders (get, list/stream, `batch_get`, cancel, publish, replace, vote,
//! latest simulation, AML screening, fee estimation). The `details` oneof and its per-kind input
//! structs live in the sibling modules.

use chrono::{DateTime, Utc};
use futures::stream::{self, Stream, TryStreamExt};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::client::{enum_label, UtilaClient};
use crate::error::{ApiError, Result};
use crate::generated::types::{
    TransactionTransfer, TransactionsEstimateTransactionFeeBody,
    TransactionsInitiateTransactionBody, TransactionsPublishTransactionBody,
    TransactionsReplaceTransactionBody, TransactionsVoteOnTransactionRequestBody,
    V2AddressBalanceChanges, V2BalanceChange, V2EstimateTransactionFeeResponse, V2EvmFeeEstimation,
    V2ListTransactionsResponse, V2Transaction, V2TransactionAmlScreening,
    V2TransactionPriorityEnum, V2TransactionReplacementTypeEnum, V2TransactionRequest,
    V2TransactionSimulation, V2TransactionTypeEnum, V2TronFeeEstimation,
    V2VoteOnTransactionRequestRequestVote,
};
use crate::generated::ClientTransactionsExt;
use crate::resource::{
    AssetId, NetworkId, ParseRef, ResourceName, SimulationRef, TransactionId, TransactionRef,
    TransactionRequestRef, UserRef, VaultId, WalletRef,
};
use crate::webhook_event::TransactionState;

use super::details::{map_details, map_estimate_details, TransactionDetails};
use super::{
    AssetTransfer, BatchAssetTransfer, EvmAccountDelegation, EvmPersonalSign, EvmTransaction,
    EvmTypedData, ExchangeWithdrawal, SolanaRaw, StellarRaw, StellarTransaction, SuiRaw,
    TronTransaction, TronTriggerSmartContract, XrplRaw,
};

/// Fee priority.
#[derive(Debug, Clone, Copy)]
pub enum Priority {
    Low,
    Normal,
    High,
}

impl From<Priority> for V2TransactionPriorityEnum {
    fn from(p: Priority) -> Self {
        match p {
            Priority::Low => Self::Low,
            Priority::Normal => Self::Normal,
            Priority::High => Self::High,
        }
    }
}

/// A vote on a pending transaction request.
#[derive(Debug, Clone, Copy)]
pub enum Vote {
    Approve,
    Deny,
}

impl From<Vote> for V2VoteOnTransactionRequestRequestVote {
    fn from(v: Vote) -> Self {
        match v {
            Vote::Approve => Self::Approve,
            Vote::Deny => Self::Deny,
        }
    }
}

/// How a signed transaction should be replaced: cancel it (zero-value self-send) or
/// accelerate it (resubmit with a higher fee).
#[derive(Debug, Clone, Copy)]
pub enum ReplacementType {
    Cancel,
    Accelerate,
}

impl From<ReplacementType> for V2TransactionReplacementTypeEnum {
    fn from(r: ReplacementType) -> Self {
        match r {
            ReplacementType::Cancel => Self::Cancel,
            ReplacementType::Accelerate => Self::Accelerate,
        }
    }
}

/// A transaction as returned by the API (facade subset).
///
/// The common/core fields are curated; the many chain-specific detail unions on
/// `V2Transaction` (`evm_transaction`, `solana_transaction`, `btc_psbt`, …) are intentionally
/// omitted — reach for the generated types directly if you need a specific chain's low-level
/// payload.
#[derive(Debug, Clone)]
pub struct Transaction {
    /// The transaction's resource name (`vaults/{v}/transactions/{t}`), parsed to a
    /// [`TransactionRef`] when it matches that shape.
    pub name: ResourceName<TransactionRef>,
    pub network: Option<NetworkId>,
    pub state: Option<TransactionState>,
    /// The transaction `type`.
    pub kind: Option<TransactionKind>,
    pub sub_type: Option<String>,
    pub direction: Option<String>,
    pub hash: Option<String>,
    /// The block the transaction was mined in (the API sends it as a string; parsed to a number).
    pub block_number: Option<u64>,
    pub note: Option<String>,
    pub spam: bool,
    pub create_time: Option<DateTime<Utc>>,
    pub confirm_time: Option<DateTime<Utc>>,
    pub mine_time: Option<DateTime<Utc>>,
    pub expire_time: Option<DateTime<Utc>>,
    /// The users designated to sign this transaction (`users/{id}` or `users/{email}`).
    pub designated_signers: Vec<ResourceName<UserRef>>,
    pub transfers: Vec<Transfer>,
    pub request: Option<TransactionRequest>,
    /// If this transaction was dropped, the transaction that replaced it.
    pub replacement_transaction: Option<TransactionRef>,
}

impl From<V2Transaction> for Transaction {
    fn from(t: V2Transaction) -> Self {
        Self {
            name: ResourceName::parse(t.name.unwrap_or_default()),
            network: t.network.filter(|s| !s.is_empty()).map(NetworkId::from),
            state: t.state.map(TransactionState::from),
            kind: t.type_.map(TransactionKind::from),
            sub_type: t.sub_type.as_ref().and_then(enum_label),
            direction: t.direction.as_ref().and_then(enum_label),
            hash: t.hash.filter(|s| !s.is_empty()),
            block_number: t.block_number.and_then(|s| s.parse().ok()),
            note: t.note.filter(|s| !s.is_empty()),
            spam: t.spam.unwrap_or(false),
            create_time: t.create_time,
            confirm_time: t.confirm_time,
            mine_time: t.mine_time,
            expire_time: t.expire_time,
            designated_signers: t
                .designated_signers
                .into_iter()
                .map(ResourceName::parse)
                .collect(),
            transfers: t.transfers.into_iter().map(Transfer::from).collect(),
            request: t.request.map(TransactionRequest::from),
            replacement_transaction: t
                .replacement_transaction
                .as_deref()
                .filter(|s| !s.is_empty())
                .and_then(TransactionRef::parse),
        }
    }
}

/// What a transaction request represents.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TransactionKind {
    /// An on-chain transaction.
    Transaction,
    /// A message to sign (e.g. `personal_sign` / typed data).
    Message,
}

impl From<V2TransactionTypeEnum> for TransactionKind {
    fn from(t: V2TransactionTypeEnum) -> Self {
        match t {
            V2TransactionTypeEnum::Transaction => Self::Transaction,
            V2TransactionTypeEnum::Message => Self::Message,
        }
    }
}

/// A single value movement within a transaction.
#[derive(Debug, Clone)]
pub struct Transfer {
    pub asset: Option<AssetId>,
    /// The amount moved, in DISPLAY units (whole tokens).
    pub amount: Option<Decimal>,
    /// The source's plain on-chain address (e.g. `0x…`, `G…`), not a Utila resource name.
    pub source_address: Option<String>,
    /// The destination's plain on-chain address.
    pub destination_address: Option<String>,
    pub note: Option<String>,
}

impl From<TransactionTransfer> for Transfer {
    fn from(t: TransactionTransfer) -> Self {
        Self {
            asset: t.asset.filter(|s| !s.is_empty()).map(AssetId::from),
            amount: t.amount.and_then(|s| s.parse::<Decimal>().ok()),
            source_address: t
                .source_address
                .and_then(|a| a.value)
                .filter(|s| !s.is_empty()),
            destination_address: t
                .destination_address
                .and_then(|a| a.value)
                .filter(|s| !s.is_empty()),
            note: t.note.filter(|s| !s.is_empty()),
        }
    }
}

/// The request envelope of a Utila-initiated transaction: who initiated it, the
/// client-supplied external id, and the lifecycle timestamps.
#[derive(Debug, Clone)]
pub struct TransactionRequest {
    pub name: ResourceName<TransactionRequestRef>,
    pub external_id: Option<String>,
    /// The user who initiated the request (`users/{user}`).
    pub initiator: Option<ResourceName<UserRef>>,
    /// The wallet the request was initiated through, if any (`vaults/{v}/wallets/{w}`).
    pub source_wallet: Option<ResourceName<WalletRef>>,
    pub origin: Option<String>,
    pub approve_time: Option<DateTime<Utc>>,
    pub sign_time: Option<DateTime<Utc>>,
    pub publish_time: Option<DateTime<Utc>>,
    pub cancel_time: Option<DateTime<Utc>>,
    pub expire_time: Option<DateTime<Utc>>,
}

impl From<V2TransactionRequest> for TransactionRequest {
    fn from(r: V2TransactionRequest) -> Self {
        Self {
            name: ResourceName::parse(r.name.unwrap_or_default()),
            external_id: r.external_id.filter(|s| !s.is_empty()),
            initiator: r
                .initiator
                .filter(|s| !s.is_empty())
                .map(ResourceName::parse),
            source_wallet: r
                .source_wallet
                .filter(|s| !s.is_empty())
                .map(ResourceName::parse),
            origin: r.origin.as_ref().and_then(enum_label),
            approve_time: r.approve_time,
            sign_time: r.sign_time,
            publish_time: r.publish_time,
            cancel_time: r.cancel_time,
            expire_time: r.expire_time,
        }
    }
}

/// The result of AML screening for a transaction (when screening is configured).
#[derive(Debug, Clone)]
pub struct AmlScreening {
    pub provider: Option<String>,
    pub raw_responses: Vec<String>,
}

impl From<V2TransactionAmlScreening> for AmlScreening {
    fn from(a: V2TransactionAmlScreening) -> Self {
        Self {
            provider: a.provider.as_ref().and_then(enum_label),
            raw_responses: a.raw_responses,
        }
    }
}

/// A dry-run of a transaction: the projected per-address balance changes plus any error.
#[derive(Debug, Clone)]
pub struct Simulation {
    pub name: ResourceName<SimulationRef>,
    pub simulation_time: Option<DateTime<Utc>>,
    /// The simulation error message, if the simulation failed.
    pub error: Option<String>,
    pub balance_changes: Vec<AddressBalanceChange>,
}

impl From<V2TransactionSimulation> for Simulation {
    fn from(s: V2TransactionSimulation) -> Self {
        Self {
            name: ResourceName::parse(s.name.unwrap_or_default()),
            simulation_time: s.simulation_time,
            error: s.error.and_then(|e| e.message).filter(|m| !m.is_empty()),
            balance_changes: s
                .address_balance_changes
                .into_iter()
                .map(AddressBalanceChange::from)
                .collect(),
        }
    }
}

/// The projected balance changes at one address from a simulated transaction.
#[derive(Debug, Clone)]
pub struct AddressBalanceChange {
    pub address: Option<String>,
    pub changes: Vec<BalanceChange>,
}

impl From<V2AddressBalanceChanges> for AddressBalanceChange {
    fn from(c: V2AddressBalanceChanges) -> Self {
        Self {
            address: c.address.and_then(|a| a.value).filter(|s| !s.is_empty()),
            changes: c
                .balance_changes
                .into_iter()
                .map(BalanceChange::from)
                .collect(),
        }
    }
}

/// A single asset's projected change at an address.
#[derive(Debug, Clone)]
pub struct BalanceChange {
    pub asset: Option<AssetId>,
    pub amount: Option<String>,
    pub negative: bool,
}

impl From<V2BalanceChange> for BalanceChange {
    fn from(b: V2BalanceChange) -> Self {
        Self {
            asset: b.asset.filter(|s| !s.is_empty()).map(AssetId::from),
            amount: b.amount.filter(|s| !s.is_empty()),
            negative: b.negative.unwrap_or(false),
        }
    }
}

/// An estimate of a transaction's fee. `total_fee` is in the network's native units; the
/// chain-specific breakdowns are populated for the relevant family.
#[derive(Debug, Clone)]
pub struct FeeEstimate {
    pub total_fee: Option<String>,
    /// True when the transfer is applied as a gasless transfer (`total_fee` is then `"0"`).
    pub gasless: bool,
    /// The fee converted to fiat, as a decimal. Always USD: the Utila API's only valuation
    /// type is `v2ConvertedValue`, whose `amount` is documented "The amount in USD" and whose
    /// `currencyCode` is documented "Always USD", so the currency is fixed by the upstream
    /// contract and not carried here. (The asset-denomination currency that *can* be non-USD —
    /// e.g. a EUR-denominated TIP-20 token — lives on a different field, `v2AssetTokenInfo`'s
    /// `tip20Currency`, and never flows into a converted value.) `None` when the API
    /// omits/blanks the value or sends a figure that doesn't parse as a decimal.
    pub converted_total_fee: Option<Decimal>,
    pub evm_fee: Option<EvmFee>,
    pub tron_fee: Option<TronFee>,
}

impl From<V2EstimateTransactionFeeResponse> for FeeEstimate {
    fn from(r: V2EstimateTransactionFeeResponse) -> Self {
        Self {
            total_fee: r.total_fee.filter(|s| !s.is_empty()),
            gasless: r.gasless.unwrap_or(false),
            converted_total_fee: r
                .converted_total_fee
                .and_then(|v| v.amount)
                .filter(|s| !s.is_empty())
                .and_then(|s| s.parse::<Decimal>().ok()),
            evm_fee: r.evm_fee.map(EvmFee::from),
            tron_fee: r.tron_fee.map(TronFee::from),
        }
    }
}

/// EVM fee breakdown: gas price (wei) and estimated gas used.
#[derive(Debug, Clone)]
pub struct EvmFee {
    pub gas_price: Option<String>,
    pub gas_used: Option<String>,
}

impl From<V2EvmFeeEstimation> for EvmFee {
    fn from(e: V2EvmFeeEstimation) -> Self {
        Self {
            gas_price: e.gas_price.filter(|s| !s.is_empty()),
            gas_used: e.gas_used.filter(|s| !s.is_empty()),
        }
    }
}

/// Tron fee breakdown (core fields). The per-resource `bandwidth`/`energy` fee objects and
/// the account-activation fee are omitted — only the consumed-resource totals are curated.
#[derive(Debug, Clone)]
pub struct TronFee {
    pub bandwidth_used: Option<String>,
    pub energy_used: Option<String>,
    pub total_sun_used: Option<String>,
}

impl From<V2TronFeeEstimation> for TronFee {
    fn from(t: V2TronFeeEstimation) -> Self {
        Self {
            bandwidth_used: t.bandwidth_used.filter(|s| !s.is_empty()),
            energy_used: t.energy_used.filter(|s| !s.is_empty()),
            total_sun_used: t.total_sun_used.filter(|s| !s.is_empty()),
        }
    }
}

/// The outcome of `initiate`, carrying the (possibly auto-generated) idempotency key.
#[derive(Debug, Clone)]
pub struct Initiated {
    pub request_id: String,
    pub transaction: Option<Transaction>,
}

/// One page of `transactions().list()`.
#[derive(Debug, Clone)]
pub struct TransactionPage {
    pub transactions: Vec<Transaction>,
    pub next_page_token: Option<String>,
    pub total_size: i32,
}

impl From<V2ListTransactionsResponse> for TransactionPage {
    fn from(r: V2ListTransactionsResponse) -> Self {
        Self {
            transactions: r.transactions.into_iter().map(Transaction::from).collect(),
            next_page_token: r.next_page_token.filter(|t| !t.is_empty()),
            total_size: r.total_size.unwrap_or(0),
        }
    }
}

pub struct Transactions<'a> {
    pub(crate) client: &'a UtilaClient,
}

impl<'a> Transactions<'a> {
    /// Internal entry point: every per-kind `send` method funnels through here, returning the
    /// modifier builder whose terminal `.send().await` issues the request.
    fn initiate(&self, vault: VaultId, details: TransactionDetails) -> InitiateBuilder<'a> {
        InitiateBuilder {
            client: self.client,
            vault,
            details,
            request_id: None,
            priority: None,
            note: None,
            external_id: None,
            validate_only: false,
            run_simulation: false,
        }
    }

    /// Transfer an asset from a wallet to a destination.
    pub fn asset_transfer(&self, vault: VaultId, transfer: AssetTransfer) -> InitiateBuilder<'a> {
        self.initiate(vault, TransactionDetails::AssetTransfer(transfer))
    }
    /// Transfer one asset to many destinations in a single transaction.
    pub fn batch_asset_transfer(
        &self,
        vault: VaultId,
        transfer: BatchAssetTransfer,
    ) -> InitiateBuilder<'a> {
        self.initiate(vault, TransactionDetails::BatchAssetTransfer(transfer))
    }
    /// Submit a raw EVM transaction.
    pub fn evm(&self, vault: VaultId, tx: EvmTransaction) -> InitiateBuilder<'a> {
        self.initiate(vault, TransactionDetails::Evm(tx))
    }
    /// Sign a message with EVM `personal_sign`.
    pub fn evm_personal_sign(&self, vault: VaultId, sign: EvmPersonalSign) -> InitiateBuilder<'a> {
        self.initiate(vault, TransactionDetails::EvmPersonalSign(sign))
    }
    /// Sign EVM typed data (`eth_signTypedData_v4`).
    pub fn evm_typed_data(&self, vault: VaultId, data: EvmTypedData) -> InitiateBuilder<'a> {
        self.initiate(vault, TransactionDetails::EvmTypedData(data))
    }
    /// Authorize an EIP-7702 account delegation.
    pub fn evm_account_delegation(
        &self,
        vault: VaultId,
        delegation: EvmAccountDelegation,
    ) -> InitiateBuilder<'a> {
        self.initiate(vault, TransactionDetails::EvmAccountDelegation(delegation))
    }
    /// Withdraw from a connected exchange account.
    pub fn exchange_withdrawal(
        &self,
        vault: VaultId,
        withdrawal: ExchangeWithdrawal,
    ) -> InitiateBuilder<'a> {
        self.initiate(vault, TransactionDetails::ExchangeWithdrawal(withdrawal))
    }
    /// Submit a pre-serialized Solana transaction.
    pub fn solana_raw(&self, vault: VaultId, tx: SolanaRaw) -> InitiateBuilder<'a> {
        self.initiate(vault, TransactionDetails::SolanaRaw(tx))
    }
    /// Submit a structured Stellar transaction (build it via [`StellarTransaction::builder`]).
    pub fn stellar(&self, vault: VaultId, tx: StellarTransaction) -> InitiateBuilder<'a> {
        self.initiate(vault, TransactionDetails::Stellar(tx))
    }
    /// Submit a pre-built Stellar transaction envelope (XDR).
    pub fn stellar_raw(&self, vault: VaultId, tx: StellarRaw) -> InitiateBuilder<'a> {
        self.initiate(vault, TransactionDetails::StellarRaw(tx))
    }
    /// Submit a pre-serialized Sui transaction (BCS bytes).
    pub fn sui_raw(&self, vault: VaultId, tx: SuiRaw) -> InitiateBuilder<'a> {
        self.initiate(vault, TransactionDetails::SuiRaw(tx))
    }
    /// Submit a Tron staking/resource transaction (build it via [`TronTransaction::builder`]).
    pub fn tron(&self, vault: VaultId, tx: TronTransaction) -> InitiateBuilder<'a> {
        self.initiate(vault, TransactionDetails::Tron(tx))
    }
    /// Call a Tron smart contract directly.
    pub fn tron_trigger_contract(
        &self,
        vault: VaultId,
        call: TronTriggerSmartContract,
    ) -> InitiateBuilder<'a> {
        self.initiate(vault, TransactionDetails::TronTriggerContract(call))
    }
    /// Submit a raw XRPL transaction (JSON transaction object).
    pub fn xrpl_raw(&self, vault: VaultId, tx: XrplRaw) -> InitiateBuilder<'a> {
        self.initiate(vault, TransactionDetails::XrplRaw(tx))
    }

    /// List a vault's transactions (single page). Returns a builder for the optional
    /// `filter` / `order_by` / pagination; call `.send()` or `.stream()`.
    pub fn list(&self, vault: VaultId) -> ListTransactionsBuilder<'a> {
        ListTransactionsBuilder {
            client: self.client,
            vault,
            filter: None,
            order_by: None,
            page_size: None,
            page_token: None,
        }
    }

    /// Get one transaction by id.
    pub async fn get(&self, vault: VaultId, transaction: TransactionId) -> Result<Transaction> {
        let resp = self
            .client
            .call(|api| {
                api.transactions_get_transaction()
                    .vault_id(vault.as_str())
                    .transaction_id(transaction.as_str())
                    .send()
            })
            .await?;
        resp.transaction
            .map(Transaction::from)
            .ok_or_else(|| missing("transaction"))
    }

    /// Get several transactions by full resource name
    /// (`vaults/{vault_id}/transactions/{transaction_id}`).
    pub async fn batch_get(&self, vault: VaultId, names: Vec<String>) -> Result<Vec<Transaction>> {
        let resp = self
            .client
            .call(|api| {
                api.transactions_batch_get_transactions()
                    .vault_id(vault.as_str())
                    .names(names)
                    .send()
            })
            .await?;
        Ok(resp
            .transactions
            .into_iter()
            .map(Transaction::from)
            .collect())
    }

    /// Cancel a transaction before it is signed (initiator or admin only). For an
    /// already-signed transaction use [`Self::replace`] instead.
    pub async fn cancel(&self, vault: VaultId, transaction: TransactionId) -> Result<()> {
        self.client
            .call(|api| {
                api.transactions_cancel_transaction()
                    .vault_id(vault.as_str())
                    .transaction_id(transaction.as_str())
                    .send()
            })
            .await?;
        Ok(())
    }

    /// Manually publish a fully-signed transaction to the blockchain (EVM only, currently).
    pub async fn publish(&self, vault: VaultId, transaction: TransactionId) -> Result<Transaction> {
        let resp = self
            .client
            .call(|api| {
                api.transactions_publish_transaction()
                    .vault_id(vault.as_str())
                    .transaction_id(transaction.as_str())
                    .body(TransactionsPublishTransactionBody(serde_json::Map::new()))
                    .send()
            })
            .await?;
        resp.transaction
            .map(Transaction::from)
            .ok_or_else(|| missing("transaction"))
    }

    /// Replace a signed-but-unmined transaction (cancel or accelerate). Returns a builder
    /// for the optional note / designated signers; call `.send().await`.
    pub fn replace(
        &self,
        vault: VaultId,
        transaction: TransactionId,
        replacement: ReplacementType,
    ) -> ReplaceBuilder<'a> {
        ReplaceBuilder {
            client: self.client,
            vault,
            transaction,
            replacement,
            note: None,
            designated_signers: Vec::new(),
        }
    }

    /// Vote on a pending transaction request. Returns the resulting transaction state, if
    /// the server reported one.
    pub async fn vote(
        &self,
        vault: VaultId,
        request_id: impl Into<String>,
        vote: Vote,
    ) -> Result<Option<String>> {
        let request_id = request_id.into();
        let body = TransactionsVoteOnTransactionRequestBody { vote: vote.into() };
        let resp = self
            .client
            .call(|api| {
                api.transactions_vote_on_transaction_request()
                    .vault_id(vault.as_str())
                    .transaction_request_id(request_id)
                    .body(body)
                    .send()
            })
            .await?;
        Ok(resp.transaction_state.as_ref().and_then(enum_label))
    }

    /// The latest simulation for a transaction.
    pub async fn latest_simulation(
        &self,
        vault: VaultId,
        transaction: TransactionId,
    ) -> Result<Simulation> {
        let resp = self
            .client
            .call(|api| {
                api.transactions_get_latest_transaction_simulation()
                    .vault_id(vault.as_str())
                    .transaction_id(transaction.as_str())
                    .send()
            })
            .await?;
        resp.transaction_simulation
            .map(Simulation::from)
            .ok_or_else(|| missing("transaction_simulation"))
    }

    /// The AML screening result for a transaction (where screening is configured).
    pub async fn aml_screening(
        &self,
        vault: VaultId,
        transaction: TransactionId,
    ) -> Result<AmlScreening> {
        let resp = self
            .client
            .call(|api| {
                api.transactions_get_transaction_aml_screening()
                    .vault_id(vault.as_str())
                    .transaction_id(transaction.as_str())
                    .send()
            })
            .await?;
        resp.aml_screening
            .map(AmlScreening::from)
            .ok_or_else(|| missing("aml_screening"))
    }

    /// Estimate the fee for a prospective transaction. Returns a builder for the optional
    /// priority; call `.send().await`.
    pub fn estimate_fee(
        &self,
        vault: VaultId,
        details: TransactionDetails,
    ) -> EstimateFeeBuilder<'a> {
        EstimateFeeBuilder {
            client: self.client,
            vault,
            details,
            priority: None,
        }
    }
}

pub struct InitiateBuilder<'a> {
    client: &'a UtilaClient,
    vault: VaultId,
    details: TransactionDetails,
    request_id: Option<String>,
    priority: Option<Priority>,
    note: Option<String>,
    external_id: Option<String>,
    validate_only: bool,
    run_simulation: bool,
}

impl InitiateBuilder<'_> {
    /// Override the idempotency key (default: a fresh UUID). Reuse it to retry safely.
    pub fn request_id(mut self, id: impl Into<String>) -> Self {
        self.request_id = Some(id.into());
        self
    }
    pub fn priority(mut self, p: Priority) -> Self {
        self.priority = Some(p);
        self
    }
    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
    pub fn external_id(mut self, id: impl Into<String>) -> Self {
        self.external_id = Some(id.into());
        self
    }
    pub fn validate_only(mut self, v: bool) -> Self {
        self.validate_only = v;
        self
    }
    pub fn run_simulation(mut self, v: bool) -> Self {
        self.run_simulation = v;
        self
    }

    pub async fn send(self) -> Result<Initiated> {
        let request_id = self
            .request_id
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let body = TransactionsInitiateTransactionBody {
            details: map_details(self.details),
            priority: self.priority.map(Into::into),
            note: self.note,
            external_id: self.external_id,
            request_id: Some(request_id.clone()),
            // Only send the boolean modifiers when set (matches the server's omit-default
            // convention and keeps the request body minimal).
            validate_only: self.validate_only.then_some(true),
            run_simulation: self.run_simulation.then_some(true),
            designated_signers: Vec::new(),
            expire_time: None,
            include_referenced_resources: None,
        };
        let resp = self
            .client
            .call(|api| {
                api.transactions_initiate_transaction()
                    .vault_id(self.vault.as_str())
                    .body(body)
                    .send()
            })
            .await?;
        Ok(Initiated {
            request_id,
            transaction: resp.transaction.map(Transaction::from),
        })
    }
}

pub struct ListTransactionsBuilder<'a> {
    client: &'a UtilaClient,
    vault: VaultId,
    filter: Option<String>,
    order_by: Option<String>,
    page_size: Option<u32>,
    page_token: Option<String>,
}

impl<'a> ListTransactionsBuilder<'a> {
    /// A server-side filter expression (see the Utila filtering docs).
    pub fn filter(mut self, f: impl Into<String>) -> Self {
        self.filter = Some(f.into());
        self
    }
    pub fn order_by(mut self, o: impl Into<String>) -> Self {
        self.order_by = Some(o.into());
        self
    }
    pub fn page_size(mut self, n: u32) -> Self {
        self.page_size = Some(n);
        self
    }
    pub fn page_token(mut self, t: impl Into<String>) -> Self {
        self.page_token = Some(t.into());
        self
    }

    pub async fn send(self) -> Result<TransactionPage> {
        fetch_transactions(
            self.client,
            &self.vault,
            self.filter.as_deref(),
            self.order_by.as_deref(),
            self.page_token.as_deref(),
            self.page_size,
        )
        .await
    }

    /// Stream every transaction across all pages, honoring `filter`/`order_by` and starting
    /// from `page_token` if set.
    pub fn stream(self) -> impl Stream<Item = Result<Transaction>> + 'a {
        transaction_stream(
            self.client,
            self.vault,
            self.filter,
            self.order_by,
            self.page_size,
            self.page_token,
        )
    }
}

pub struct ReplaceBuilder<'a> {
    client: &'a UtilaClient,
    vault: VaultId,
    transaction: TransactionId,
    replacement: ReplacementType,
    note: Option<String>,
    designated_signers: Vec<UserRef>,
}

impl ReplaceBuilder<'_> {
    /// A note for the replacement transaction (visible to all vault members).
    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
    /// Designate specific signers (build each with [`UserRef::new`], from a user id or email).
    pub fn designated_signers(mut self, signers: Vec<UserRef>) -> Self {
        self.designated_signers = signers;
        self
    }

    pub async fn send(self) -> Result<Transaction> {
        let ReplaceBuilder {
            client,
            vault,
            transaction,
            replacement,
            note,
            designated_signers,
        } = self;
        let body = TransactionsReplaceTransactionBody {
            designated_signers: designated_signers.iter().map(ToString::to_string).collect(),
            expire_time: None,
            include_referenced_resources: None,
            note,
            type_: replacement.into(),
        };
        let resp = client
            .call(|api| {
                api.transactions_replace_transaction()
                    .vault_id(vault.as_str())
                    .transaction_id(transaction.as_str())
                    .body(body)
                    .send()
            })
            .await?;
        resp.transaction
            .map(Transaction::from)
            .ok_or_else(|| missing("transaction"))
    }
}

pub struct EstimateFeeBuilder<'a> {
    client: &'a UtilaClient,
    vault: VaultId,
    details: TransactionDetails,
    priority: Option<Priority>,
}

impl EstimateFeeBuilder<'_> {
    pub fn priority(mut self, p: Priority) -> Self {
        self.priority = Some(p);
        self
    }

    pub async fn send(self) -> Result<FeeEstimate> {
        let EstimateFeeBuilder {
            client,
            vault,
            details,
            priority,
        } = self;
        let body = TransactionsEstimateTransactionFeeBody {
            details: map_estimate_details(details),
            priority: priority.map(Into::into),
        };
        let resp = client
            .call(|api| {
                api.transactions_estimate_transaction_fee()
                    .vault_id(vault.as_str())
                    .body(body)
                    .send()
            })
            .await?;
        Ok(FeeEstimate::from(resp))
    }
}

enum PageState {
    First,
    Next(String),
    Done,
}

fn transaction_stream(
    client: &UtilaClient,
    vault: VaultId,
    filter: Option<String>,
    order_by: Option<String>,
    page_size: Option<u32>,
    first_token: Option<String>,
) -> impl Stream<Item = Result<Transaction>> + '_ {
    let init = match first_token {
        Some(t) => PageState::Next(t),
        None => PageState::First,
    };
    stream::try_unfold(init, move |state| {
        let vault = vault.clone();
        let filter = filter.clone();
        let order_by = order_by.clone();
        async move {
            let token = match state {
                PageState::First => None,
                PageState::Next(t) => Some(t),
                PageState::Done => return Ok::<_, ApiError>(None),
            };
            let page = fetch_transactions(
                client,
                &vault,
                filter.as_deref(),
                order_by.as_deref(),
                token.as_deref(),
                page_size,
            )
            .await?;
            let next = match page.next_page_token {
                Some(t) => PageState::Next(t),
                None => PageState::Done,
            };
            let items = stream::iter(
                page.transactions
                    .into_iter()
                    .map(Ok::<Transaction, ApiError>),
            );
            Ok(Some((items, next)))
        }
    })
    .try_flatten()
}

async fn fetch_transactions(
    client: &UtilaClient,
    vault: &VaultId,
    filter: Option<&str>,
    order_by: Option<&str>,
    page_token: Option<&str>,
    page_size: Option<u32>,
) -> Result<TransactionPage> {
    let resp: V2ListTransactionsResponse = client
        .call(|api| {
            let mut b = api
                .transactions_list_transactions()
                .vault_id(vault.as_str());
            if let Some(f) = filter {
                b = b.filter(f);
            }
            if let Some(o) = order_by {
                b = b.order_by(o);
            }
            if let Some(n) = page_size {
                b = b.page_size(n);
            }
            if let Some(t) = page_token {
                b = b.page_token(t);
            }
            b.send()
        })
        .await?;
    Ok(resp.into())
}

/// A missing top-level response field surfaces as a synthetic API error.
fn missing(field: &str) -> ApiError {
    ApiError::missing(field)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generated::types::V2TransactionPriorityEnum as E;

    #[test]
    fn priority_maps_every_variant() {
        assert!(matches!(E::from(Priority::Low), E::Low));
        assert!(matches!(E::from(Priority::Normal), E::Normal));
        assert!(matches!(E::from(Priority::High), E::High));
    }

    #[test]
    fn transaction_kind_maps_both_variants() {
        assert_eq!(
            TransactionKind::from(V2TransactionTypeEnum::Transaction),
            TransactionKind::Transaction
        );
        assert_eq!(
            TransactionKind::from(V2TransactionTypeEnum::Message),
            TransactionKind::Message
        );
    }

    #[test]
    fn vote_and_replacement_map_every_variant() {
        use crate::generated::types::{
            V2TransactionReplacementTypeEnum as R, V2VoteOnTransactionRequestRequestVote as V,
        };
        assert!(matches!(V::from(Vote::Approve), V::Approve));
        assert!(matches!(V::from(Vote::Deny), V::Deny));
        assert!(matches!(R::from(ReplacementType::Cancel), R::Cancel));
        assert!(matches!(
            R::from(ReplacementType::Accelerate),
            R::Accelerate
        ));
    }

    fn test_client() -> UtilaClient {
        UtilaClient::builder()
            .credential(
                "a",
                crate::auth::SignerSource::local_pem(include_bytes!("../../tests/test_key.pem"))
                    .unwrap(),
            )
            .base_url("http://localhost")
            .build()
            .unwrap()
    }

    /// Every per-kind entry method funnels the right [`TransactionDetails`] variant into the
    /// returned builder (without issuing a request).
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one assertion per kind; a flat list is clearer than splitting"
    )]
    fn per_kind_methods_select_the_right_variant() {
        use rust_decimal::dec;
        let c = test_client();
        let v = || VaultId::new("vault");
        let txs = c.transactions();

        macro_rules! is_variant {
            ($builder:expr, $pat:pat) => {
                assert!(matches!($builder.details, $pat));
            };
        }

        is_variant!(
            txs.asset_transfer(
                v(),
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
                }
            ),
            TransactionDetails::AssetTransfer(_)
        );
        is_variant!(
            txs.batch_asset_transfer(
                v(),
                BatchAssetTransfer {
                    asset: "assets/x".into(),
                    source: "s".into(),
                    destinations: vec![],
                }
            ),
            TransactionDetails::BatchAssetTransfer(_)
        );
        is_variant!(
            txs.evm(
                v(),
                EvmTransaction {
                    from_address: "0xa".into(),
                    network: "ethereum-mainnet".into(),
                    to: None,
                    value: None,
                    data: None,
                    publish: None,
                }
            ),
            TransactionDetails::Evm(_)
        );
        is_variant!(
            txs.evm_personal_sign(
                v(),
                EvmPersonalSign {
                    from_address: "0xa".into(),
                    message: None,
                    message_hex: None,
                }
            ),
            TransactionDetails::EvmPersonalSign(_)
        );
        is_variant!(
            txs.evm_typed_data(
                v(),
                EvmTypedData {
                    from_address: "0xa".into(),
                    message: "{}".into(),
                }
            ),
            TransactionDetails::EvmTypedData(_)
        );
        is_variant!(
            txs.evm_account_delegation(
                v(),
                EvmAccountDelegation {
                    from_address: "0xa".into(),
                    contract_address: "0xc".into(),
                    chain_id: None,
                    nonce: None,
                    offset_nonce: None,
                }
            ),
            TransactionDetails::EvmAccountDelegation(_)
        );
        is_variant!(
            txs.exchange_withdrawal(
                v(),
                ExchangeWithdrawal {
                    asset: "assets/x".into(),
                    amount: dec!(1),
                    source: "s".into(),
                    destination: "d".into(),
                    destination_network: "n".into(),
                    pay_fee_from_amount: None,
                }
            ),
            TransactionDetails::ExchangeWithdrawal(_)
        );
        is_variant!(
            txs.solana_raw(
                v(),
                SolanaRaw {
                    network: "solana-mainnet".into(),
                    raw_transaction: "x".into(),
                    publish: None,
                    replace_blockhash: None,
                    try_replace_blockhash: None,
                }
            ),
            TransactionDetails::SolanaRaw(_)
        );
        is_variant!(
            txs.stellar(
                v(),
                StellarTransaction::builder()
                    .network("stellar-mainnet")
                    .source_address("G")
                    .build()
                    .unwrap()
            ),
            TransactionDetails::Stellar(_)
        );
        is_variant!(
            txs.stellar_raw(
                v(),
                StellarRaw {
                    network: "stellar-mainnet".into(),
                    source_address: "G".into(),
                    xdr_envelope: "AAAA".into(),
                    publish: None,
                    use_latest_sequence_number: None,
                }
            ),
            TransactionDetails::StellarRaw(_)
        );
        is_variant!(
            txs.sui_raw(
                v(),
                SuiRaw {
                    network: "sui-mainnet".into(),
                    sender: "0xs".into(),
                    tx_bcs_bytes: "AAAA".into(),
                    publish: None,
                }
            ),
            TransactionDetails::SuiRaw(_)
        );
        is_variant!(
            txs.tron(
                v(),
                TronTransaction::builder()
                    .network("tron-mainnet")
                    .action(crate::transactions::TronAction::WithdrawBalance {
                        owner_address: "T".into(),
                    })
                    .build()
                    .unwrap()
            ),
            TransactionDetails::Tron(_)
        );
        is_variant!(
            txs.tron_trigger_contract(
                v(),
                TronTriggerSmartContract {
                    network: "tron-mainnet".into(),
                    owner_address: "T".into(),
                    contract_address: "Tc".into(),
                    data: None,
                    call_value: None,
                }
            ),
            TransactionDetails::TronTriggerContract(_)
        );
        is_variant!(
            txs.xrpl_raw(
                v(),
                XrplRaw {
                    network: "xrpl-mainnet".into(),
                    sender: "r".into(),
                    json_transaction_data: serde_json::Map::new(),
                    publish: None,
                }
            ),
            TransactionDetails::XrplRaw(_)
        );
    }
}
