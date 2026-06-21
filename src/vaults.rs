//! The `vaults()` group: models, list (page + stream), and get — over the generated ops.

use chrono::{DateTime, Utc};
use futures::stream::{self, Stream, TryStreamExt};

use crate::client::UtilaClient;
use crate::error::{ApiError, Result};
use crate::generated::types::{V2ListVaultsResponse, V2Vault};
use crate::generated::ClientVaultsExt;
use crate::resource::{ResourceName, VaultId, VaultRef};

/// A Utila vault. `name` is the resource name, e.g. `vaults/abc123`.
#[derive(Debug, Clone)]
pub struct Vault {
    pub name: ResourceName<VaultRef>,
    pub display_name: Option<String>,
    pub archived: bool,
    pub create_time: Option<DateTime<Utc>>,
}

impl From<V2Vault> for Vault {
    fn from(v: V2Vault) -> Self {
        Self {
            name: ResourceName::parse(v.name.unwrap_or_default()),
            display_name: (!v.display_name.is_empty()).then_some(v.display_name),
            archived: v.archived.unwrap_or(false),
            create_time: v.create_time,
        }
    }
}

/// One page of `vaults().list()`.
#[derive(Debug, Clone)]
pub struct VaultPage {
    pub vaults: Vec<Vault>,
    pub next_page_token: Option<String>,
    pub total_size: i32,
}

impl From<V2ListVaultsResponse> for VaultPage {
    fn from(r: V2ListVaultsResponse) -> Self {
        Self {
            vaults: r.vaults.into_iter().map(Vault::from).collect(),
            next_page_token: r.next_page_token.filter(|t| !t.is_empty()),
            total_size: r.total_size.unwrap_or(0),
        }
    }
}

pub struct Vaults<'a> {
    pub(crate) client: &'a UtilaClient,
}

impl<'a> Vaults<'a> {
    /// Single-page listing with explicit control over page size / token.
    pub fn list(&self) -> ListVaultsBuilder<'a> {
        ListVaultsBuilder {
            client: self.client,
            page_size: None,
            page_token: None,
        }
    }

    /// Get one vault by id.
    pub async fn get(&self, id: VaultId) -> Result<Vault> {
        let resp = self
            .client
            .call(|api| api.vaults_get_vault().vault_id(id.as_str()).send())
            .await?;
        resp.vault
            .map(Vault::from)
            .ok_or_else(|| ApiError::missing("vault"))
    }

    /// Stream every vault across all pages.
    pub fn stream(&self) -> impl Stream<Item = Result<Vault>> + 'a {
        let client = self.client;
        stream::try_unfold(PageState::First, move |state| async move {
            let token = match state {
                PageState::First => None,
                PageState::Next(t) => Some(t),
                PageState::Done => return Ok::<_, ApiError>(None),
            };
            let page = fetch_page(client, token.as_deref()).await?;
            let next = match page.next_page_token {
                Some(t) => PageState::Next(t),
                None => PageState::Done,
            };
            let items = stream::iter(page.vaults.into_iter().map(Ok::<Vault, ApiError>));
            Ok(Some((items, next)))
        })
        .try_flatten()
    }
}

enum PageState {
    First,
    Next(String),
    Done,
}

async fn fetch_page(client: &UtilaClient, page_token: Option<&str>) -> Result<VaultPage> {
    let resp: V2ListVaultsResponse = client
        .call(|api| {
            let mut b = api.vaults_list_vaults();
            if let Some(t) = page_token {
                b = b.page_token(t);
            }
            b.send()
        })
        .await?;
    Ok(resp.into())
}

pub struct ListVaultsBuilder<'a> {
    client: &'a UtilaClient,
    page_size: Option<u32>,
    page_token: Option<String>,
}

impl ListVaultsBuilder<'_> {
    pub fn page_size(mut self, n: u32) -> Self {
        self.page_size = Some(n);
        self
    }
    pub fn page_token(mut self, t: impl Into<String>) -> Self {
        self.page_token = Some(t.into());
        self
    }

    pub async fn send(self) -> Result<VaultPage> {
        let resp: V2ListVaultsResponse = self
            .client
            .call(|api| {
                let mut b = api.vaults_list_vaults();
                if let Some(n) = self.page_size {
                    b = b.page_size(n);
                }
                if let Some(t) = &self.page_token {
                    b = b.page_token(t.as_str());
                }
                b.send()
            })
            .await?;
        Ok(resp.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_from_maps_present_and_absent_fields() {
        // All optional fields present (archived/createTime never appear in the wire tests).
        let create_time = "2021-02-03T04:05:06Z"
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap();
        let full = Vault::from(V2Vault {
            archived: Some(true),
            create_time: Some(create_time),
            display_name: "Treasury".into(),
            name: Some("vaults/abc".into()),
        });
        assert_eq!(full.name.to_string(), "vaults/abc");
        assert_eq!(full.display_name.as_deref(), Some("Treasury"));
        assert!(full.archived);
        assert_eq!(full.create_time, Some(create_time));

        // The absent/empty branches: empty display → None, missing name/archived/time → defaults.
        let bare = Vault::from(V2Vault {
            archived: None,
            create_time: None,
            display_name: String::new(),
            name: None,
        });
        assert_eq!(bare.name.to_string(), "");
        assert_eq!(bare.display_name, None);
        assert!(!bare.archived);
        assert_eq!(bare.create_time, None);
    }

    #[test]
    fn vault_page_from_drops_empty_next_token() {
        let page = VaultPage::from(V2ListVaultsResponse {
            next_page_token: Some(String::new()), // empty token → no next page
            total_size: None,
            vaults: vec![],
        });
        assert!(page.next_page_token.is_none());
        assert_eq!(page.total_size, 0);
    }
}
