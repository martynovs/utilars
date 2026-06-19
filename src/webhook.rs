//! Receiver-side webhook signature verification and typed events.
//!
//! Utila signs each webhook POST over the **raw** request body and sends the signature in the
//! `x-utila-signature` header: RSA-4096 / SHA-512 / PSS (salt length = digest size), base64.
//! This is a *different* scheme from auth JWT signing (PKCS#1-v1.5 / SHA-256). [`verify`]
//! checks the signature against Utila's published key; [`VerifiedEvent::parse`] then decodes
//! the verified body into a typed [`Event`].
//!
//! ```no_run
//! # fn handle(raw_body: &[u8], sig_header: &str) -> utilars::Result<()> {
//! let verified = utilars::webhook::verify(raw_body, sig_header)?;
//! match verified.parse()?.kind {
//!     utilars::webhook::EventKind::TransactionCreated => { /* … */ }
//!     _ => {}
//! }
//! # Ok(()) }
//! ```

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use rsa::pkcs8::DecodePublicKey;
use rsa::pss::{Signature, VerifyingKey};
use rsa::signature::Verifier;
use rsa::RsaPublicKey;
use serde::Deserialize;
use sha2::Sha512;

use crate::error::{Result, UtilaError};

/// Utila's published webhook public key (RSA-4096), bundled as the default for [`verify`].
/// Override via [`verify_with_key`] if Utila rotates it before this crate ships an update.
pub const UTILA_WEBHOOK_PUBLIC_KEY: &str = "-----BEGIN PUBLIC KEY-----
MIICIjANBgkqhkiG9w0BAQEFAAOCAg8AMIICCgKCAgEAulI1XPGRDFcymdf2zXvD
spfdTXA1g0NOavZ50+AtcQP7f+KTpXoO1bkr6x9dO2Jq8FHImRT1sbhKhcNXT4WC
dLSa/2Zh60QE3tp9d51o1XDSnzRMwcGbFyJ7C30DVpVEIwqD2Z5GRlzXinqIeVdY
GOubuVol/wOAynS32DX+6y2PiqbYj7P84csBOgpNT27Mc6InEqKb7LWQtU8LPttx
tfyceOPXE5G4h+UujPsPG6WN5MHHVbP9r6oneEF3knbfL3hCJRjwV9HfTtG6JyYr
25Dy6SOCphrlEZi8IGcKxL6fEMetDGGVCjm7XfHyt6fYoUonD9lZsvbSyUsRwf/1
+x77F2LxtzQyvMJR9jD16WUyzm+fUBSVQixxKnKSrVkeLqkmGboDTY5kw3doSVTP
zcGDzWkzqC3lgwRLnSg4J+koQY+yo9jYBbFdSp+/PfVmp9NEaBuCV63mp/85VWIh
1FRYe6lEdGZWdmIcbDvNYU/Cui/yGZoID7+sJJq/rWN0Qxx/0skEaT/083+iYLVA
QNLvWtmQfgNKPm6GeQknRUEWyWUJtq6ANeP/8hGVM1G/edOdLn+KfhXZvw41O5z1
uKHEqHIV+NaCNnFbDj924bJhA/fWNKxYv7/Nm44Wy1nXlgqdHiFkSqtjUBPmzE/n
yj92azWBq1RbGHY+9/POguMCAwEAAQ==
-----END PUBLIC KEY-----
";

/// Verify `signature_b64` (the `x-utila-signature` header, base64) over the raw `body`
/// against Utila's bundled public key. On success the returned [`VerifiedEvent`] holds the
/// verified bytes; call [`VerifiedEvent::parse`] to decode the typed event.
///
/// # Errors
/// Returns [`UtilaError::Auth`] if the signature is malformed or does not verify.
pub fn verify(body: &[u8], signature_b64: &str) -> Result<VerifiedEvent> {
    verify_with_key(body, signature_b64, UTILA_WEBHOOK_PUBLIC_KEY)
}

/// Like [`verify`] but against a caller-supplied PEM public key (testing, or a rotated key).
///
/// # Errors
/// [`UtilaError::Config`] if the key PEM is invalid; [`UtilaError::Auth`] if the signature is
/// malformed or does not verify.
pub fn verify_with_key(
    body: &[u8],
    signature_b64: &str,
    public_key_pem: &str,
) -> Result<VerifiedEvent> {
    let sig_bytes = B64
        .decode(signature_b64.trim())
        .map_err(|e| UtilaError::Auth(format!("invalid webhook signature base64: {e}")))?;
    let key = RsaPublicKey::from_public_key_pem(public_key_pem)
        .map_err(|e| UtilaError::Config(format!("invalid webhook public key: {e}")))?;
    let verifying_key = VerifyingKey::<Sha512>::new(key);
    let signature = Signature::try_from(sig_bytes.as_slice())
        .map_err(|e| UtilaError::Auth(format!("malformed webhook signature: {e}")))?;
    #[expect(
        clippy::map_err_ignore,
        reason = "the rsa signature error is deliberately opaque; a uniform failure is the right surface"
    )]
    verifying_key
        .verify(body, &signature)
        .map_err(|_| UtilaError::Auth("webhook signature verification failed".into()))?;
    Ok(VerifiedEvent {
        body: body.to_vec(),
    })
}

