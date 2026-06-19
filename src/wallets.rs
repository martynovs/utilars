//! The `wallets()` group: curated `Wallet` / `WalletAddress` models over
//! `ClientWalletsExt`, covering the full surface — list/get/create/archive/unarchive
//! wallets (single and batch), and list/get/create/batch-get of their addresses. Both
//! list endpoints expose a single-page builder (`.send()`) and an all-pages `.stream()`.
//!
//! Curated to the meaningful fields: the per-chain detail sub-objects (`btcDetails`,
//! `evmDetails`, `solanaDetails`, …), the `FULL`-view `convertedValue`, and the
//! creation-only `createParams` on a wallet are intentionally omitted, as are an
//! address's `keyDerivationPath`. Add them here if a caller needs them.

use futures::stream::{self, Stream, TryStreamExt};

use crate::client::{enum_label, UtilaClient};
use crate::error::{Result, UtilaError};
use crate::generated::types::{
    V2BatchGetWalletAddressesResponse, V2BatchGetWalletsResponse, V2ListWalletAddressesResponse,
    V2ListWalletsResponse, V2Wallet, V2WalletAddress, WalletsArchiveWalletBody,
    WalletsBatchArchiveWalletsBody, WalletsBatchUnarchiveWalletsBody, WalletsUnarchiveWalletBody,
};
use crate::generated::ClientWalletsExt;
use crate::ids::{AddressId, NetworkId, VaultId, WalletId};

/// A wallet in a vault. `name` is the resource name, e.g.
/// `vaults/{vault_id}/wallets/{wallet_id}`.
#[derive(Debug, Clone)]
pub struct Wallet {
    pub name: String,
    pub display_name: Option<String>,
    /// Resource names of the networks enabled on the wallet (as returned, e.g.
    /// `networks/ethereum-mainnet`).
    pub networks: Vec<NetworkId>,
    pub archived: bool,
    /// External wallets are not managed by Utila.
    pub external: bool,
    /// Some assets are frozen by an AML policy (only populated under the `FULL` view).
    pub has_frozen_assets: bool,
}

impl From<V2Wallet> for Wallet {
    fn from(w: V2Wallet) -> Self {
        Self {
            name: w.name.unwrap_or_default(),
            display_name: (!w.display_name.is_empty()).then_some(w.display_name),
            networks: w
                .networks
                .into_iter()
                .filter(|s| !s.is_empty())
                .map(NetworkId::from)
                .collect(),
            archived: w.archived.unwrap_or(false),
            external: w.external.unwrap_or(false),
            has_frozen_assets: w.has_frozen_assets.unwrap_or(false),
        }
    }
}

/// A single address derived in a wallet for one network. `name` is the resource name,
/// e.g. `vaults/{vault_id}/wallets/{wallet_id}/addresses/{address_id}`.
#[derive(Debug, Clone)]
pub struct WalletAddress {
    pub name: String,
    /// The blockchain address (read-only; populated once derived).
    pub address: Option<String>,
    pub display_name: Option<String>,
    /// Resource name of the address's network (e.g. `networks/bitcoin-mainnet`).
    pub network: NetworkId,
    /// Address format label, e.g. `BITCOIN_P2WPKH`.
    pub format: Option<String>,
    /// Address type label, e.g. `DEPOSIT`.
    pub kind: Option<String>,
    /// Resource name of the MPC key this address was derived from.
    pub key: Option<String>,
    pub note: Option<String>,
}

impl From<V2WalletAddress> for WalletAddress {
    fn from(a: V2WalletAddress) -> Self {
        Self {
            name: a.name.unwrap_or_default(),
            address: a.address.filter(|s| !s.is_empty()),
            display_name: a.display_name.filter(|s| !s.is_empty()),
            network: NetworkId::from(a.network),
            format: a.format.as_ref().and_then(enum_label),
            kind: a.type_.as_ref().and_then(enum_label),
            key: a.key.filter(|s| !s.is_empty()),
            note: a.note.filter(|s| !s.is_empty()),
        }
    }
}

