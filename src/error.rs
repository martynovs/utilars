use thiserror::Error;

pub type Result<T> = std::result::Result<T, ApiError>;

/// Everything that can go wrong **talking to the Utila API** — auth, transport, a server-returned
/// status, or client configuration. Value/format problems that aren't API calls have their own
/// errors instead (e.g. [`crate::AmountError`], or [`serde_json::Error`] from webhook decoding).
#[derive(Debug, Error)]
pub enum ApiError {
    /// Token minting / signing failed, or the credential is malformed.
    #[error("auth error: {0}")]
    Auth(String),

    /// Transport-level failure (connection, timeout, body read).
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    /// The API returned a non-success status with a gRPC status envelope.
    /// `details` carries the `google.rpc.Status.details` entries verbatim (each a
    /// `google.protobuf.Any` JSON object); empty for client-side synthetic errors.
    #[error("api error (code {code}): {message}")]
    Api {
        code: i32,
        message: String,
        details: Vec<serde_json::Value>,
    },

    /// Missing or invalid client configuration.
    #[error("config error: {0}")]
    Config(String),
}

impl ApiError {
    /// A required field was absent from an otherwise-successful response. Synthetic
    /// (`code -1`, no `details`) — distinct from a real server-returned status.
    pub(crate) fn missing(field: &str) -> Self {
        ApiError::Api {
            code: -1,
            message: format!("{field} missing in response"),
            details: Vec::new(),
        }
    }

    /// Whether retrying the operation might plausibly succeed — transient transport
    /// failures and server-side "try again later" statuses, but not client mistakes
    /// (bad request, not-found, auth). Shaped as `fn(&ApiError) -> bool` so it drops
    /// straight into a retry crate's predicate, e.g. with `backon`:
    /// `op.retry(ExponentialBuilder::default()).when(ApiError::is_retryable)`.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            // Connect/timeout are transient; a malformed request or decode is not.
            ApiError::Http(e) => e.is_timeout() || e.is_connect(),
            // gRPC canonical codes (DEADLINE_EXCEEDED / RESOURCE_EXHAUSTED / UNAVAILABLE)
            // plus the HTTP-status fallback `parse_api_error` uses when there is no gRPC
            // status envelope in the body.
            ApiError::Api { code, .. } => matches!(*code, 4 | 8 | 14 | 429 | 500..=599),
            ApiError::Auth(_) | ApiError::Config(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ApiError;

    #[test]
    fn is_retryable_classifies_codes_and_kinds() {
        let api = |code| ApiError::Api {
            code,
            message: String::new(),
            details: Vec::new(),
        };
        // gRPC UNAVAILABLE + HTTP 5xx/429 fallbacks retry; NOT_FOUND and client-side don't.
        assert!(api(14).is_retryable());
        assert!(api(503).is_retryable());
        assert!(api(429).is_retryable());
        assert!(!api(5).is_retryable());
        assert!(!ApiError::Auth("nope".into()).is_retryable());
        assert!(!ApiError::Config("nope".into()).is_retryable());
    }

    #[tokio::test]
    async fn http_transport_errors_are_retryable() {
        // A connect failure is transient; port 1 on loopback refuses immediately.
        let e = reqwest::Client::new()
            .get("http://127.0.0.1:1")
            .send()
            .await
            .unwrap_err();
        assert!(ApiError::Http(e).is_retryable());
    }
}
