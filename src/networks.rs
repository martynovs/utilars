//! The `networks()` group (the API's "blockchains" tag): list/get networks, the networks
//! enabled for a vault, and a network's latest batch (multicall) contract — curated over
//! `ClientBlockchainsExt`.

use futures::stream::{self, Stream, TryStreamExt};

use crate::client::{enum_label, UtilaClient};
use crate::error::{Result, UtilaError};
use crate::generated::types::{
    V2BatchContract, V2ListNetworksResponse, V2ListVaultNetworksResponse, V2Network,
};
use crate::generated::ClientBlockchainsExt;
use crate::ids::{AssetId, NetworkId, VaultId};

/// A blockchain network Utila supports. `name` is the resource name, e.g.
/// `networks/ethereum-mainnet`.
#[derive(Debug, Clone)]
pub struct Network {
    pub name: String,
    pub display_name: Option<String>,
    pub native_asset: Option<AssetId>,
    pub status: Option<String>,
    pub testnet: bool,
    pub custom: bool,
}

impl From<V2Network> for Network {
    fn from(n: V2Network) -> Self {
        Self {
            name: n.name.unwrap_or_default(),
            display_name: n.display_name.filter(|s| !s.is_empty()),
            native_asset: n.native_asset.filter(|s| !s.is_empty()).map(AssetId::from),
            status: n.status.as_ref().and_then(enum_label),
            testnet: n.testnet.unwrap_or(false),
            custom: n.custom.unwrap_or(false),
        }
    }
}

/// The latest batch (multicall) contract deployed for a network.
#[derive(Debug, Clone)]
pub struct BatchContract {
    pub name: String,
    pub address: Option<String>,
    pub network: Option<NetworkId>,
}

impl From<V2BatchContract> for BatchContract {
    fn from(c: V2BatchContract) -> Self {
        Self {
            name: c.name.unwrap_or_default(),
            address: c.address.filter(|s| !s.is_empty()),
            network: c.network.filter(|s| !s.is_empty()).map(NetworkId::from),
        }
    }
}

/// One page of networks.
#[derive(Debug, Clone)]
pub struct NetworkPage {
    pub networks: Vec<Network>,
    pub next_page_token: Option<String>,
}

impl From<V2ListNetworksResponse> for NetworkPage {
    fn from(r: V2ListNetworksResponse) -> Self {
        Self {
            networks: r.networks.into_iter().map(Network::from).collect(),
            next_page_token: r.next_page_token.filter(|t| !t.is_empty()),
        }
    }
}

impl From<V2ListVaultNetworksResponse> for NetworkPage {
    fn from(r: V2ListVaultNetworksResponse) -> Self {
        Self {
            networks: r.networks.into_iter().map(Network::from).collect(),
            next_page_token: r.next_page_token.filter(|t| !t.is_empty()),
        }
    }
}

pub struct Networks<'a> {
    pub(crate) client: &'a UtilaClient,
}

impl<'a> Networks<'a> {
    /// List supported networks (single page).
    pub fn list(&self) -> ListNetworksBuilder<'a> {
        ListNetworksBuilder {
            client: self.client,
            vault: None,
            page_size: None,
            page_token: None,
        }
    }

    /// List the networks enabled for a vault (single page).
    pub fn list_for_vault(&self, vault: VaultId) -> ListNetworksBuilder<'a> {
        ListNetworksBuilder {
            client: self.client,
            vault: Some(vault),
            page_size: None,
            page_token: None,
        }
    }

    /// Get one network by id.
    pub async fn get(&self, id: NetworkId) -> Result<Network> {
        let resp = self
            .client
            .call(|api| api.blockchains_get_network().network_id(id.as_str()).send())
            .await?;
        resp.network
            .map(Network::from)
            .ok_or_else(|| UtilaError::missing("network"))
    }

    /// The latest batch (multicall) contract for a network.
    pub async fn latest_batch_contract(&self, network: NetworkId) -> Result<BatchContract> {
        let contract = self
            .client
            .call(|api| {
                api.blockchains_get_latest_batch_contract()
                    .network_id(network.as_str())
                    .send()
            })
            .await?;
        Ok(BatchContract::from(contract))
    }

    /// Stream every supported network across all pages.
    pub fn stream(&self) -> impl Stream<Item = Result<Network>> + 'a {
        network_stream(self.client, None)
    }

    /// Stream every network enabled for a vault.
    pub fn stream_for_vault(&self, vault: VaultId) -> impl Stream<Item = Result<Network>> + 'a {
        network_stream(self.client, Some(vault))
    }
}

pub struct ListNetworksBuilder<'a> {
    client: &'a UtilaClient,
    vault: Option<VaultId>,
    page_size: Option<u32>,
    page_token: Option<String>,
}

impl ListNetworksBuilder<'_> {
    pub fn page_size(mut self, n: u32) -> Self {
        self.page_size = Some(n);
        self
    }
    pub fn page_token(mut self, t: impl Into<String>) -> Self {
        self.page_token = Some(t.into());
        self
    }

    pub async fn send(self) -> Result<NetworkPage> {
        fetch_networks(
            self.client,
            self.vault.as_ref(),
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

fn network_stream(
    client: &UtilaClient,
    vault: Option<VaultId>,
) -> impl Stream<Item = Result<Network>> + '_ {
    stream::try_unfold(PageState::First, move |state| {
        let vault = vault.clone();
        async move {
            let token = match state {
                PageState::First => None,
                PageState::Next(t) => Some(t),
                PageState::Done => return Ok::<_, UtilaError>(None),
            };
            let page = fetch_networks(client, vault.as_ref(), token.as_deref(), None).await?;
            let next = match page.next_page_token {
                Some(t) => PageState::Next(t),
                None => PageState::Done,
            };
            let items = stream::iter(page.networks.into_iter().map(Ok::<Network, UtilaError>));
            Ok(Some((items, next)))
        }
    })
    .try_flatten()
}

async fn fetch_networks(
    client: &UtilaClient,
    vault: Option<&VaultId>,
    page_token: Option<&str>,
    page_size: Option<u32>,
) -> Result<NetworkPage> {
    if let Some(vault) = vault {
        let resp: V2ListVaultNetworksResponse = client
            .call(|api| {
                let mut b = api
                    .blockchains_list_vault_networks()
                    .vault_id(vault.as_str());
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
    } else {
        let resp: V2ListNetworksResponse = client
            .call(|api| {
                let mut b = api.blockchains_list_networks();
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
}
