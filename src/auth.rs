use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine as _;
use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey};
use serde::Serialize;
use tokio::sync::RwLock;
use tokio::time::Instant;

use crate::error::{ApiError, Result};
use crate::kms::KmsKey;

/// The JWT `aud` Utila requires — a fixed constant, NOT derived from the base URL.
/// (Note the trailing slash.)
pub(crate) const AUDIENCE: &str = "https://api.utila.io/";

/// Produces the base64url signature over a JWT signing-input. The local signer runs in
/// process (RSASSA-PKCS1-v1.5 / SHA-256 via `jsonwebtoken`); the `aws` signer runs remotely
/// via KMS. Native `async fn` (static dispatch — no `async-trait`, no `dyn Signer`).
trait Signer {
    async fn sign(&self, signing_input: &[u8]) -> Result<String>;
}

/// In-process RS256 signer wrapping a parsed local key.
struct LocalRsaSigner<'a> {
    key: &'a EncodingKey,
}

impl Signer for LocalRsaSigner<'_> {
    async fn sign(&self, signing_input: &[u8]) -> Result<String> {
        jsonwebtoken::crypto::sign(signing_input, self.key, Algorithm::RS256)
            .map_err(|e| ApiError::Auth(format!("sign failed: {e}")))
    }
}

/// Remote signer that delegates to AWS KMS (`RSASSA_PKCS1_V15_SHA_256`). KMS computes the
/// SHA-256 digest itself (`MessageType::Raw`), so the small JWT signing-input is sent as-is.
#[cfg(feature = "aws")]
struct AwsKmsSigner<'a> {
    client: &'a aws_sdk_kms::Client,
    key_id: &'a str,
}

#[cfg(feature = "aws")]
impl Signer for AwsKmsSigner<'_> {
    async fn sign(&self, signing_input: &[u8]) -> Result<String> {
        use aws_sdk_kms::primitives::Blob;
        use aws_sdk_kms::types::{MessageType, SigningAlgorithmSpec};

        let out = self
            .client
            .sign()
            .key_id(self.key_id)
            .message(Blob::new(signing_input))
            .message_type(MessageType::Raw)
            .signing_algorithm(SigningAlgorithmSpec::RsassaPkcs1V15Sha256)
            .send()
            .await
            .map_err(|e| ApiError::Auth(format!("AWS KMS sign failed: {e}")))?;
        let sig = out
            .signature()
            .ok_or_else(|| ApiError::Auth("AWS KMS returned no signature".into()))?;
        Ok(B64.encode(sig.as_ref()))
    }
}

const TOKEN_TTL: Duration = Duration::from_hours(1);
const REFRESH_MARGIN: Duration = Duration::from_mins(5);

/// Where the JWT signature comes from. Mutually exclusive — exactly one source is
/// valid — so it is an enum, not a pair of optional fields. Both variants hold a
/// *validated* value, so an invalid signer can't be constructed (the parsing/validation
/// happens in the constructors below, which return `Result`).
#[derive(Clone)]
pub enum SignerSource {
    /// A parsed local RSA signing key (the key bytes are zeroized on drop).
    Local(EncodingKey),
    /// A validated KMS key (e.g. AWS). Requires the `kms` feature to sign.
    Kms(KmsKey),
}

impl SignerSource {
    /// Parse an RSA private key in PEM form (PKCS#1 or PKCS#8). Errors on invalid PEM.
    pub fn local_pem(pem: &[u8]) -> Result<Self> {
        EncodingKey::from_rsa_pem(pem)
            .map(SignerSource::Local)
            .map_err(|e| ApiError::Auth(format!("invalid RSA private key: {e}")))
    }
}

// Redacting Debug: the local key is secret material and must never be printed.
impl std::fmt::Debug for SignerSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignerSource::Local(_) => f.write_str("SignerSource::Local(<redacted>)"),
            SignerSource::Kms(key) => f.debug_tuple("SignerSource::Kms").field(key).finish(),
        }
    }
}

