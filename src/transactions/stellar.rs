//! Stellar transaction detail kinds: a pre-built raw envelope and a structured builder with
//! curated operations, memo, and time bounds.

use crate::error::{ApiError, Result};
use crate::generated::types as g;
use crate::resource::NetworkId;

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
                ApiError::Config("StellarTransaction: network is required".into())
            })?,
            source_address: self.source_address.ok_or_else(|| {
                ApiError::Config("StellarTransaction: source_address is required".into())
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