/// One page of wallets.
#[derive(Debug, Clone)]
pub struct WalletPage {
    pub wallets: Vec<Wallet>,
    pub next_page_token: Option<String>,
    pub total_size: i32,
}

impl From<V2ListWalletsResponse> for WalletPage {
    fn from(r: V2ListWalletsResponse) -> Self {
        Self {
            wallets: r.wallets.into_iter().map(Wallet::from).collect(),
            next_page_token: r.next_page_token.filter(|t| !t.is_empty()),
            total_size: r.total_size.unwrap_or(0),
        }
    }
}

/// One page of wallet addresses.
#[derive(Debug, Clone)]
pub struct WalletAddressPage {
    pub addresses: Vec<WalletAddress>,
    pub next_page_token: Option<String>,
    pub total_size: i32,
}

impl From<V2ListWalletAddressesResponse> for WalletAddressPage {
    fn from(r: V2ListWalletAddressesResponse) -> Self {
        Self {
            addresses: r
                .wallet_addresses
                .into_iter()
                .map(WalletAddress::from)
                .collect(),
            next_page_token: r.next_page_token.filter(|t| !t.is_empty()),
            total_size: r.total_size.unwrap_or(0),
        }
    }
}

pub struct Wallets<'a> {
    pub(crate) client: &'a UtilaClient,
}

impl<'a> Wallets<'a> {
    /// List the wallets in a vault (single page via `.send()`, all pages via `.stream()`).
    pub fn list(&self, vault: VaultId) -> ListWalletsBuilder<'a> {
        ListWalletsBuilder {
            client: self.client,
            vault,
            filter: None,
            show_archived: None,
            page_size: None,
            page_token: None,
        }
    }

    /// Get one wallet by id.
    pub async fn get(&self, vault: VaultId, wallet: WalletId) -> Result<Wallet> {
        let resp = self
            .client
            .call(|api| {
                api.wallets_get_wallet()
                    .vault_id(vault.as_str())
                    .wallet_id(wallet.as_str())
                    .send()
            })
            .await?;
        resp.wallet
            .map(Wallet::from)
            .ok_or_else(|| UtilaError::missing("wallet"))
    }

    /// Create a wallet. Networks can be supplied up front via `.network(..)`, or added
    /// later through `create_address`.
    pub fn create(
        &self,
        vault: VaultId,
        display_name: impl Into<String>,
    ) -> CreateWalletBuilder<'a> {
        CreateWalletBuilder {
            client: self.client,
            vault,
            display_name: display_name.into(),
            networks: Vec::new(),
            external: None,
        }
    }

    /// Archive a wallet. Call `.allow_missing(true)` to make it a no-op when absent.
    pub fn archive(&self, vault: VaultId, wallet: WalletId) -> ArchiveWalletBuilder<'a> {
        ArchiveWalletBuilder {
            client: self.client,
            vault,
            wallet,
            allow_missing: None,
            unarchive: false,
        }
    }

    /// Unarchive a wallet.
    pub fn unarchive(&self, vault: VaultId, wallet: WalletId) -> ArchiveWalletBuilder<'a> {
        ArchiveWalletBuilder {
            client: self.client,
            vault,
            wallet,
            allow_missing: None,
            unarchive: true,
        }
    }

    /// Archive up to 1000 wallets by resource name (`vaults/{v}/wallets/{w}`).
    pub fn batch_archive(
        &self,
        vault: VaultId,
        names: Vec<String>,
    ) -> BatchArchiveWalletsBuilder<'a> {
        BatchArchiveWalletsBuilder {
            client: self.client,
            vault,
            names,
            allow_missing: None,
            unarchive: false,
        }
    }

    /// Unarchive up to 1000 wallets by resource name.
    pub fn batch_unarchive(
        &self,
        vault: VaultId,
        names: Vec<String>,
    ) -> BatchArchiveWalletsBuilder<'a> {
        BatchArchiveWalletsBuilder {
            client: self.client,
            vault,
            names,
            allow_missing: None,
            unarchive: true,
        }
    }

    /// Retrieve multiple wallets by resource name (`vaults/{v}/wallets/{w}`).
    pub async fn batch_get(&self, vault: VaultId, names: Vec<String>) -> Result<Vec<Wallet>> {
        let resp: V2BatchGetWalletsResponse = self
            .client
            .call(|api| {
                api.wallets_batch_get_wallets()
                    .vault_id(vault.as_str())
                    .names(names)
                    .send()
            })
            .await?;
        Ok(resp.wallets.into_iter().map(Wallet::from).collect())
    }

    /// List a wallet's addresses (single page via `.send()`, all pages via `.stream()`).
    pub fn list_addresses(&self, vault: VaultId, wallet: WalletId) -> ListAddressesBuilder<'a> {
        ListAddressesBuilder {
            client: self.client,
            vault,
            wallet,
            page_size: None,
            page_token: None,
        }
    }

    /// Get one wallet address by id.
    pub async fn get_address(
        &self,
        vault: VaultId,
        wallet: WalletId,
        address: AddressId,
    ) -> Result<WalletAddress> {
        let resp = self
            .client
            .call(|api| {
                api.wallets_get_wallet_address()
                    .vault_id(vault.as_str())
                    .wallet_id(wallet.as_str())
                    .address_id(address.as_str())
                    .send()
            })
            .await?;
        resp.wallet_address
            .map(WalletAddress::from)
            .ok_or_else(|| UtilaError::missing("wallet address"))
    }

    /// Generate a new address in a wallet for `network` (a `networks/{id}` resource name).
    pub fn create_address(
        &self,
        vault: VaultId,
        wallet: WalletId,
        network: NetworkId,
    ) -> CreateAddressBuilder<'a> {
        CreateAddressBuilder {
            client: self.client,
            vault,
            wallet,
            network,
            display_name: None,
            note: None,
        }
    }

    /// Retrieve multiple of a wallet's addresses by resource name.
    pub async fn batch_get_addresses(
        &self,
        vault: VaultId,
        wallet: WalletId,
        names: Vec<String>,
    ) -> Result<Vec<WalletAddress>> {
        let resp: V2BatchGetWalletAddressesResponse = self
            .client
            .call(|api| {
                api.wallets_batch_get_wallet_addresses()
                    .vault_id(vault.as_str())
                    .wallet_id(wallet.as_str())
                    .names(names)
                    .send()
            })
            .await?;
        Ok(resp
            .wallet_addresses
            .into_iter()
            .map(WalletAddress::from)
            .collect())
    }
}

