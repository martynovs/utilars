//! The `address_book()` group: curated over `ClientAddressBookExt`. Lists a vault's
//! address book entries (single page + stream), batch-reads them by resource name, and
//! the three mutating quorum operations (batch create, unsigned batch create, batch add
//! to a group) that each return a pending [`VaultAction`].

use chrono::{DateTime, Utc};
use futures::stream::{self, Stream, TryStreamExt};

use crate::client::UtilaClient;
use crate::error::{ApiError, Result};
use crate::generated::types::{
    AddressBookBatchAddAddressBookEntriesToGroupBody, AddressBookBatchCreateAddressBookEntriesBody,
    AddressBookBatchCreateUnsignedAddressBookEntriesBody, V2AddressBookEntry,
    V2CreateAddressBookEntryRequest, V2ListAddressBookEntriesResponse, V2VaultAction,
    V2VaultActionStatusEnum,
};
use crate::generated::ClientAddressBookExt;
use crate::resource::{
    AddressBookEntryId, AddressBookEntryRef, NetworkId, ResourceName, UserRef, VaultActionRef,
    VaultId, WalletRef,
};

/// A labelled external address tracked in a vault's address book. `name` is the resource
/// name, e.g. `vaults/{vault_id}/addressBookEntries/{entry_id}`.
#[derive(Debug, Clone)]
pub struct AddressBookEntry {
    pub name: ResourceName<AddressBookEntryRef>,
    pub display_name: Option<String>,
    /// The blockchain address.
    pub address: String,
    pub network: Option<NetworkId>,
    pub note: Option<String>,
    /// Whether Utila tracks assets held by this address.
    pub tracked: bool,
    /// The Utila wallet this address is associated with (only set when `tracked`).
    pub associated_wallet: Option<ResourceName<WalletRef>>,
    /// The user who created the entry.
    pub creator: Option<ResourceName<UserRef>>,
    pub create_time: Option<DateTime<Utc>>,
}

impl From<V2AddressBookEntry> for AddressBookEntry {
    fn from(e: V2AddressBookEntry) -> Self {
        Self {
            name: ResourceName::parse(e.name.unwrap_or_default()),
            display_name: (!e.display_name.is_empty()).then_some(e.display_name),
            address: e.address,
            network: (!e.network.is_empty()).then(|| NetworkId::from(e.network)),
            note: e.note.filter(|s| !s.is_empty()),
            tracked: e.tracked.unwrap_or(false),
            associated_wallet: e
                .associated_external_wallet
                .filter(|s| !s.is_empty())
                .map(ResourceName::parse),
            creator: e.creator.filter(|s| !s.is_empty()).map(ResourceName::parse),
            create_time: e.create_time,
        }
    }
}

/// A pending quorum action returned by the mutating batch operations. `name` is the
/// resource name, e.g. `vaults/{vault_id}/actions/{action_id}`.
#[derive(Debug, Clone)]
pub struct VaultAction {
    pub name: ResourceName<VaultActionRef>,
    pub status: Option<VaultActionStatus>,
    pub create_time: Option<DateTime<Utc>>,
    pub expire_time: Option<DateTime<Utc>>,
}

impl From<V2VaultAction> for VaultAction {
    fn from(a: V2VaultAction) -> Self {
        Self {
            name: ResourceName::parse(a.name.unwrap_or_default()),
            status: a.status.map(VaultActionStatus::from),
            create_time: a.create_time,
            expire_time: a.expire_time,
        }
    }
}

/// The lifecycle status of a pending quorum [`VaultAction`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum VaultActionStatus {
    /// Awaiting quorum approval.
    Pending,
    /// Rejected by the quorum.
    Rejected,
    /// Approved; awaiting the required signatures.
    AwaitingSignature,
    /// Currently executing.
    Running,
    /// Completed successfully.
    Finished,
    /// Execution failed.
    Failed,
    /// Canceled before completion.
    Canceled,
    /// Expired before reaching quorum.
    Expired,
}

