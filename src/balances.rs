//! The `balances()` group: models, plus querying balances/UTXOs with their assets
//! resolved from cached, batch-fetched metadata.
//!
//! Five operations are curated here:
//! * `query` — all balances in a vault (`balances_query_balances`).
//! * `query_wallet_balances` — balances in a wallet (`-` wildcard spans all wallets).
//! * `query_wallet_address_balances` — balances in a wallet address (`-` spans all addresses).
//! * `query_wallet_utxos` — the UTXOs of a wallet on a UTXO-model network.
//! * `refresh_asset_address_balance` — force-refresh one asset balance for an address.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use futures::stream::{self, Stream, TryStreamExt};
use rust_decimal::Decimal;

use crate::amount::Amount;
use crate::assets::{Asset, ResolvedAsset};
use crate::client::UtilaClient;
use crate::error::{ApiError, Result};
use crate::generated::types::{
    Apiv2Utxo, BalancesQueryWalletAddressBalancesBody, BalancesQueryWalletBalancesBody,
    BalancesQueryWalletUtxOsBody, BalancesRefreshAssetAddressBalanceBody, V2Balance,
    V2QueryBalancesResponse, V2QueryWalletAddressBalancesResponse, V2QueryWalletBalancesResponse,
    V2QueryWalletUtxOsResponse, V2RefreshAssetAddressBalanceResponse, V2UtxoState,
};
use crate::generated::ClientBalancesExt;
use crate::resource::{
    AddressId, AssetId, AssetRef, NetworkId, ParseRef, ResourceName, VaultId, WalletAddressRef,
    WalletId, WalletRef,
};

/// A vault balance for one asset. The exact base-unit `amount` is always present; a
/// human-readable value needs the asset's decimals (available when `asset` is resolved).
#[derive(Debug, Clone)]
pub struct Balance {
    pub asset: Asset,
    pub amount: Amount,
    pub frozen: Amount,
}

/// A balance scoped to a single wallet (the `wallet` resource name it belongs to).
#[derive(Debug, Clone)]
pub struct WalletBalance {
    pub wallet: ResourceName<WalletRef>,
    pub asset: Asset,
    pub amount: Amount,
    pub frozen: Amount,
}

/// A balance scoped to a single wallet address (the `wallet_address` resource name).
#[derive(Debug, Clone)]
pub struct WalletAddressBalance {
    pub wallet_address: ResourceName<WalletAddressRef>,
    pub asset: Asset,
    pub amount: Amount,
    pub frozen: Amount,
}

/// A single unspent transaction output on a UTXO-model network (e.g. Bitcoin). `value` is
/// the human-readable decimal amount (UTXOs are not asset-enriched — they carry a network,
/// not a tracked asset id).
#[derive(Debug, Clone)]
pub struct Utxo {
    pub wallet_address: ResourceName<WalletAddressRef>,
    pub network: Option<NetworkId>,
    pub tx_hash: Option<String>,
    pub vout: u32,
    /// Decimal value (precision included).
    pub value: Option<Decimal>,
    /// Spending state.
    pub state: Option<UtxoState>,
    /// The address's script type label, e.g. `BITCOIN_P2WPKH` (free-form; no fixed enum).
    pub script_type: Option<String>,
    pub confirmations: u32,
    pub create_time: Option<DateTime<Utc>>,
}

/// A UTXO's spending state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum UtxoState {
    Available,
    Locked,
    Frozen,
}

impl From<V2UtxoState> for UtxoState {
    fn from(s: V2UtxoState) -> Self {
        match s {
            V2UtxoState::Available => Self::Available,
            V2UtxoState::Locked => Self::Locked,
            V2UtxoState::Frozen => Self::Frozen,
        }
    }
}

impl From<Apiv2Utxo> for Utxo {
    fn from(u: Apiv2Utxo) -> Self {
        Self {
            wallet_address: ResourceName::parse(u.wallet_address.unwrap_or_default()),
            network: u.network.filter(|s| !s.is_empty()).map(NetworkId::from),
            tx_hash: u.tx_hash.filter(|s| !s.is_empty()),
            vout: u.vout.and_then(|v| u32::try_from(v).ok()).unwrap_or(0),
            value: u.value.and_then(|s| s.parse::<Decimal>().ok()),
            state: u.state.map(UtxoState::from),
            script_type: u.script_type.filter(|s| !s.is_empty()),
            confirmations: u
                .confirmations
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(0),
            create_time: u.create_time,
        }
    }
}