enum PageState {
    First,
    Next(String),
    Done,
}

// ---- list wallets ----

pub struct ListWalletsBuilder<'a> {
    client: &'a UtilaClient,
    vault: VaultId,
    filter: Option<String>,
    show_archived: Option<bool>,
    page_size: Option<u32>,
    page_token: Option<String>,
}

impl<'a> ListWalletsBuilder<'a> {
    /// EBNF filter expression (see the API docs for supported fields/functions).
    pub fn filter(mut self, f: impl Into<String>) -> Self {
        self.filter = Some(f.into());
        self
    }
    /// Include archived wallets alongside non-archived ones.
    pub fn show_archived(mut self, v: bool) -> Self {
        self.show_archived = Some(v);
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

    pub async fn send(self) -> Result<WalletPage> {
        fetch_wallets(
            self.client,
            &self.vault,
            self.filter.as_deref(),
            self.show_archived,
            self.page_size,
            self.page_token.as_deref(),
        )
        .await
    }

    /// Stream every matching wallet across all pages.
    pub fn stream(self) -> impl Stream<Item = Result<Wallet>> + 'a {
        let Self {
            client,
            vault,
            filter,
            show_archived,
            page_size,
            ..
        } = self;
        stream::try_unfold(PageState::First, move |state| {
            let vault = vault.clone();
            let filter = filter.clone();
            async move {
                let token = match state {
                    PageState::First => None,
                    PageState::Next(t) => Some(t),
                    PageState::Done => return Ok::<_, UtilaError>(None),
                };
                let page = fetch_wallets(
                    client,
                    &vault,
                    filter.as_deref(),
                    show_archived,
                    page_size,
                    token.as_deref(),
                )
                .await?;
                let next = match page.next_page_token {
                    Some(t) => PageState::Next(t),
                    None => PageState::Done,
                };
                let items = stream::iter(page.wallets.into_iter().map(Ok::<Wallet, UtilaError>));
                Ok(Some((items, next)))
            }
        })
        .try_flatten()
    }
}