impl From<V2VaultActionStatusEnum> for VaultActionStatus {
    fn from(s: V2VaultActionStatusEnum) -> Self {
        match s {
            V2VaultActionStatusEnum::Pending => Self::Pending,
            V2VaultActionStatusEnum::Rejected => Self::Rejected,
            V2VaultActionStatusEnum::AwaitingSignature => Self::AwaitingSignature,
            V2VaultActionStatusEnum::Running => Self::Running,
            V2VaultActionStatusEnum::Finished => Self::Finished,
            V2VaultActionStatusEnum::Failed => Self::Failed,
            V2VaultActionStatusEnum::Canceled => Self::Canceled,
            V2VaultActionStatusEnum::Expired => Self::Expired,
        }
    }
}

/// A new address book entry to create (the curated `CreateAddressBookEntryRequest`).
#[derive(Debug, Clone)]
pub struct NewAddressBookEntry {
    /// The blockchain address.
    pub address: String,
    pub display_name: String,
    pub network: NetworkId,
    pub note: Option<String>,
}

impl From<NewAddressBookEntry> for V2CreateAddressBookEntryRequest {
    fn from(e: NewAddressBookEntry) -> Self {
        Self {
            address: e.address,
            display_name: e.display_name,
            network: e.network.to_string(),
            note: e.note,
        }
    }
}

/// One page of `address_book().list(vault)`.
#[derive(Debug, Clone)]
pub struct AddressBookPage {
    pub entries: Vec<AddressBookEntry>,
    pub next_page_token: Option<String>,
    pub total_size: i32,
}

impl From<V2ListAddressBookEntriesResponse> for AddressBookPage {
    fn from(r: V2ListAddressBookEntriesResponse) -> Self {
        Self {
            entries: r
                .address_book_entries
                .into_iter()
                .map(AddressBookEntry::from)
                .collect(),
            next_page_token: r.next_page_token.filter(|t| !t.is_empty()),
            total_size: r.total_size.unwrap_or(0),
        }
    }
}

pub struct AddressBook<'a> {
    pub(crate) client: &'a UtilaClient,
}

impl<'a> AddressBook<'a> {
    /// List a vault's address book entries (single page). Add `.filter(..)`, `.order_by(..)`,
    /// `.page_size(..)` and `.page_token(..)` before `.send()`.
    pub fn list(&self, vault: VaultId) -> ListAddressBookEntriesBuilder<'a> {
        ListAddressBookEntriesBuilder {
            client: self.client,
            vault,
            filter: None,
            order_by: None,
            page_size: None,
            page_token: None,
        }
    }

    /// Retrieve multiple entries by resource name.
    pub async fn get_many(
        &self,
        vault: VaultId,
        names: Vec<AddressBookEntryId>,
    ) -> Result<Vec<AddressBookEntry>> {
        let names: Vec<String> = names.into_iter().map(|n| n.to_string()).collect();
        let resp = self
            .client
            .call(|api| {
                api.address_book_batch_get_address_book_entries()
                    .vault_id(vault.as_str())
                    .names(names)
                    .send()
            })
            .await?;
        Ok(resp
            .address_book_entries
            .into_iter()
            .map(AddressBookEntry::from)
            .collect())
    }

    /// Batch create address book entries. Initiates a quorum action.
    pub async fn batch_create(
        &self,
        vault: VaultId,
        entries: Vec<NewAddressBookEntry>,
    ) -> Result<VaultAction> {
        let body = AddressBookBatchCreateAddressBookEntriesBody {
            requests: entries.into_iter().map(Into::into).collect(),
        };
        let resp = self
            .client
            .call(|api| {
                api.address_book_batch_create_address_book_entries()
                    .vault_id(vault.as_str())
                    .body(body)
                    .send()
            })
            .await?;
        require_action(resp.vault_action)
    }

    /// Batch create address book entries without per-entry vault signatures (quorum
    /// approval only). Initiates a quorum action.
    pub async fn batch_create_unsigned(
        &self,
        vault: VaultId,
        entries: Vec<NewAddressBookEntry>,
    ) -> Result<VaultAction> {
        let body = AddressBookBatchCreateUnsignedAddressBookEntriesBody {
            requests: entries.into_iter().map(Into::into).collect(),
        };
        let resp = self
            .client
            .call(|api| {
                api.address_book_batch_create_unsigned_address_book_entries()
                    .vault_id(vault.as_str())
                    .body(body)
                    .send()
            })
            .await?;
        require_action(resp.vault_action)
    }

    /// Batch add existing entries to an address book entry group (e.g.
    /// `vaults/{vault_id}/addressBookEntryGroups/{group_id}`). Initiates a quorum action.
    pub async fn batch_add_to_group(
        &self,
        vault: VaultId,
        group: impl Into<String>,
        names: Vec<AddressBookEntryId>,
    ) -> Result<VaultAction> {
        let body = AddressBookBatchAddAddressBookEntriesToGroupBody {
            address_book_entry_group: group.into(),
            names: names.into_iter().map(|n| n.to_string()).collect(),
        };
        let resp = self
            .client
            .call(|api| {
                api.address_book_batch_add_address_book_entries_to_group()
                    .vault_id(vault.as_str())
                    .body(body)
                    .send()
            })
            .await?;
        require_action(resp.vault_action)
    }

    /// Stream every address book entry in a vault across all pages.
    pub fn stream(&self, vault: VaultId) -> impl Stream<Item = Result<AddressBookEntry>> + 'a {
        entry_stream(self.client, vault)
    }
}