/// One page of wallet balances.
#[derive(Debug, Clone)]
pub struct WalletBalancePage {
    pub balances: Vec<WalletBalance>,
    pub next_page_token: Option<String>,
    pub total_size: i32,
}

/// One page of wallet-address balances.
#[derive(Debug, Clone)]
pub struct WalletAddressBalancePage {
    pub balances: Vec<WalletAddressBalance>,
    pub next_page_token: Option<String>,
    pub total_size: i32,
}

/// One page of UTXOs.
#[derive(Debug, Clone)]
pub struct UtxoPage {
    pub utxos: Vec<Utxo>,
    pub next_page_token: Option<String>,
    pub total_size: i32,
}

impl From<V2QueryWalletUtxOsResponse> for UtxoPage {
    fn from(r: V2QueryWalletUtxOsResponse) -> Self {
        Self {
            utxos: r.utxos.into_iter().map(Utxo::from).collect(),
            next_page_token: r.next_page_token.filter(|t| !t.is_empty()),
            total_size: r.total_size.unwrap_or(0),
        }
    }
}

pub struct Balances<'a> {
    pub(crate) client: &'a UtilaClient,
}

impl<'a> Balances<'a> {
    /// Query balances for a vault. Each balance's `asset` is resolved to its
    /// decimals/symbol via one batched, cached lookup; assets that can't be resolved come
    /// back as `Asset::Unresolved` (the exact base-unit `amount` is always present).
    pub async fn query(&self, vault: VaultId) -> Result<Vec<Balance>> {
        let resp: V2QueryBalancesResponse = self
            .client
            .call(|api| {
                api.balances_query_balances()
                    .vault_id(vault.as_str())
                    .send()
            })
            .await?;

        let names: Vec<String> = resp
            .balances
            .iter()
            .filter_map(|b| b.asset.clone())
            .collect();
        let metas = self.client.asset_cache().resolve(self.client, &names).await;
        resp.balances
            .into_iter()
            .map(|b| balance_from(b, &metas))
            .collect()
    }

    /// Query the balances of a wallet. Pass `WalletId::new("-")` to span every wallet in
    /// the vault ([AIP-159] wildcard). The asset of each balance is resolved like
    /// [`query`](Self::query).
    ///
    /// [AIP-159]: https://google.aip.dev/159
    pub fn query_wallet_balances(
        &self,
        vault: VaultId,
        wallet: WalletId,
    ) -> QueryWalletBalancesBuilder<'a> {
        QueryWalletBalancesBuilder {
            client: self.client,
            vault,
            wallet,
            filter: None,
            page_size: None,
            page_token: None,
        }
    }

    /// Query the balances of a wallet address. Pass `AddressId::new("-")` to span every
    /// address of the wallet ([AIP-159] wildcard).
    ///
    /// [AIP-159]: https://google.aip.dev/159
    pub fn query_wallet_address_balances(
        &self,
        vault: VaultId,
        wallet: WalletId,
        address: AddressId,
    ) -> QueryWalletAddressBalancesBuilder<'a> {
        QueryWalletAddressBalancesBuilder {
            client: self.client,
            vault,
            wallet,
            address,
            filter: None,
            page_size: None,
            page_token: None,
        }
    }

    /// Query the UTXOs of a wallet on the given (UTXO-model) network.
    pub fn query_wallet_utxos(
        &self,
        vault: VaultId,
        wallet: WalletId,
        network: NetworkId,
    ) -> QueryUtxosBuilder<'a> {
        QueryUtxosBuilder {
            client: self.client,
            vault,
            wallet,
            network,
            filter: None,
            order_by: None,
            page_size: None,
            page_token: None,
            skip: None,
        }
    }

    /// Force a refresh of one asset's balance for a specific address, returning the
    /// freshly-read [`Balance`] (asset resolved like [`query`](Self::query)).
    pub async fn refresh_asset_address_balance(
        &self,
        vault: VaultId,
        asset: AssetId,
        address: impl Into<String>,
    ) -> Result<Balance> {
        let body = BalancesRefreshAssetAddressBalanceBody {
            address: address.into(),
            asset: AssetRef::resource_name(&asset),
            include_referenced_resources: None,
        };
        let resp: V2RefreshAssetAddressBalanceResponse = self
            .client
            .call(|api| {
                api.balances_refresh_asset_address_balance()
                    .vault_id(vault.as_str())
                    .body(body)
                    .send()
            })
            .await?;
        let bal = resp.balance.ok_or_else(|| ApiError::missing("balance"))?;
        let names: Vec<String> = bal.asset.clone().into_iter().collect();
        let metas = self.client.asset_cache().resolve(self.client, &names).await;
        balance_from(bal, &metas)
    }
}

