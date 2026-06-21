//! EVM transaction detail kinds: raw transactions, `personal_sign`, typed data, and
//! EIP-7702 account delegation.

use crate::generated::types as g;
use crate::resource::NetworkId;

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