pub struct ListAddressBookEntriesBuilder<'a> {
    client: &'a UtilaClient,
    vault: VaultId,
    filter: Option<String>,
    order_by: Option<String>,
    page_size: Option<u32>,
    page_token: Option<String>,
}

impl ListAddressBookEntriesBuilder<'_> {
    /// A filter expression, e.g. `network("networks/ethereum-mainnet")`.
    pub fn filter(mut self, f: impl Into<String>) -> Self {
        self.filter = Some(f.into());
        self
    }
    /// A comma-separated order, e.g. `create_time desc`.
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

    pub async fn send(self) -> Result<AddressBookPage> {
        fetch_entries(
            self.client,
            &self.vault,
            self.filter.as_deref(),
            self.order_by.as_deref(),
            self.page_token.as_deref(),
            self.page_size,
        )
        .await
    }
}

enum PageState {
    First,
    Next(String),
    Done,
}

fn entry_stream(
    client: &UtilaClient,
    vault: VaultId,
) -> impl Stream<Item = Result<AddressBookEntry>> + '_ {
    stream::try_unfold(PageState::First, move |state| {
        let vault = vault.clone();
        async move {
            let token = match state {
                PageState::First => None,
                PageState::Next(t) => Some(t),
                PageState::Done => return Ok::<_, ApiError>(None),
            };
            let page = fetch_entries(client, &vault, None, None, token.as_deref(), None).await?;
            let next = match page.next_page_token {
                Some(t) => PageState::Next(t),
                None => PageState::Done,
            };
            let items = stream::iter(
                page.entries
                    .into_iter()
                    .map(Ok::<AddressBookEntry, ApiError>),
            );
            Ok(Some((items, next)))
        }
    })
    .try_flatten()
}

async fn fetch_entries(
    client: &UtilaClient,
    vault: &VaultId,
    filter: Option<&str>,
    order_by: Option<&str>,
    page_token: Option<&str>,
    page_size: Option<u32>,
) -> Result<AddressBookPage> {
    let resp: V2ListAddressBookEntriesResponse = client
        .call(|api| {
            let mut b = api
                .address_book_list_address_book_entries()
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

/// A quorum action is the meaningful result of every mutating batch op; treat its absence
/// as a malformed response.
fn require_action(action: Option<V2VaultAction>) -> Result<VaultAction> {
    action
        .map(VaultAction::from)
        .ok_or_else(|| ApiError::missing("vault action"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_action_status_maps_every_variant() {
        use V2VaultActionStatusEnum as E;
        use VaultActionStatus as S;
        assert_eq!(S::from(E::Pending), S::Pending);
        assert_eq!(S::from(E::Rejected), S::Rejected);
        assert_eq!(S::from(E::AwaitingSignature), S::AwaitingSignature);
        assert_eq!(S::from(E::Running), S::Running);
        assert_eq!(S::from(E::Finished), S::Finished);
        assert_eq!(S::from(E::Failed), S::Failed);
        assert_eq!(S::from(E::Canceled), S::Canceled);
        assert_eq!(S::from(E::Expired), S::Expired);
    }
}
