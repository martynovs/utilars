//! Tron transaction detail kinds: the top-level `triggerSmartContract` plus the nested
//! staking/resource [`TronTransaction`] and its [`TronAction`] variants.

use crate::error::{ApiError, Result};
use crate::generated::types as g;
use crate::resource::NetworkId;

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
                .ok_or_else(|| ApiError::Config("TronTransaction: network is required".into()))?,
            action: self
                .action
                .ok_or_else(|| ApiError::Config("TronTransaction: action is required".into()))?,
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