async fn fetch_wallets(
    client: &UtilaClient,
    vault: &VaultId,
    filter: Option<&str>,
    show_archived: Option<bool>,
    page_size: Option<u32>,
    page_token: Option<&str>,
) -> Result<WalletPage> {
    let resp: V2ListWalletsResponse = client
        .call(|api| {
            let mut b = api.wallets_list_wallets().vault_id(vault.as_str());
            if let Some(f) = filter {
                b = b.filter(f);
            }
            if let Some(v) = show_archived {
                b = b.show_archived(v);
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

// ---- list wallet addresses ----

pub struct ListAddressesBuilder<'a> {
    client: &'a UtilaClient,
    vault: VaultId,
    wallet: WalletId,
    page_size: Option<u32>,
    page_token: Option<String>,
}

impl<'a> ListAddressesBuilder<'a> {
    pub fn page_size(mut self, n: u32) -> Self {
        self.page_size = Some(n);
        self
    }
    pub fn page_token(mut self, t: impl Into<String>) -> Self {
        self.page_token = Some(t.into());
        self
    }

    pub async fn send(self) -> Result<WalletAddressPage> {
        fetch_addresses(
            self.client,
            &self.vault,
            &self.wallet,
            self.page_size,
            self.page_token.as_deref(),
        )
        .await
    }

    /// Stream every address of the wallet across all pages.
    pub fn stream(self) -> impl Stream<Item = Result<WalletAddress>> + 'a {
        let Self {
            client,
            vault,
            wallet,
            page_size,
            ..
        } = self;
        stream::try_unfold(PageState::First, move |state| {
            let vault = vault.clone();
            let wallet = wallet.clone();
            async move {
                let token = match state {
                    PageState::First => None,
                    PageState::Next(t) => Some(t),
                    PageState::Done => return Ok::<_, UtilaError>(None),
                };
                let page =
                    fetch_addresses(client, &vault, &wallet, page_size, token.as_deref()).await?;
                let next = match page.next_page_token {
                    Some(t) => PageState::Next(t),
                    None => PageState::Done,
                };
                let items = stream::iter(
                    page.addresses
                        .into_iter()
                        .map(Ok::<WalletAddress, UtilaError>),
                );
                Ok(Some((items, next)))
            }
        })
        .try_flatten()
    }
}

async fn fetch_addresses(
    client: &UtilaClient,
    vault: &VaultId,
    wallet: &WalletId,
    page_size: Option<u32>,
    page_token: Option<&str>,
) -> Result<WalletAddressPage> {
    let resp: V2ListWalletAddressesResponse = client
        .call(|api| {
            let mut b = api
                .wallets_list_wallet_addresses()
                .vault_id(vault.as_str())
                .wallet_id(wallet.as_str());
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

// ---- create wallet ----

pub struct CreateWalletBuilder<'a> {
    client: &'a UtilaClient,
    vault: VaultId,
    display_name: String,
    networks: Vec<NetworkId>,
    external: Option<bool>,
}

impl CreateWalletBuilder<'_> {
    /// Enable a network on the wallet at creation (pass the `networks/{id}` resource name).
    pub fn network(mut self, network: NetworkId) -> Self {
        self.networks.push(network);
        self
    }
    /// Mark the wallet as external (not managed by Utila).
    pub fn external(mut self, v: bool) -> Self {
        self.external = Some(v);
        self
    }

    pub async fn send(self) -> Result<Wallet> {
        let body = V2Wallet {
            archived: None,
            btc_details: None,
            converted_value: None,
            create_params: None,
            display_name: self.display_name,
            evm_details: None,
            external: self.external,
            has_frozen_assets: None,
            name: None,
            networks: self
                .networks
                .iter()
                .map(|n| n.as_str().to_string())
                .collect(),
            solana_details: None,
            ton_details: None,
            tron_details: None,
            xrpl_details: None,
        };
        let resp = self
            .client
            .call(|api| {
                api.wallets_create_wallet()
                    .vault_id(self.vault.as_str())
                    .body(body)
                    .send()
            })
            .await?;
        resp.wallet
            .map(Wallet::from)
            .ok_or_else(|| UtilaError::missing("wallet"))
    }
}

// ---- create wallet address ----

pub struct CreateAddressBuilder<'a> {
    client: &'a UtilaClient,
    vault: VaultId,
    wallet: WalletId,
    network: NetworkId,
    display_name: Option<String>,
    note: Option<String>,
}

impl CreateAddressBuilder<'_> {
    pub fn display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = Some(name.into());
        self
    }
    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    pub async fn send(self) -> Result<WalletAddress> {
        let body = V2WalletAddress {
            address: None,
            display_name: self.display_name,
            format: None,
            key: None,
            key_derivation_path: Vec::new(),
            name: None,
            network: self.network.as_str().to_string(),
            note: self.note,
            type_: None,
        };
        let resp = self
            .client
            .call(|api| {
                api.wallets_create_wallet_address()
                    .vault_id(self.vault.as_str())
                    .wallet_id(self.wallet.as_str())
                    .body(body)
                    .send()
            })
            .await?;
        resp.wallet_address
            .map(WalletAddress::from)
            .ok_or_else(|| UtilaError::missing("wallet address"))
    }
}