/// A webhook body whose signature has been verified. Parsing into a typed [`Event`] is a
/// separate step (an unknown/new event shape can't retroactively invalidate the signature).
#[derive(Debug, Clone)]
pub struct VerifiedEvent {
    body: Vec<u8>,
}

impl VerifiedEvent {
    /// Decode the verified body into a typed [`Event`].
    ///
    /// # Errors
    /// [`UtilaError::Api`] if the body is not a valid event payload.
    pub fn parse(&self) -> Result<Event> {
        serde_json::from_slice(&self.body).map_err(|e| UtilaError::Api {
            code: -1,
            message: format!("invalid webhook payload: {e}"),
            details: Vec::new(),
        })
    }

    /// The verified raw body bytes.
    #[must_use]
    pub fn raw_body(&self) -> &[u8] {
        &self.body
    }
}

/// A Utila webhook event.
#[derive(Debug, Clone, Deserialize)]
pub struct Event {
    pub id: String,
    pub vault: String,
    #[serde(rename = "type")]
    pub kind: EventKind,
    #[serde(default, rename = "resourceType")]
    pub resource_type: Option<String>,
    #[serde(default)]
    pub resource: Option<String>,
    #[serde(default)]
    pub details: Option<EventDetails>,
}

/// The webhook event type. Unknown/future types deserialize to [`EventKind::Unknown`] rather
/// than failing, so a new server-side event can't break a deployed receiver.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum EventKind {
    #[serde(rename = "TRANSACTION_CREATED")]
    TransactionCreated,
    #[serde(rename = "TRANSACTION_STATE_UPDATED")]
    TransactionStateUpdated,
    #[serde(rename = "TRANSACTION_AML_SCREENING_RESULT_READY")]
    TransactionAmlScreeningResultReady,
    #[serde(rename = "WALLET_CREATED")]
    WalletCreated,
    #[serde(rename = "WALLET_ADDRESS_CREATED")]
    WalletAddressCreated,
    #[serde(rename = "TEST")]
    Test,
    #[serde(other)]
    Unknown,
}

/// Event-specific detail payloads (only populated for the relevant `kind`).
#[derive(Debug, Clone, Deserialize)]
pub struct EventDetails {
    #[serde(default, rename = "transactionStateUpdated")]
    pub transaction_state_updated: Option<TransactionStateUpdated>,
}

/// Detail for [`EventKind::TransactionStateUpdated`].
#[derive(Debug, Clone, Deserialize)]
pub struct TransactionStateUpdated {
    #[serde(default, rename = "newState")]
    pub new_state: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::pkcs8::{DecodePrivateKey, EncodePublicKey, LineEnding};
    use rsa::pss::SigningKey;
    use rsa::signature::{RandomizedSigner, SignatureEncoding};
    use rsa::RsaPrivateKey;

    /// Load the committed test key and derive its public PEM (a *different* key from the
    /// bundled Utila key, so it also serves as the "wrong key" in rejection tests).
    fn test_keypair() -> (RsaPrivateKey, String) {
        let pem = std::str::from_utf8(include_bytes!("../tests/test_key.pem")).unwrap();
        let priv_key = RsaPrivateKey::from_pkcs8_pem(pem).unwrap();
        let pub_pem = priv_key
            .to_public_key()
            .to_public_key_pem(LineEnding::LF)
            .unwrap();
        (priv_key, pub_pem)
    }

    fn sign(priv_key: &RsaPrivateKey, body: &[u8]) -> String {
        let signing_key = SigningKey::<Sha512>::new(priv_key.clone());
        let sig = signing_key.sign_with_rng(&mut rand::thread_rng(), body);
        B64.encode(sig.to_bytes())
    }

    const SAMPLE: &str = r#"{"id":"94ea3ce9","vault":"vaults/abc","type":"TRANSACTION_CREATED","resourceType":"TRANSACTION","resource":"vaults/abc/transactions/t1"}"#;

    #[test]
    fn authentic_event_verifies_and_parses() {
        let (priv_key, pub_pem) = test_keypair();
        let sig = sign(&priv_key, SAMPLE.as_bytes());

        let verified = verify_with_key(SAMPLE.as_bytes(), &sig, &pub_pem).unwrap();
        assert_eq!(verified.raw_body(), SAMPLE.as_bytes());
        let event = verified.parse().unwrap();
        assert_eq!(event.kind, EventKind::TransactionCreated);
        assert_eq!(event.vault, "vaults/abc");
        assert_eq!(
            event.resource.as_deref(),
            Some("vaults/abc/transactions/t1")
        );
    }