#[derive(Serialize)]
struct Claims<'a> {
    sub: &'a str,
    aud: &'a str,
    exp: u64,
}

/// The JWT header. Fixed `RS256` regardless of signer backend (both local and KMS produce
/// RSASSA-PKCS1-v1.5 / SHA-256 signatures).
#[derive(Serialize)]
struct JwtHeader {
    alg: &'static str,
    typ: &'static str,
}

struct Cached {
    token: String,
    /// Re-mint once the monotonic clock passes this point (mint + TTL − margin).
    refresh_after: Instant,
}

/// Mints and caches the self-signed RS256 JWT used as the API bearer token.
///
/// Refresh timing runs on the (mockable) `tokio::time` clock; the wall clock is read
/// exactly once — in `new`, the crate's sole non-injectable time read — to anchor the JWT
/// `exp` claim. "Now" is then reconstructed as `anchor + tokio_elapsed`, so token expiry is
/// driven by the tokio clock. Tests bypass the wall clock entirely via `with_anchor_unix` +
/// `tokio::time::advance`, making `exp` fully deterministic.
pub struct TokenManager {
    account: String,
    signer: SignerSource,
    anchor_unix: i64,
    anchor_instant: Instant,
    cached: RwLock<Option<Cached>>,
    /// Lazily-built KMS client, shared across signs (a KMS client carries connection pools and
    /// credential caches; rebuilding per token would be wasteful). Only present with `aws`.
    #[cfg(feature = "aws")]
    kms_client: tokio::sync::OnceCell<aws_sdk_kms::Client>,
}

impl TokenManager {
    pub fn new(account: impl Into<String>, signer: SignerSource) -> Self {
        // The one unavoidable real wall-clock read (a JWT `exp` must be real Unix time),
        // isolated to this single call so it is the *only* non-injectable time source in the
        // crate. Everything downstream runs on `tokio::time` (advanceable in tests), and tests
        // pin this anchor via `with_anchor_unix` rather than reading the wall clock.
        Self::with_anchor_unix(account, signer, Utc::now().timestamp())
    }

    /// Construct with an explicit wall-clock anchor (Unix seconds). Combined with
    /// `tokio::time::advance`, this lets a test drive `exp` fully deterministically without
    /// ever reading the real clock.
    fn with_anchor_unix(
        account: impl Into<String>,
        signer: SignerSource,
        anchor_unix: i64,
    ) -> Self {
        Self {
            account: account.into(),
            signer,
            anchor_unix,
            anchor_instant: Instant::now(),
            cached: RwLock::new(None),
            #[cfg(feature = "aws")]
            kms_client: tokio::sync::OnceCell::new(),
        }
    }

    /// Return a valid bearer token, minting (or refreshing) if necessary.
    pub async fn token(&self) -> Result<String> {
        // Fast path: a shared read lock, no minting.
        if let Some(c) = self.cached.read().await.as_ref() {
            if Instant::now() < c.refresh_after {
                return Ok(c.token.clone());
            }
        }
        self.refresh().await
    }

    /// Mint a fresh token under the exclusive write lock — or return the cached one if another
    /// caller refreshed while we were waiting for the lock (the double-checked-lock recheck
    /// that single-flights concurrent refreshes; one signature per refresh, which matters for
    /// billed KMS calls). The guard is a tokio lock, so awaiting the possibly-remote signer
    /// across it is sound. Split from `token` so that recheck branch — otherwise only reachable
    /// under a genuine race — is unit-testable by pre-filling the cache and calling this.
    async fn refresh(&self) -> Result<String> {
        let mut guard = self.cached.write().await;
        let now = Instant::now();
        if let Some(c) = guard.as_ref() {
            if now < c.refresh_after {
                return Ok(c.token.clone());
            }
        }

        let token = self.mint(now).await?;
        *guard = Some(Cached {
            token: token.clone(),
            refresh_after: now + TOKEN_TTL - REFRESH_MARGIN,
        });
        Ok(token)
    }

