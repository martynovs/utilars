use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;

use crate::asset_cache::AssetCache;
use crate::auth::{SignerSource, TokenManager};
use crate::balances::Balances;
use crate::error::{ApiError, Result};
use crate::generated::{Client as ApiClient, Error as GenError, ResponseValue};
use crate::transactions::Transactions;
use crate::vaults::Vaults;

const DEFAULT_BASE_URL: &str = "https://api.utila.io";

/// A client for the Utila v2 API: a grouped facade over the progenitor-generated
/// transport. The generated `Client` carries the [`TokenManager`] and authenticates
/// every request via the async pre-hook; the facade adds typed inputs/outputs,
/// pagination, and asset enrichment. Retry is not built in — it is applied externally by
/// the caller (gate a retry crate on [`ApiError::is_retryable`]).
pub struct UtilaClient {
    api: ApiClient,
    assets: AssetCache,
}

impl UtilaClient {
    pub fn builder() -> UtilaClientBuilder {
        UtilaClientBuilder::default()
    }

    // ---- groups ----
    pub fn vaults(&self) -> Vaults<'_> {
        Vaults { client: self }
    }
    pub fn balances(&self) -> Balances<'_> {
        Balances { client: self }
    }
    pub fn transactions(&self) -> Transactions<'_> {
        Transactions { client: self }
    }
    pub fn networks(&self) -> crate::networks::Networks<'_> {
        crate::networks::Networks { client: self }
    }
    pub fn wallets(&self) -> crate::wallets::Wallets<'_> {
        crate::wallets::Wallets { client: self }
    }
    pub fn address_book(&self) -> crate::address_book::AddressBook<'_> {
        crate::address_book::AddressBook { client: self }
    }
    pub fn assets(&self) -> crate::assets::Assets<'_> {
        crate::assets::Assets { client: self }
    }

    /// Run a generated operation and map its result to the success body or a typed
    /// [`ApiError`]. `op` receives the authenticated transport and returns the
    /// (un-awaited) `send()` future; `call` awaits it — boxed, since the generated future
    /// is large (~17 KB) — so call sites read `client.call(|api| api.foo().send()).await`:
    /// one await, no reach for the transport. Each call is exactly one request; retry wraps
    /// the whole call externally (gate on [`ApiError::is_retryable`]).
    pub(crate) async fn call<'s, T, F, Fut>(&'s self, op: F) -> Result<T>
    where
        F: FnOnce(&'s ApiClient) -> Fut,
        Fut: std::future::Future<Output = std::result::Result<ResponseValue<T>, GenError>> + 's,
    {
        match Box::pin(op(&self.api)).await {
            Ok(resp) => Ok(resp.into_inner()),
            Err(e) => Err(map_error(e).await),
        }
    }

    /// The shared asset cache (resolution logic lives in the `assets` module). Named
    /// `asset_cache` so the public `assets()` accessor can return the `Assets` group.
    pub(crate) fn asset_cache(&self) -> &AssetCache {
        &self.assets
    }
}

/// Render a generated string-enum (e.g. a status/state) to its serialized label without
/// depending on the enum's `Display` impl. Returns `None` for non-string serializations.
pub(crate) fn enum_label<T: serde::Serialize>(value: &T) -> Option<String> {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => Some(s),
        _ => None,
    }
}

/// Map a generated transport error into a typed [`ApiError`]. Non-success responses
/// (the spec declares only 200, so all of them surface as `UnexpectedResponse`) get the
/// gRPC status envelope parsed out of the body; auth pre-hook failures arrive as `Custom`.
async fn map_error(e: GenError) -> ApiError {
    match e {
        GenError::UnexpectedResponse(resp) => parse_api_error(resp).await,
        GenError::Custom(msg) => ApiError::Auth(msg),
        GenError::InvalidRequest(msg) => ApiError::Config(msg),
        GenError::CommunicationError(e)
        | GenError::InvalidUpgrade(e)
        | GenError::ResponseBodyError(e) => ApiError::Http(e),
        GenError::InvalidResponsePayload(_, e) => ApiError::Api {
            code: -1,
            message: format!("invalid response payload: {e}"),
            details: Vec::new(),
        },
        GenError::ErrorResponse(rv) => ApiError::Api {
            code: i32::from(rv.status().as_u16()),
            message: "unexpected error response".into(),
            details: Vec::new(),
        },
    }
}