pub struct QueryWalletBalancesBuilder<'a> {
    client: &'a UtilaClient,
    vault: VaultId,
    wallet: WalletId,
    filter: Option<String>,
    page_size: Option<u32>,
    page_token: Option<String>,
}

impl<'a> QueryWalletBalancesBuilder<'a> {
    /// EBNF filter (e.g. `asset("assets/native.ethereum-mainnet")`).
    pub fn filter(mut self, f: impl Into<String>) -> Self {
        self.filter = Some(f.into());
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

    pub async fn send(self) -> Result<WalletBalancePage> {
        fetch_wallet_balances(
            self.client,
            &self.vault,
            &self.wallet,
            self.filter.as_deref(),
            self.page_size,
            self.page_token.as_deref(),
        )
        .await
    }

    /// Stream every wallet balance across all pages (the `filter` is preserved).
    pub fn stream(self) -> impl Stream<Item = Result<WalletBalance>> + 'a {
        wallet_balance_stream(self.client, self.vault, self.wallet, self.filter)
    }
}

pub struct QueryWalletAddressBalancesBuilder<'a> {
    client: &'a UtilaClient,
    vault: VaultId,
    wallet: WalletId,
    address: AddressId,
    filter: Option<String>,
    page_size: Option<u32>,
    page_token: Option<String>,
}

impl<'a> QueryWalletAddressBalancesBuilder<'a> {
    /// EBNF filter (e.g. `network("networks/ethereum-mainnet")`).
    pub fn filter(mut self, f: impl Into<String>) -> Self {
        self.filter = Some(f.into());
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

    pub async fn send(self) -> Result<WalletAddressBalancePage> {
        fetch_wallet_address_balances(
            self.client,
            &self.vault,
            &self.wallet,
            &self.address,
            self.filter.as_deref(),
            self.page_size,
            self.page_token.as_deref(),
        )
        .await
    }

    /// Stream every wallet-address balance across all pages (the `filter` is preserved).
    pub fn stream(self) -> impl Stream<Item = Result<WalletAddressBalance>> + 'a {
        wallet_address_balance_stream(
            self.client,
            self.vault,
            self.wallet,
            self.address,
            self.filter,
        )
    }
}

pub struct QueryUtxosBuilder<'a> {
    client: &'a UtilaClient,
    vault: VaultId,
    wallet: WalletId,
    network: NetworkId,
    filter: Option<String>,
    order_by: Option<String>,
    page_size: Option<u32>,
    page_token: Option<String>,
    skip: Option<u32>,
}

impl<'a> QueryUtxosBuilder<'a> {
    /// EBNF filter over `txHash`, `address`, or `state`.
    pub fn filter(mut self, f: impl Into<String>) -> Self {
        self.filter = Some(f.into());
        self
    }
    /// SQL-like ordering over `create_time` / `value`.
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
    /// How many results to skip (may be combined with token pagination).
    pub fn skip(mut self, n: u32) -> Self {
        self.skip = Some(n);
        self
    }

    pub async fn send(self) -> Result<UtxoPage> {
        fetch_utxos(
            self.client,
            &self.vault,
            &self.wallet,
            &self.network,
            self.filter.as_deref(),
            self.order_by.as_deref(),
            self.page_size,
            self.page_token.as_deref(),
            self.skip,
        )
        .await
    }

    /// Stream every UTXO across all pages (the `filter`/`order_by` are preserved).
    pub fn stream(self) -> impl Stream<Item = Result<Utxo>> + 'a {
        utxo_stream(
            self.client,
            self.vault,
            self.wallet,
            self.network,
            self.filter,
            self.order_by,
        )
    }
}

enum PageState {
    First,
    Next(String),
    Done,
}

fn wallet_balance_stream(
    client: &UtilaClient,
    vault: VaultId,
    wallet: WalletId,
    filter: Option<String>,
) -> impl Stream<Item = Result<WalletBalance>> + '_ {
    stream::try_unfold(PageState::First, move |state| {
        let vault = vault.clone();
        let wallet = wallet.clone();
        let filter = filter.clone();
        async move {
            let token = match state {
                PageState::First => None,
                PageState::Next(t) => Some(t),
                PageState::Done => return Ok::<_, ApiError>(None),
            };
            let page = fetch_wallet_balances(
                client,
                &vault,
                &wallet,
                filter.as_deref(),
                None,
                token.as_deref(),
            )
            .await?;
            let next = next_state(page.next_page_token);
            let items = stream::iter(page.balances.into_iter().map(Ok::<WalletBalance, ApiError>));
            Ok(Some((items, next)))
        }
    })
    .try_flatten()
}