    /// Assemble and sign a fresh JWT: `base64url(header).base64url(claims).base64url(sig)`.
    async fn mint(&self, now: Instant) -> Result<String> {
        let elapsed_secs =
            i64::try_from(now.saturating_duration_since(self.anchor_instant).as_secs())
                .unwrap_or(i64::MAX);
        let ttl_secs = i64::try_from(TOKEN_TTL.as_secs()).unwrap_or(i64::MAX);
        let exp = u64::try_from(self.anchor_unix + elapsed_secs + ttl_secs).unwrap_or(0);
        let claims = Claims {
            sub: &self.account,
            aud: AUDIENCE,
            exp,
        };
        let header = JwtHeader {
            alg: "RS256",
            typ: "JWT",
        };
        let header_json = serde_json::to_vec(&header)
            .map_err(|e| ApiError::Auth(format!("encode header: {e}")))?;
        let claims_json = serde_json::to_vec(&claims)
            .map_err(|e| ApiError::Auth(format!("encode claims: {e}")))?;
        let signing_input = format!("{}.{}", B64.encode(header_json), B64.encode(claims_json));
        let signature = self.sign(signing_input.as_bytes()).await?;
        Ok(format!("{signing_input}.{signature}"))
    }

    /// Dispatch signing to the configured backend.
    async fn sign(&self, signing_input: &[u8]) -> Result<String> {
        match &self.signer {
            SignerSource::Local(key) => LocalRsaSigner { key }.sign(signing_input).await,
            SignerSource::Kms(key) => self.sign_kms(key, signing_input).await,
        }
    }

    #[cfg(feature = "aws")]
    async fn sign_kms(&self, key: &KmsKey, signing_input: &[u8]) -> Result<String> {
        let KmsKey::Aws(arn) = key;
        let client = self.kms_client(arn).await;
        AwsKmsSigner {
            client,
            key_id: arn.as_str(),
        }
        .sign(signing_input)
        .await
    }

    #[cfg(not(feature = "aws"))]
    #[expect(
        clippy::unused_async,
        reason = "mirrors the async `aws` variant so the call site is feature-agnostic"
    )]
    async fn sign_kms(&self, key: &KmsKey, _signing_input: &[u8]) -> Result<String> {
        Err(ApiError::Auth(format!(
            "KMS signing for {key} requires the `aws` feature (build with `--features aws`)"
        )))
    }

    /// The shared KMS client, built (once) from the default AWS config with the ARN's region.
    #[cfg(feature = "aws")]
    async fn kms_client(&self, arn: &crate::kms::AwsKmsArn) -> &aws_sdk_kms::Client {
        self.kms_client
            .get_or_init(|| async {
                let region = aws_config::Region::new(arn.region().to_string());
                let cfg = aws_config::defaults(aws_config::BehaviorVersion::latest())
                    .region(region)
                    .load()
                    .await;
                aws_sdk_kms::Client::new(&cfg)
            })
            .await
    }

    pub async fn invalidate(&self) {
        *self.cached.write().await = None;
    }
}

// Manual, redacting Debug: the generated `Client` derives `Debug` and holds an
// `Arc<TokenManager>`, but the signer/cached token are secrets and must not be printed.
impl std::fmt::Debug for TokenManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenManager")
            .field("account", &self.account)
            .finish_non_exhaustive()
    }
}

/// progenitor async pre-hook: stamp `Authorization: Bearer <jwt>` on every request.
/// The generated `Client` carries an `Arc<TokenManager>` as its inner state, which the
/// hook receives. Returning `Err` aborts the request before it hits the network.
pub(crate) async fn inject_bearer(
    tokens: &std::sync::Arc<TokenManager>,
    req: &mut reqwest::Request,
) -> Result<()> {
    let token = tokens.token().await?;
    let value = format!("Bearer {token}")
        .parse()
        .map_err(|e: reqwest::header::InvalidHeaderValue| ApiError::Auth(e.to_string()))?;
    req.headers_mut()
        .insert(reqwest::header::AUTHORIZATION, value);
    Ok(())
}

