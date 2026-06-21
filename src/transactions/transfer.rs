//! The chain-agnostic value-transfer detail kinds: single and batch asset transfers and
//! exchange withdrawals.

use rust_decimal::Decimal;

use super::details::{Sponsor, TransferEndpoint};
use super::stellar::StellarMemo;
use crate::generated::types as g;
use crate::resource::AssetId;

/// Transfer an asset from a source wallet to a destination address. `assetTransfer` is the
/// API's one **cross-chain** transfer primitive; the optional fields below are the per-chain
/// riders it carries (they only apply when the asset lives on the relevant chain) — distinct
/// from the per-network *transaction* kinds ([`EvmTransaction`], [`TronTransaction`], …).
///
/// [`EvmTransaction`]: super::EvmTransaction
/// [`TronTransaction`]: super::TronTransaction
#[derive(Debug, Clone)]
pub struct AssetTransfer {
    pub asset: AssetId,
    /// Where the asset is sent from (a wallet, wallet address, or raw address).
    pub source: TransferEndpoint,
    /// Where the asset is sent to (a raw address, wallet, or wallet address).
    pub destination: TransferEndpoint,
    /// Amount in DISPLAY units (whole tokens, e.g. `1.5`).
    pub amount: Decimal,
    pub memo: Option<String>,
    /// A sponsor that pays the network fee — a *native sponsored transfer*
    /// (EVM / Solana / Sui / Aptos). `source` still sends the asset; `sponsor` only pays fees.
    pub sponsor: Option<Sponsor>,
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
            destination: t.destination.to_wire(),
            source: t.source.to_wire(),
            memo: t.memo,
            pay_fee_from_amount: t.pay_fee_from_amount,
            sponsor: t.sponsor.map(|s| s.to_wire()),
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
    pub source: TransferEndpoint,
    pub destinations: Vec<BatchDestination>,
}

/// One leg of a [`BatchAssetTransfer`].
#[derive(Debug, Clone)]
pub struct BatchDestination {
    pub destination: TransferEndpoint,
    /// Amount in DISPLAY units.
    pub amount: Decimal,
    pub note: Option<String>,
}

impl From<BatchDestination> for g::BatchAssetTransferBatchTransferDestination {
    fn from(d: BatchDestination) -> Self {
        g::BatchAssetTransferBatchTransferDestination {
            amount: d.amount.to_string(),
            destination: d.destination.to_wire(),
            note: d.note,
        }
    }
}

impl From<BatchAssetTransfer> for g::V2BatchAssetTransfer {
    fn from(t: BatchAssetTransfer) -> Self {
        g::V2BatchAssetTransfer {
            asset: t.asset.as_str().to_string(),
            source: t.source.to_wire(),
            destinations: t.destinations.into_iter().map(Into::into).collect(),
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
