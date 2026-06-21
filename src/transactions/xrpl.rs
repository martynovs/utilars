//! XRPL transaction detail kinds.

use crate::generated::types as g;
use crate::resource::NetworkId;

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