fn wallet_address_balance_stream(
    client: &UtilaClient,
    vault: VaultId,
    wallet: WalletId,
    address: AddressId,
    filter: Option<String>,
) -> impl Stream<Item = Result<WalletAddressBalance>> + '_ {
    stream::try_unfold(PageState::First, move |state| {
        let vault = vault.clone();
        let wallet = wallet.clone();
        let address = address.clone();
        let filter = filter.clone();
        async move {
            let token = match state {
                PageState::First => None,
                PageState::Next(t) => Some(t),
                PageState::Done => return Ok::<_, ApiError>(None),
            };
            let page = fetch_wallet_address_balances(
                client,
                &vault,
                &wallet,
                &address,
                filter.as_deref(),
                None,
                token.as_deref(),
            )
            .await?;
            let next = next_state(page.next_page_token);
            let items = stream::iter(
                page.balances
                    .into_iter()
                    .map(Ok::<WalletAddressBalance, ApiError>),
            );
            Ok(Some((items, next)))
        }
    })
    .try_flatten()
}

fn utxo_stream(
    client: &UtilaClient,
    vault: VaultId,
    wallet: WalletId,
    network: NetworkId,
    filter: Option<String>,
    order_by: Option<String>,
) -> impl Stream<Item = Result<Utxo>> + '_ {
    stream::try_unfold(PageState::First, move |state| {
        let vault = vault.clone();
        let wallet = wallet.clone();
        let network = network.clone();
        let filter = filter.clone();
        let order_by = order_by.clone();
        async move {
            let token = match state {
                PageState::First => None,
                PageState::Next(t) => Some(t),
                PageState::Done => return Ok::<_, ApiError>(None),
            };
            let page = fetch_utxos(
                client,
                &vault,
                &wallet,
                &network,
                filter.as_deref(),
                order_by.as_deref(),
                None,
                token.as_deref(),
                None,
            )
            .await?;
            let next = next_state(page.next_page_token);
            let items = stream::iter(page.utxos.into_iter().map(Ok::<Utxo, ApiError>));
            Ok(Some((items, next)))
        }
    })
    .try_flatten()
}

fn next_state(token: Option<String>) -> PageState {
    match token {
        Some(t) => PageState::Next(t),
        None => PageState::Done,
    }
}

