//! Process-lifetime cache resolving asset ids to their decimals/symbol.
//!
//! Used by the `balances` slice to enrich balances without re-fetching: decimals/symbol are
//! immutable per asset, so a resolved entry never expires within a process. Distinct from the
//! public [`assets()`](crate::Assets) group (which is a thin facade over the API) — this is an
//! internal, shared, mutex-guarded store keyed by asset resource name.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::assets::ResolvedAsset;
use crate::client::UtilaClient;
use crate::generated::ClientAssetsExt;

/// A cache of resolved assets (decimals/symbol are immutable per asset, so it never expires
/// within a process).
pub(crate) struct AssetCache {
    cache: Mutex<HashMap<String, ResolvedAsset>>,
}

impl AssetCache {
    pub(crate) fn new() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Resolve assets by resource name, using the cache and a single batched fetch for
    /// misses. Unresolved assets are simply absent from the result (degrade gracefully).
    pub(crate) async fn resolve(
        &self,
        client: &UtilaClient,
        names: &[String],
    ) -> HashMap<String, ResolvedAsset> {
        let mut out = HashMap::new();
        let mut missing: Vec<&str> = Vec::new();
        {
            let cache = self.cache.lock().expect("asset cache mutex poisoned");
            for n in names {
                if n.is_empty() {
                    continue;
                }
                match cache.get(n) {
                    Some(r) => {
                        out.insert(n.clone(), r.clone());
                    }
                    None => missing.push(n.as_str()),
                }
            }
        }
        if missing.is_empty() {
            return out;
        }
        missing.sort_unstable();
        missing.dedup();

        let names: Vec<String> = missing
            .into_iter()
            .map(std::string::ToString::to_string)
            .collect();
        let result = client
            .call(|api| api.assets_batch_get_assets().names(names).send())
            .await;
        if let Ok(resp) = result {
            let mut cache = self.cache.lock().expect("asset cache mutex poisoned");
            for asset in resp.assets {
                let Some(name) = asset.name.clone() else {
                    continue;
                };
                if name.is_empty() {
                    continue;
                }
                let resolved = ResolvedAsset::from(asset);
                cache.insert(name.clone(), resolved.clone());
                out.insert(name, resolved);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn resolve_skips_empty_and_nameless_assets() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/assets:batchGet"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "assets": [
                    { "name": "assets/eth", "symbol": "ETH", "decimals": 18 },
                    { "symbol": "NoName" },
                    { "name": "", "symbol": "Empty" }
                ]
            })))
            .mount(&server)
            .await;
        let signer =
            crate::auth::SignerSource::local_pem(include_bytes!("../tests/test_key.pem")).unwrap();
        let client = crate::client::UtilaClient::builder()
            .credential("a", signer)
            .base_url(server.uri())
            .build()
            .unwrap();
        let cache = AssetCache::new();
        // Empty input name skipped; no-name + empty-name response assets skipped.
        let out = cache
            .resolve(&client, &[String::new(), "assets/eth".to_string()])
            .await;
        assert_eq!(out.len(), 1);
        assert!(out.contains_key("assets/eth"));
    }

    #[tokio::test]
    async fn resolve_degrades_when_batch_fetch_fails() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/assets:batchGet"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let signer =
            crate::auth::SignerSource::local_pem(include_bytes!("../tests/test_key.pem")).unwrap();
        let client = crate::client::UtilaClient::builder()
            .credential("a", signer)
            .base_url(server.uri())
            .build()
            .unwrap();
        let cache = AssetCache::new();
        let out = cache.resolve(&client, &["assets/eth".to_string()]).await;
        assert!(out.is_empty());
    }
}
