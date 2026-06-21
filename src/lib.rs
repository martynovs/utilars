//! # utilars — Rust client for the Utila v2 API
//!
//! A typed, async client for the Utila v2 custody API, exposed as a grouped facade:
//! `client.vaults()`, `client.wallets()`, `client.balances()`, `client.transactions()`,
//! `client.networks()`, `client.address_book()`, `client.assets()`. Authentication is a
//! self-signed RS256 service-account JWT (minted, cached, refreshed); amounts are exact
//! integer base units ([`Amount`]) with assets resolved to decimals/symbol; webhook
//! verification lives in [`webhook`].
//!
//! The low-level
//! transport is generated from Utila's OpenAPI spec with progenitor; this facade adds typed
//! inputs/outputs, pagination, and asset enrichment. The AWS KMS signer is behind the
//! off-by-default `aws` feature. See the crate README for a full tour.
//!
//! ```no_run
//! # async fn demo(service_account_pem: &[u8]) -> utilars::Result<()> {
//! use utilars::{UtilaClient, SignerSource, VaultId, AssetTransfer, Priority};
//! use rust_decimal::dec;
//!
//! // The caller supplies the account + key (e.g. read from its own env/secrets);
//! // the library never touches the environment itself.
//! let client = UtilaClient::builder()
//!     .credential(
//!         "my-sa@vault-a1b2c3d4.utilaserviceaccount.io",
//!         SignerSource::local_pem(service_account_pem)?,
//!     )
//!     .build()?;
//!
//! // list + stream
//! let page = client.vaults().list().page_size(50).send().await?;
//! use futures::TryStreamExt;
//! let all: Vec<_> = client.vaults().stream().try_collect().await?;
//!
//! // balances, with each asset's decimals/symbol resolved + cached
//! for bal in client.balances().query(VaultId::new("abc")).await? {
//!     println!("{} {:?}", bal.amount, bal.asset.symbol());
//! }
//!
//! // initiate a transfer (idempotency key auto-generated); per-kind method, `send()` terminal
//! let out = client.transactions().asset_transfer(
//!     VaultId::new("abc"),
//!     AssetTransfer {
//!         asset: "assets/native.ethereum-mainnet".into(),
//!         source: "vaults/abc/wallets/w1".into(),
//!         destination: "0x00".into(),
//!         amount: dec!(1.5),
//!         memo: None,
//!         // a sponsor (gas) wallet would pay the fee; here a plain transfer:
//!         sponsor: None,
//!         pay_fee_from_amount: None,
//!         stellar_memo: None,
//!         xrpl_destination_tag: None,
//!     },
//! ).priority(Priority::High).send().await?;
//! println!("request_id = {}", out.request_id);
//! # Ok(())
//! # }
//! ```
//!
//! ## Retrying
//!
//! Retry is intentionally *not* built in — each call issues a single request. To retry,
//! wrap a whole operation with a retry crate (we recommend [`backon`](https://docs.rs/backon))
//! and gate it on [`ApiError::is_retryable`], which classifies transient transport/server
//! errors:
//!
//! ```no_run
//! # async fn demo(client: utilars::UtilaClient, id: utilars::VaultId) -> utilars::Result<()> {
//! use backon::{ExponentialBuilder, Retryable};
//! use utilars::ApiError;
//!
//! let vault = (|| client.vaults().get(id.clone()))
//!     .retry(ExponentialBuilder::default())
//!     .when(ApiError::is_retryable)
//!     .await?;
//! # let _ = vault; Ok(())
//! # }
//! ```

mod amount;
mod asset_cache;
mod assets;
mod auth;
mod balances;
mod client;
mod error;
mod macros;
#[rustfmt::skip]
mod generated;
mod address_book;
mod kms;
mod networks;
mod resource;
mod transactions;
mod vaults;
mod wallets;
pub mod webhook;
mod webhook_event;

pub use address_book::{
    AddressBook, AddressBookEntry, AddressBookPage, ListAddressBookEntriesBuilder,
    NewAddressBookEntry, VaultAction, VaultActionStatus,
};
pub use amount::{Amount, AmountError};
pub use assets::{Asset, Assets, ResolvedAsset};
pub use auth::{SignerSource, TokenManager};
pub use balances::{
    Balance, Balances, QueryUtxosBuilder, QueryWalletAddressBalancesBuilder,
    QueryWalletBalancesBuilder, Utxo, UtxoPage, UtxoState, WalletAddressBalance,
    WalletAddressBalancePage, WalletBalance, WalletBalancePage,
};
pub use client::{UtilaClient, UtilaClientBuilder};
pub use error::{ApiError, Result};
pub use kms::{AwsKmsArn, KmsKey};
pub use networks::{
    BatchContract, ListNetworksBuilder, Network, NetworkPage, NetworkStatus, Networks,
};
pub use resource::{
    AddressBookEntryId, AddressBookEntryRef, AddressId, AssetId, GasStationId, GasStationRef,
    NetworkId, NetworkRef, ParseRef, ResourceName, SimulationId, SimulationRef, TransactionId,
    TransactionRef, TransactionRequestId, TransactionRequestRef, UserId, UserRef, VaultId,
    VaultRef, WalletAddressRef, WalletId, WalletRef,
};
pub use transactions::{
    AddressBalanceChange, AmlScreening, AssetTransfer, BalanceChange, BatchAssetTransfer,
    BatchDestination, EstimateFeeBuilder, EvmAccountDelegation, EvmFee, EvmPersonalSign,
    EvmTransaction, EvmTypedData, ExchangeWithdrawal, FeeEstimate, InitiateBuilder, Initiated,
    ListTransactionsBuilder, Priority, ReplaceBuilder, ReplacementType, Simulation, SolanaRaw,
    Sponsor, StellarMemo, StellarMemoType, StellarOpBody, StellarOperation, StellarRaw,
    StellarTimeBounds, StellarTransaction, StellarTransactionBuilder, SuiRaw, Transaction,
    TransactionDetails, TransactionKind, TransactionPage, TransactionRequest, Transactions,
    Transfer, TransferEndpoint, TronAction, TronContractCall, TronDelegate, TronFee, TronFreeze,
    TronResource, TronTransaction, TronTransactionBuilder, TronTriggerSmartContract,
    TronUndelegate, TronVote, Vote, XrplRaw,
};
pub use vaults::{ListVaultsBuilder, Vault, VaultPage, Vaults};
pub use wallets::{
    ArchiveWalletBuilder, BatchArchiveWalletsBuilder, CreateAddressBuilder, CreateWalletBuilder,
    ListAddressesBuilder, ListWalletsBuilder, Wallet, WalletAddress, WalletAddressPage, WalletPage,
    Wallets,
};
// `TransactionState`/`AmlAction` are shared by the transactions surface and webhook events; they
// are also re-exported under [`webhook`] for receiver-side code.
pub use webhook_event::{AmlAction, TransactionState};