// ---- archive / unarchive (single) ----

pub struct ArchiveWalletBuilder<'a> {
    client: &'a UtilaClient,
    vault: VaultId,
    wallet: WalletId,
    allow_missing: Option<bool>,
    unarchive: bool,
}

impl ArchiveWalletBuilder<'_> {
    /// Don't fail if the wallet is missing.
    pub fn allow_missing(mut self, v: bool) -> Self {
        self.allow_missing = Some(v);
        self
    }

    pub async fn send(self) -> Result<()> {
        if self.unarchive {
            let body = WalletsUnarchiveWalletBody {
                allow_missing: self.allow_missing,
            };
            self.client
                .call(|api| {
                    api.wallets_unarchive_wallet()
                        .vault_id(self.vault.as_str())
                        .wallet_id(self.wallet.as_str())
                        .body(body)
                        .send()
                })
                .await?;
        } else {
            let body = WalletsArchiveWalletBody {
                allow_missing: self.allow_missing,
            };
            self.client
                .call(|api| {
                    api.wallets_archive_wallet()
                        .vault_id(self.vault.as_str())
                        .wallet_id(self.wallet.as_str())
                        .body(body)
                        .send()
                })
                .await?;
        }
        Ok(())
    }
}

// ---- batch archive / unarchive ----

pub struct BatchArchiveWalletsBuilder<'a> {
    client: &'a UtilaClient,
    vault: VaultId,
    names: Vec<String>,
    allow_missing: Option<bool>,
    unarchive: bool,
}

impl BatchArchiveWalletsBuilder<'_> {
    /// Don't fail if any of the wallets are missing.
    pub fn allow_missing(mut self, v: bool) -> Self {
        self.allow_missing = Some(v);
        self
    }

    pub async fn send(self) -> Result<()> {
        if self.unarchive {
            let body = WalletsBatchUnarchiveWalletsBody {
                allow_missing: self.allow_missing,
                names: self.names,
            };
            self.client
                .call(|api| {
                    api.wallets_batch_unarchive_wallets()
                        .vault_id(self.vault.as_str())
                        .body(body)
                        .send()
                })
                .await?;
        } else {
            let body = WalletsBatchArchiveWalletsBody {
                allow_missing: self.allow_missing,
                names: self.names,
            };
            self.client
                .call(|api| {
                    api.wallets_batch_archive_wallets()
                        .vault_id(self.vault.as_str())
                        .body(body)
                        .send()
                })
                .await?;
        }
        Ok(())
    }
}