    #[test]
    fn tampered_body_is_rejected() {
        let (priv_key, pub_pem) = test_keypair();
        let sig = sign(&priv_key, SAMPLE.as_bytes());
        let mut tampered = SAMPLE.as_bytes().to_vec();
        tampered.extend_from_slice(b" ");
        let err = verify_with_key(&tampered, &sig, &pub_pem).unwrap_err();
        assert!(matches!(err, UtilaError::Auth(_)));
    }

    #[test]
    fn verify_uses_the_bundled_key() {
        // `verify` (the default-key wrapper) checks against the bundled Utila key, so a
        // signature made with the test key is rejected — exercising the wrapper itself.
        let (priv_key, _pub_pem) = test_keypair();
        let sig = sign(&priv_key, SAMPLE.as_bytes());
        assert!(matches!(
            verify(SAMPLE.as_bytes(), &sig),
            Err(UtilaError::Auth(_))
        ));
    }

    #[test]
    fn wrong_key_is_rejected() {
        let (priv_key, _pub_pem) = test_keypair();
        let sig = sign(&priv_key, SAMPLE.as_bytes());
        // Verify against the bundled Utila key (a different key) → must fail.
        let err = verify_with_key(SAMPLE.as_bytes(), &sig, UTILA_WEBHOOK_PUBLIC_KEY).unwrap_err();
        assert!(matches!(err, UtilaError::Auth(_)));
    }

    #[test]
    fn malformed_signature_base64_is_rejected() {
        let (_priv, pub_pem) = test_keypair();
        let err = verify_with_key(SAMPLE.as_bytes(), "not base64!!!", &pub_pem).unwrap_err();
        assert!(matches!(err, UtilaError::Auth(_)));
    }

    #[test]
    fn invalid_public_key_is_config_error() {
        let err =
            verify_with_key(SAMPLE.as_bytes(), &B64.encode([0u8; 8]), "not a pem").unwrap_err();
        assert!(matches!(err, UtilaError::Config(_)));
    }

    #[test]
    fn bundled_utila_key_is_a_valid_rsa_public_key() {
        RsaPublicKey::from_public_key_pem(UTILA_WEBHOOK_PUBLIC_KEY)
            .expect("bundled Utila webhook key parses");
    }

    #[test]
    fn verified_body_parses_each_event_kind() {
        let (priv_key, pub_pem) = test_keypair();
        let cases = [
            (
                r#"{"id":"1","vault":"v","type":"TRANSACTION_CREATED"}"#,
                EventKind::TransactionCreated,
            ),
            (
                r#"{"id":"1","vault":"v","type":"TRANSACTION_STATE_UPDATED","details":{"transactionStateUpdated":{"newState":"CONFIRMED"}}}"#,
                EventKind::TransactionStateUpdated,
            ),
            (
                r#"{"id":"1","vault":"v","type":"TRANSACTION_AML_SCREENING_RESULT_READY"}"#,
                EventKind::TransactionAmlScreeningResultReady,
            ),
            (
                r#"{"id":"1","vault":"v","type":"WALLET_CREATED"}"#,
                EventKind::WalletCreated,
            ),
            (
                r#"{"id":"1","vault":"v","type":"WALLET_ADDRESS_CREATED"}"#,
                EventKind::WalletAddressCreated,
            ),
            (r#"{"id":"1","vault":"v","type":"TEST"}"#, EventKind::Test),
            (
                r#"{"id":"1","vault":"v","type":"SOMETHING_NEW"}"#,
                EventKind::Unknown,
            ),
        ];
        for (body, expected) in cases {
            let sig = sign(&priv_key, body.as_bytes());
            let event = verify_with_key(body.as_bytes(), &sig, &pub_pem)
                .unwrap()
                .parse()
                .unwrap();
            assert_eq!(event.kind, expected, "body: {body}");
        }
        // The state-updated detail decodes.
        let body = r#"{"id":"1","vault":"v","type":"TRANSACTION_STATE_UPDATED","details":{"transactionStateUpdated":{"newState":"CONFIRMED"}}}"#;
        let sig = sign(&priv_key, body.as_bytes());
        let event = verify_with_key(body.as_bytes(), &sig, &pub_pem)
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            event
                .details
                .and_then(|d| d.transaction_state_updated)
                .and_then(|t| t.new_state)
                .as_deref(),
            Some("CONFIRMED")
        );
    }

    #[test]
    fn verified_but_nonjson_body_fails_to_parse() {
        let (priv_key, pub_pem) = test_keypair();
        let body = b"not json";
        let sig = sign(&priv_key, body);
        let verified = verify_with_key(body, &sig, &pub_pem).unwrap();
        // Signature is valid, but the payload isn't an event.
        assert!(matches!(
            verified.parse(),
            Err(UtilaError::Api { code: -1, .. })
        ));
    }
}