async fn fetch_wallet_balances(
    client: &UtilaClient,
    vault: &VaultId,
    wallet: &WalletId,
    filter: Option<&str>,
    page_size: Option<u32>,
    page_token: Option<&str>,
) -> Result<WalletBalancePage> {
    let body = BalancesQueryWalletBalancesBody {
        filter: filter.map(str::to_owned),
        include_referenced_resources: None,
        page_size: page_size.and_then(|n| i32::try_from(n).ok()),
        page_token: page_token.map(str::to_owned),
    };
    let resp: V2QueryWalletBalancesResponse = client
        .call(|api| {
            api.balances_query_wallet_balances()
                .vault_id(vault.as_str())
                .wallet_id(wallet.as_str())
                .body(body)
                .send()
        })
        .await?;

    let names: Vec<String> = resp
        .wallet_balances
        .iter()
        .filter_map(|b| b.asset.clone())
        .collect();
    let metas = client.asset_cache().resolve(client, &names).await;
    let balances = resp
        .wallet_balances
        .into_iter()
        .map(|b| {
            Ok(WalletBalance {
                wallet: ResourceName::parse(b.wallet.unwrap_or_default()),
                asset: resolve_asset(&metas, &b.asset.unwrap_or_default()),
                amount: amount_from(b.raw_value)?,
                frozen: amount_from(b.frozen_value)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(WalletBalancePage {
        balances,
        next_page_token: resp.next_page_token.filter(|t| !t.is_empty()),
        total_size: resp.total_size.unwrap_or(0),
    })
}

async fn fetch_wallet_address_balances(
    client: &UtilaClient,
    vault: &VaultId,
    wallet: &WalletId,
    address: &AddressId,
    filter: Option<&str>,
    page_size: Option<u32>,
    page_token: Option<&str>,
) -> Result<WalletAddressBalancePage> {
    let body = BalancesQueryWalletAddressBalancesBody {
        filter: filter.map(str::to_owned),
        include_referenced_resources: None,
        page_size: page_size.and_then(|n| i32::try_from(n).ok()),
        page_token: page_token.map(str::to_owned),
    };
    let resp: V2QueryWalletAddressBalancesResponse = client
        .call(|api| {
            api.balances_query_wallet_address_balances()
                .vault_id(vault.as_str())
                .wallet_id(wallet.as_str())
                .address_id(address.as_str())
                .body(body)
                .send()
        })
        .await?;

    let names: Vec<String> = resp
        .wallet_address_balances
        .iter()
        .filter_map(|b| b.asset.clone())
        .collect();
    let metas = client.asset_cache().resolve(client, &names).await;
    let balances = resp
        .wallet_address_balances
        .into_iter()
        .map(|b| {
            Ok(WalletAddressBalance {
                wallet_address: ResourceName::parse(b.wallet_address.unwrap_or_default()),
                asset: resolve_asset(&metas, &b.asset.unwrap_or_default()),
                amount: amount_from(b.raw_value)?,
                frozen: amount_from(b.frozen_value)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(WalletAddressBalancePage {
        balances,
        next_page_token: resp.next_page_token.filter(|t| !t.is_empty()),
        total_size: resp.total_size.unwrap_or(0),
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "mirrors the generated queryUTXOs request: required path ids + all optional modifiers"
)]
async fn fetch_utxos(
    client: &UtilaClient,
    vault: &VaultId,
    wallet: &WalletId,
    network: &NetworkId,
    filter: Option<&str>,
    order_by: Option<&str>,
    page_size: Option<u32>,
    page_token: Option<&str>,
    skip: Option<u32>,
) -> Result<UtxoPage> {
    let body = BalancesQueryWalletUtxOsBody {
        filter: filter.map(str::to_owned),
        include_referenced_resources: None,
        network: network.as_str().to_string(),
        order_by: order_by.map(str::to_owned),
        page_size: page_size.and_then(|n| i32::try_from(n).ok()),
        page_token: page_token.map(str::to_owned),
        skip: skip.and_then(|n| i32::try_from(n).ok()),
    };
    let resp: V2QueryWalletUtxOsResponse = client
        .call(|api| {
            api.balances_query_wallet_utx_os()
                .vault_id(vault.as_str())
                .wallet_id(wallet.as_str())
                .body(body)
                .send()
        })
        .await?;
    Ok(resp.into())
}

/// Build a curated [`Balance`] from a generated one, resolving its asset against `metas`.
fn balance_from(b: V2Balance, metas: &HashMap<String, ResolvedAsset>) -> Result<Balance> {
    Ok(Balance {
        asset: resolve_asset(metas, &b.asset.unwrap_or_default()),
        amount: amount_from(b.raw_value)?,
        frozen: amount_from(b.frozen_value)?,
    })
}

/// Resolve an asset id against the batch-fetched metadata, degrading to `Unresolved`.
fn resolve_asset(metas: &HashMap<String, ResolvedAsset>, asset_id: &str) -> Asset {
    match metas.get(asset_id) {
        Some(resolved) => Asset::Resolved(resolved.clone()),
        None => Asset::Unresolved(
            AssetRef::parse(asset_id).map_or_else(|| AssetId::new(asset_id), AssetId::from),
        ),
    }
}

/// Parse a protojson base-unit field (`Option<String>`) into an exact [`Amount`], treating
/// absent/empty as zero (the API omits zero-valued fields).
fn amount_from(value: Option<String>) -> Result<Amount> {
    match value.filter(|s| !s.is_empty()) {
        // A malformed amount in an API response is a response-decode failure (client-side,
        // synthetic code -1) — surface it through the API-result error type.
        Some(s) => Amount::parse(&s).map_err(|e| ApiError::Api {
            code: -1,
            message: e.to_string(),
            details: Vec::new(),
        }),
        None => Ok(Amount::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utxo_state_maps_every_variant() {
        assert_eq!(
            UtxoState::from(V2UtxoState::Available),
            UtxoState::Available
        );
        assert_eq!(UtxoState::from(V2UtxoState::Locked), UtxoState::Locked);
        assert_eq!(UtxoState::from(V2UtxoState::Frozen), UtxoState::Frozen);
    }

    #[test]
    fn amount_from_handles_absent_empty_valid_and_bad() {
        assert_eq!(amount_from(None).unwrap(), Amount::default());
        // protojson omits zero-valued fields ⇒ empty string is zero
        assert_eq!(amount_from(Some(String::new())).unwrap(), Amount::default());
        assert_eq!(
            amount_from(Some("42".into())).unwrap(),
            Amount::from_base_units(42)
        );
        // A malformed amount surfaces as a synthetic API (response-decode) error.
        assert!(matches!(
            amount_from(Some("nope".into())),
            Err(ApiError::Api { code: -1, .. })
        ));
    }
}