/// progenitor async post-hook: on a `401 Unauthorized`, drop the cached token so the next
/// request re-mints (defends against a token the server rejected — clock skew, a rotated
/// key, early revocation). Never fails the request: invalidation is a side-effect, and the
/// `401` itself still surfaces to the caller as a typed error.
pub(crate) async fn on_response(
    tokens: &std::sync::Arc<TokenManager>,
    result: &std::result::Result<reqwest::Response, reqwest::Error>,
) -> Result<()> {
    if let Ok(resp) = result {
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            tokens.invalidate().await;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_KEY: &[u8] = include_bytes!("../tests/test_key.pem");

    fn manager() -> TokenManager {
        TokenManager::new(
            "sa@vault-x.utilaserviceaccount.io",
            SignerSource::local_pem(TEST_KEY).unwrap(),
        )
    }

    #[test]
    fn signer_debug_redacts_local_and_labels_kms() {
        let local = SignerSource::local_pem(TEST_KEY).unwrap();
        assert_eq!(format!("{local:?}"), "SignerSource::Local(<redacted>)");

        let kms = SignerSource::Kms(
            KmsKey::parse("awskms:///arn:aws:kms:us-east-1:123:key/abc").unwrap(),
        );
        assert!(format!("{kms:?}").contains("Kms"));
    }

    #[test]
    fn token_manager_debug_omits_secrets() {
        let dbg = format!("{:?}", manager());
        assert!(dbg.contains("TokenManager"), "got: {dbg}");
        assert!(
            dbg.contains("utilaserviceaccount.io"),
            "account shown: {dbg}"
        );
        assert!(!dbg.to_lowercase().contains("begin"), "leaked key: {dbg}");
    }

    #[tokio::test(start_paused = true)]
    async fn caches_until_refresh_margin_then_remints() {
        let tm = manager();

        let first = tm.token().await.unwrap();
        // within the usable window (< TTL − margin = 55 min) → cached token reused
        tokio::time::advance(Duration::from_mins(50)).await;
        assert_eq!(
            tm.token().await.unwrap(),
            first,
            "should reuse cached token"
        );

        // cross into the 5-min refresh margin → re-mint (later exp ⇒ different token)
        tokio::time::advance(Duration::from_mins(10)).await; // 60 min total
        let second = tm.token().await.unwrap();
        assert_ne!(second, first, "should re-mint near expiry");

        // invalidate forces the next call to re-mint even within the window
        tm.invalidate().await;
        tokio::time::advance(Duration::from_mins(5)).await;
        assert_ne!(
            tm.token().await.unwrap(),
            second,
            "invalidate should force re-mint"
        );
    }

    fn decode_exp(token: &str) -> u64 {
        let claims_b64 = token.split('.').nth(1).expect("claims segment");
        let claims: serde_json::Value =
            serde_json::from_slice(&B64.decode(claims_b64).unwrap()).unwrap();
        claims["exp"].as_u64().expect("exp is a number")
    }

    #[tokio::test(start_paused = true)]
    async fn exp_tracks_injected_anchor_and_tokio_clock() {
        // No wall-clock read: the anchor is injected and the tokio clock is advanced, so `exp`
        // is fully deterministic — the rule that time-dependent logic use only inject-able
        // providers, applied to the JWT `exp` itself.
        let anchor: i64 = 1_700_000_000;
        let ttl = TOKEN_TTL.as_secs();
        let tm = TokenManager::with_anchor_unix(
            "sa@vault-x.utilaserviceaccount.io",
            SignerSource::local_pem(TEST_KEY).unwrap(),
            anchor,
        );

        // Minted at elapsed 0 → exp = anchor + TTL.
        let exp0 = decode_exp(&tm.token().await.unwrap());
        assert_eq!(exp0, u64::try_from(anchor).unwrap() + ttl);

        // Advance 50 min, force a re-mint → exp moves by exactly the elapsed seconds.
        tm.invalidate().await;
        tokio::time::advance(Duration::from_mins(50)).await;
        let exp1 = decode_exp(&tm.token().await.unwrap());
        assert_eq!(exp1, u64::try_from(anchor).unwrap() + ttl + 50 * 60);
    }

    #[tokio::test]
    async fn mints_well_formed_rs256_jwt() {
        // The manual assembly must yield a 3-part JWT whose header is RS256 and whose claims
        // carry the exact `aud`/`sub`/an `exp`.
        let token = manager().token().await.unwrap();
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3, "header.claims.signature");

        let header: serde_json::Value =
            serde_json::from_slice(&B64.decode(parts[0]).unwrap()).unwrap();
        assert_eq!(header["alg"], "RS256");
        assert_eq!(header["typ"], "JWT");

        let claims: serde_json::Value =
            serde_json::from_slice(&B64.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(claims["aud"], AUDIENCE);
        assert_eq!(claims["sub"], "sa@vault-x.utilaserviceaccount.io");
        assert!(claims["exp"].is_number());
        assert!(!parts[2].is_empty(), "signature present");
    }

    #[tokio::test(start_paused = true)]
    async fn refresh_returns_cached_token_when_already_fresh_under_lock() {
        // Models the double-checked-lock race: another caller refreshed while we waited for
        // the write lock. Pre-fill the cache (via `token`), then call `refresh` directly — the
        // under-lock recheck must find it fresh and return it without re-minting.
        let tm = manager();
        let first = tm.token().await.unwrap();
        let again = tm.refresh().await.unwrap();
        assert_eq!(
            again, first,
            "fresh cache short-circuits the write-lock mint path"
        );
    }

    #[tokio::test]
    async fn on_response_invalidates_cache_only_on_401() {
        use wiremock::matchers::path;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(path("/ok"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        Mock::given(path("/unauth"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let tokens = std::sync::Arc::new(manager());
        let http = reqwest::Client::new();
        tokens.token().await.unwrap();
        assert!(tokens.cached.read().await.is_some(), "cache populated");

        // A 200 leaves the cache intact.
        let ok = http
            .get(format!("{}/ok", server.uri()))
            .send()
            .await
            .unwrap();
        on_response(&tokens, &Ok(ok)).await.unwrap();
        assert!(
            tokens.cached.read().await.is_some(),
            "2xx must not invalidate"
        );

        // A transport error is a no-op (port 1 refuses immediately).
        let err = http.get("http://127.0.0.1:1/").send().await.unwrap_err();
        on_response(&tokens, &Err(err)).await.unwrap();
        assert!(
            tokens.cached.read().await.is_some(),
            "transport error must not invalidate"
        );

        // A 401 drops the cached token so the next call re-mints.
        let unauth = http
            .get(format!("{}/unauth", server.uri()))
            .send()
            .await
            .unwrap();
        on_response(&tokens, &Ok(unauth)).await.unwrap();
        assert!(
            tokens.cached.read().await.is_none(),
            "401 must invalidate the cached token"
        );
    }

    // Without the `aws` feature the KMS path returns a typed error pointing at the flag.
    #[cfg(not(feature = "aws"))]
    #[tokio::test]
    async fn kms_signer_cannot_sign_without_feature() {
        let kms = KmsKey::parse("awskms:///arn:aws:kms:us-east-1:123:key/abc").unwrap();
        let tm = TokenManager::new("sa@vault-x.utilaserviceaccount.io", SignerSource::Kms(kms));
        let err = tm.token().await.unwrap_err();
        assert!(matches!(err, ApiError::Auth(m) if m.contains("aws")));
    }
}