async fn parse_api_error(resp: reqwest::Response) -> ApiError {
    #[derive(Deserialize, Default)]
    struct GrpcStatus {
        #[serde(default)]
        code: i32,
        #[serde(default)]
        message: String,
        /// `google.rpc.Status.details` — each entry a `google.protobuf.Any` JSON object,
        /// surfaced verbatim so callers can inspect typed error details.
        #[serde(default)]
        details: Vec<serde_json::Value>,
    }
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    match serde_json::from_str::<GrpcStatus>(&body) {
        Ok(s) => ApiError::Api {
            code: s.code,
            message: s.message,
            details: s.details,
        },
        // Body wasn't a gRPC status envelope: fall back to the HTTP status + raw body.
        Err(_) => ApiError::Api {
            code: i32::from(status.as_u16()),
            message: body,
            details: Vec::new(),
        },
    }
}

/// Builder for [`UtilaClient`].
#[derive(Default)]
pub struct UtilaClientBuilder {
    credential: Option<(String, SignerSource)>,
    base_url: Option<String>,
    timeout: Option<Duration>,
    connect_timeout: Option<Duration>,
}

impl UtilaClientBuilder {
    /// Service account email (the JWT `sub`) and an already-validated signer.
    pub fn credential(mut self, account: impl Into<String>, signer: SignerSource) -> Self {
        self.credential = Some((account.into(), signer));
        self
    }

    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    /// Overall per-request timeout (connect + send + receive). Unset by default.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Timeout for only the connect phase. Unset by default.
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = Some(timeout);
        self
    }

    pub fn build(self) -> Result<UtilaClient> {
        let (account, signer) = self
            .credential
            .ok_or_else(|| ApiError::Config("credential is required".into()))?;
        let tokens = Arc::new(TokenManager::new(account, signer));
        let base_url = self
            .base_url
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());

        let mut http = reqwest::Client::builder();
        if let Some(t) = self.timeout {
            http = http.timeout(t);
        }
        if let Some(t) = self.connect_timeout {
            http = http.connect_timeout(t);
        }
        let http = http.build()?;

        Ok(UtilaClient {
            api: ApiClient::new_with_client(&base_url, http, tokens),
            assets: AssetCache::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_KEY: &[u8] = include_bytes!("../tests/test_key.pem");

    #[tokio::test]
    async fn map_error_maps_transport_variants() {
        use crate::generated::Error as E;

        assert!(matches!(
            map_error(E::Custom("boom".into())).await,
            ApiError::Auth(m) if m == "boom"
        ));
        assert!(matches!(
            map_error(E::InvalidRequest("bad".into())).await,
            ApiError::Config(m) if m == "bad"
        ));
        let serde_err = serde_json::from_str::<i32>("\"nope\"").unwrap_err();
        assert!(matches!(
            map_error(E::InvalidResponsePayload(bytes::Bytes::new(), serde_err)).await,
            ApiError::Api { code: -1, .. }
        ));
        // A real (connection-refused) reqwest::Error exercises the transport arm; port 1
        // on loopback refuses immediately, so this stays deterministic and offline.
        let http_err = reqwest::Client::new()
            .get("http://127.0.0.1:1")
            .send()
            .await
            .unwrap_err();
        assert!(matches!(
            map_error(E::CommunicationError(http_err)).await,
            ApiError::Http(_)
        ));
        // The other two reqwest-wrapping variants share the same arm.
        let e2 = reqwest::Client::new()
            .get("http://127.0.0.1:1")
            .send()
            .await
            .unwrap_err();
        assert!(matches!(
            map_error(E::InvalidUpgrade(e2)).await,
            ApiError::Http(_)
        ));
        let e3 = reqwest::Client::new()
            .get("http://127.0.0.1:1")
            .send()
            .await
            .unwrap_err();
        assert!(matches!(
            map_error(E::ResponseBodyError(e3)).await,
            ApiError::Http(_)
        ));
        // `ErrorResponse` only arises for documented non-200s (the spec has none), but the
        // arm must still map; `ResponseValue::new` lets us exercise it directly.
        let rv = ResponseValue::new(
            (),
            reqwest::StatusCode::BAD_GATEWAY,
            reqwest::header::HeaderMap::new(),
        );
        assert!(matches!(
            map_error(E::ErrorResponse(rv)).await,
            ApiError::Api { code: 502, .. }
        ));
    }

    #[test]
    fn builder_builds_with_local_pem_and_timeouts() {
        let signer = SignerSource::local_pem(TEST_KEY).unwrap();
        UtilaClient::builder()
            .credential("a", signer)
            .base_url("http://localhost")
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .unwrap();
    }

    #[test]
    fn enum_label_returns_none_for_non_string() {
        // status/state enums serialize to strings → Some; anything else → None.
        assert_eq!(enum_label(&5_i32), None);
    }
}
