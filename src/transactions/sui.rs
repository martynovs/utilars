//! Sui transaction detail kinds.

use crate::generated::types as g;
use crate::resource::NetworkId;

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
