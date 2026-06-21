//! Solana transaction detail kinds.

use crate::generated::types as g;
use crate::resource::NetworkId;

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
