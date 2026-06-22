//! Assets: the resolved/unresolved asset model and the public `assets()` group over
//! `ClientAssetsExt` (get / batch-get / vault-scoped get). The process-lifetime resolution
//! cache used by the balances slice lives separately in [`crate::asset_cache`].

use crate::client::UtilaClient;
use crate::error::{ApiError, Result};
use crate::generated::types::V2Asset;
use crate::generated::ClientAssetsExt;
use crate::resource::{AssetId, AssetRef, ParseRef, VaultId};

/// An asset reference — either resolved with metadata, or just its id if it couldn't be
/// looked up. Match on it to get decimals/symbol with no `Option`.
#[derive(Debug, Clone)]
pub enum Asset {
    Resolved(ResolvedAsset),
    Unresolved(AssetId),
}

/// A resolved asset: its id plus the metadata needed to interpret base-unit amounts.
///
/// This is the curated projection of the API's `V2Asset`. It keeps the fields needed to
/// interpret amounts (`decimals`) and label them (`symbol`); chain-specific token details
/// (contract address, Canton/Sui token details, converted USD value, token standard) are
/// intentionally omitted — they are not needed by the consumers of this crate and would
/// pull in a large union of exotic per-chain variants.
#[derive(Debug, Clone)]
pub struct ResolvedAsset {
    pub id: AssetId,
    pub decimals: u32,
    pub symbol: String,
}

impl From<V2Asset> for ResolvedAsset {
    fn from(a: V2Asset) -> Self {
        Self {
            id: a
                .name
                .filter(|s| !s.is_empty())
                .and_then(|n| AssetRef::parse(&n).map(AssetId::from))
                .unwrap_or(AssetId::EMPTY),
            decimals: a.decimals.and_then(|d| u32::try_from(d).ok()).unwrap_or(0),
            symbol: a.symbol.unwrap_or_default(),
        }
    }
}

impl Asset {
    /// The asset id — known either way.
    pub fn id(&self) -> &AssetId {
        match self {
            Asset::Resolved(a) => &a.id,
            Asset::Unresolved(id) => id,
        }
    }
    /// Decimals, if the asset was resolved.
    pub fn decimals(&self) -> Option<u32> {
        match self {
            Asset::Resolved(a) => Some(a.decimals),
            Asset::Unresolved(_) => None,
        }
    }
    /// Symbol, if the asset was resolved.
    pub fn symbol(&self) -> Option<&str> {
        match self {
            Asset::Resolved(a) => Some(&a.symbol),
            Asset::Unresolved(_) => None,
        }
    }
}

/// The `assets()` group: look up asset metadata by resource name. Each method returns the
/// curated [`ResolvedAsset`]; missing optional fields collapse to defaults.
pub struct Assets<'a> {
    pub(crate) client: &'a UtilaClient,
}

impl Assets<'_> {
    /// Get one asset by resource name (e.g. `assets/native.ethereum-mainnet`).
    pub async fn get(&self, id: AssetId) -> Result<ResolvedAsset> {
        let segment = id.as_str().to_string();
        let resp = self
            .client
            .call(|api| api.assets_get_asset().asset_id(segment).send())
            .await?;
        resp.asset
            .map(ResolvedAsset::from)
            .ok_or_else(|| ApiError::missing("asset"))
    }

    /// Get a vault-scoped asset (an imported token or custom-chain token) by resource name.
    pub async fn get_for_vault(&self, vault: VaultId, asset: AssetId) -> Result<ResolvedAsset> {
        let segment = asset.as_str().to_string();
        let resp = self
            .client
            .call(|api| {
                api.assets_get_vault_asset()
                    .vault_id(vault.as_str())
                    .asset_id(segment)
                    .send()
            })
            .await?;
        resp.asset
            .map(ResolvedAsset::from)
            .ok_or_else(|| ApiError::missing("asset"))
    }

    /// Get several assets at once by resource name. Assets the server doesn't return are
    /// simply absent from the result.
    pub async fn batch_get(
        &self,
        names: impl IntoIterator<Item = AssetId>,
    ) -> Result<Vec<ResolvedAsset>> {
        let names: Vec<String> = names
            .into_iter()
            .map(|id| AssetRef::resource_name(&id))
            .collect();
        let resp = self
            .client
            .call(|api| api.assets_batch_get_assets().names(names).send())
            .await?;
        Ok(resp.assets.into_iter().map(ResolvedAsset::from).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_accessors_resolved_and_unresolved() {
        let r = Asset::Resolved(ResolvedAsset {
            id: AssetId::from("assets/x"),
            decimals: 18,
            symbol: "ETH".into(),
        });
        assert_eq!(r.id().as_str(), "assets/x");
        assert_eq!(r.decimals(), Some(18));
        assert_eq!(r.symbol(), Some("ETH"));

        let u = Asset::Unresolved(AssetId::from("assets/y"));
        assert_eq!(u.id().as_str(), "assets/y");
        assert_eq!(u.decimals(), None);
        assert_eq!(u.symbol(), None);
    }

    #[test]
    fn resolved_asset_from_v2_collapses_missing_fields() {
        let a = V2Asset {
            name: Some("assets/native.ethereum-mainnet".into()),
            decimals: Some(18),
            symbol: Some("ETH".into()),
            ..Default::default()
        };
        let r = ResolvedAsset::from(a);
        assert_eq!(r.id.as_str(), "native.ethereum-mainnet");
        assert_eq!(r.decimals, 18);
        assert_eq!(r.symbol, "ETH");

        // Missing / negative decimals collapse to 0; missing name/symbol to empty.
        let bare = ResolvedAsset::from(V2Asset {
            decimals: Some(-1),
            ..Default::default()
        });
        assert_eq!(bare.id.as_str(), "");
        assert_eq!(bare.decimals, 0);
        assert_eq!(bare.symbol, "");
    }
}
