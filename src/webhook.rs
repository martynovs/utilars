//! Receiver-side webhook signature verification.
//!
//! Utila signs each webhook POST over the **raw** request body and sends the signature in the
//! `x-utila-signature` header: RSA-4096 / SHA-512 / PSS (salt length = digest size), base64.
//! This is a *different* scheme from auth JWT signing (PKCS#1-v1.5 / SHA-256). [`verify`]
//! checks the signature against Utila's published key; [`Event::parse`] then decodes the verified
//! body into a typed [`Event`] ([`VerifiedPayload`] impls `AsRef<[u8]>`, so it parses directly).
//!
//! # Idempotency / replay
//!
//! Utila's scheme carries **no timestamp**, so a signature alone can't prove freshness — a
//! captured valid `(body, signature)` pair stays valid forever and can be redelivered (Utila
//! retries failed deliveries, and a network attacker who sees one delivery can replay it).
//! Verification therefore proves *authenticity*, not *novelty*. Process events **idempotently**:
//! treat each event's `id` as a dedupe key and make repeated delivery of the same id a no-op.
//!
//! ```no_run
//! # fn handle(raw_body: &[u8], sig_header: &str) -> utilars::Result<()> {
//! let verified = utilars::webhook::verify(raw_body, sig_header)?;
//! match utilars::webhook::Event::parse(verified)? {
//!     utilars::webhook::Event::TransactionCreated { transaction, .. } => { /* … */ }
//!     _ => {}
//! }
//! # Ok(()) }
//! ```

use std::sync::LazyLock;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use rsa::pkcs8::DecodePublicKey;
use rsa::pss::{Signature, VerifyingKey};
use rsa::signature::Verifier;
use rsa::RsaPublicKey;
use sha2::Sha512;

use crate::error::{ApiError, Result};
pub use crate::webhook_event::{AmlAction, Event, TransactionState};

/// Utila's published webhook public key (RSA-4096), bundled as the default for [`verify`].
/// Override via [`WebhookKey::from_pem`] + [`verify_with_key`] if Utila rotates it before this
/// crate ships an update.
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

/// SHA-256 (hex) of [`UTILA_WEBHOOK_PUBLIC_KEY`]'s canonical SPKI DER encoding — a stable
/// fingerprint of the bundled key. Pinned so a stale or accidentally-edited key is caught by
/// a test at build time rather than by every real webhook silently failing to verify in
/// production. Verified equal to `openssl pkey -pubin -pubout -outform DER | openssl dgst
/// -sha256` over the bundled PEM. If Utila rotates the key, update the PEM and this hash
/// together (and prefer [`verify_with_key`] until the new crate version ships).
pub const UTILA_WEBHOOK_PUBLIC_KEY_SHA256: &str =
    "78d93f384ec9f956170657d5600f8e93e04bb049687163e41a05a6d66f03bf9a";

/// A parsed Utila webhook verifying key (RSA-4096 / SHA-512 / PSS). Constructing one validates
/// and parses the PEM **once**; reuse it across many [`verify_with_key`] calls so an RSA-4096
/// key isn't re-parsed on every request.
#[derive(Clone, Debug)]
pub struct WebhookKey(VerifyingKey<Sha512>);

impl WebhookKey {
    /// Parse a PEM-encoded RSA public key.
    ///
    /// # Errors
    /// [`ApiError::Config`] if the PEM is not a valid RSA public key.
    pub fn from_pem(pem: &str) -> Result<Self> {
        let key = RsaPublicKey::from_public_key_pem(pem)
            .map_err(|e| ApiError::Config(format!("invalid webhook public key: {e}")))?;
        Ok(Self(VerifyingKey::<Sha512>::new(key)))
    }
}

/// The bundled Utila key, parsed once on first use and cached. The PEM is a validated constant
/// (its fingerprint is pinned and asserted by a test), so parsing cannot fail in practice.
static BUNDLED_KEY: LazyLock<WebhookKey> = LazyLock::new(|| {
    WebhookKey::from_pem(UTILA_WEBHOOK_PUBLIC_KEY).expect("bundled Utila webhook key is valid")
});

/// Verify `signature_b64` (the `x-utila-signature` header, base64) over the raw `body` against
/// Utila's bundled public key (parsed once and cached). On success the returned
/// [`VerifiedPayload`] holds the verified bytes; call [`Event::parse`] to decode the typed event.
///
/// # Errors
/// Returns [`ApiError::Auth`] if the signature is malformed or does not verify.
pub fn verify<'a>(body: &'a [u8], signature_b64: &str) -> Result<VerifiedPayload<'a>> {
    verify_with_key(body, signature_b64, &BUNDLED_KEY)
}

/// Like [`verify`] but against a caller-supplied [`WebhookKey`] (testing, or a rotated key).
/// Build the key once with [`WebhookKey::from_pem`] and reuse it.
///
/// # Errors
/// [`ApiError::Auth`] if the signature is malformed or does not verify.
pub fn verify_with_key<'a>(
    body: &'a [u8],
    signature_b64: &str,
    key: &WebhookKey,
) -> Result<VerifiedPayload<'a>> {
    let sig_bytes = B64
        .decode(signature_b64.trim())
        .map_err(|e| ApiError::Auth(format!("invalid webhook signature base64: {e}")))?;
    let signature = Signature::try_from(sig_bytes.as_slice())
        .map_err(|e| ApiError::Auth(format!("malformed webhook signature: {e}")))?;
    #[expect(
        clippy::map_err_ignore,
        reason = "the rsa signature error is deliberately opaque; a uniform failure is the right surface"
    )]
    key.0
        .verify(body, &signature)
        .map_err(|_| ApiError::Auth("webhook signature verification failed".into()))?;
    Ok(VerifiedPayload(body))
}

/// A webhook body whose signature has been verified — a zero-copy view borrowing the bytes you
/// passed to [`verify`]. Get the verified bytes via `AsRef<[u8]>`, and decode the typed event with
/// [`Event::parse`] (parsing is a separate step: an unknown/new event shape can't retroactively
/// invalidate the signature).
#[derive(Debug, Clone, Copy)]
pub struct VerifiedPayload<'a>(&'a [u8]);

impl AsRef<[u8]> for VerifiedPayload<'_> {
    fn as_ref(&self) -> &[u8] {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::pkcs8::{DecodePrivateKey, EncodePublicKey, LineEnding};
    use rsa::pss::SigningKey;
    use rsa::signature::{RandomizedSigner, SignatureEncoding};
    use rsa::RsaPrivateKey;

    /// Load the committed test key and derive a [`WebhookKey`] from its public half (a
    /// *different* key from the bundled Utila key, so it also serves as the "wrong key" in
    /// rejection tests).
    fn test_keypair() -> (RsaPrivateKey, WebhookKey) {
        let pem = std::str::from_utf8(include_bytes!("../tests/test_key.pem")).unwrap();
        let priv_key = RsaPrivateKey::from_pkcs8_pem(pem).unwrap();
        let pub_pem = priv_key
            .to_public_key()
            .to_public_key_pem(LineEnding::LF)
            .unwrap();
        let key = WebhookKey::from_pem(&pub_pem).unwrap();
        (priv_key, key)
    }

    fn sign(priv_key: &RsaPrivateKey, body: &[u8]) -> String {
        let signing_key = SigningKey::<Sha512>::new(priv_key.clone());
        let sig = signing_key.sign_with_rng(&mut rand::thread_rng(), body);
        B64.encode(sig.to_bytes())
    }

    const SAMPLE: &str = r#"{"id":"94ea3ce9","vault":"vaults/abc","type":"TRANSACTION_CREATED","resourceType":"TRANSACTION","resource":"vaults/abc/transactions/t1"}"#;

    #[test]
    fn authentic_event_verifies_and_parses() {
        let (priv_key, key) = test_keypair();
        let sig = sign(&priv_key, SAMPLE.as_bytes());

        let verified = verify_with_key(SAMPLE.as_bytes(), &sig, &key).unwrap();
        assert_eq!(verified.as_ref(), SAMPLE.as_bytes());
        let event = Event::parse(verified).unwrap();
        assert_eq!(
            event,
            Event::TransactionCreated {
                id: "94ea3ce9".into(),
                transaction: crate::resource::TransactionRef {
                    vault: crate::resource::VaultId::new("abc"),
                    transaction: crate::resource::TransactionId::new("t1"),
                },
            }
        );
    }

    #[test]
    fn tampered_body_is_rejected() {
        let (priv_key, key) = test_keypair();
        let sig = sign(&priv_key, SAMPLE.as_bytes());
        let mut tampered = SAMPLE.as_bytes().to_vec();
        tampered.extend_from_slice(b" ");
        let err = verify_with_key(&tampered, &sig, &key).unwrap_err();
        assert!(matches!(err, ApiError::Auth(_)));
    }

    #[test]
    fn verify_uses_the_bundled_key() {
        // `verify` (the default-key wrapper) checks against the bundled Utila key, so a
        // signature made with the test key is rejected — exercising the wrapper + cached key.
        let (priv_key, _key) = test_keypair();
        let sig = sign(&priv_key, SAMPLE.as_bytes());
        assert!(matches!(
            verify(SAMPLE.as_bytes(), &sig),
            Err(ApiError::Auth(_))
        ));
    }

    #[test]
    fn wrong_key_is_rejected() {
        let (priv_key, _key) = test_keypair();
        let sig = sign(&priv_key, SAMPLE.as_bytes());
        // Verify against the bundled Utila key (a different key) → must fail.
        let bundled = WebhookKey::from_pem(UTILA_WEBHOOK_PUBLIC_KEY).unwrap();
        let err = verify_with_key(SAMPLE.as_bytes(), &sig, &bundled).unwrap_err();
        assert!(matches!(err, ApiError::Auth(_)));
    }

    #[test]
    fn malformed_signature_base64_is_rejected() {
        let (_priv, key) = test_keypair();
        let err = verify_with_key(SAMPLE.as_bytes(), "not base64!!!", &key).unwrap_err();
        assert!(matches!(err, ApiError::Auth(_)));
    }

    #[test]
    fn invalid_public_key_is_config_error() {
        let err = WebhookKey::from_pem("not a pem").unwrap_err();
        assert!(matches!(err, ApiError::Config(_)));
    }

    #[test]
    fn bundled_utila_key_is_a_valid_rsa_public_key() {
        WebhookKey::from_pem(UTILA_WEBHOOK_PUBLIC_KEY).expect("bundled Utila webhook key parses");
    }

    #[test]
    fn bundled_key_matches_pinned_fingerprint() {
        use std::fmt::Write as _;

        use rsa::pkcs8::EncodePublicKey;
        use sha2::{Digest, Sha256};
        let der = RsaPublicKey::from_public_key_pem(UTILA_WEBHOOK_PUBLIC_KEY)
            .unwrap()
            .to_public_key_der()
            .unwrap();
        let mut hex = String::new();
        for b in Sha256::digest(der.as_bytes()) {
            write!(hex, "{b:02x}").unwrap();
        }
        // A drifted PEM (rotation, typo) changes this hash — fail here, not in production.
        assert_eq!(hex, UTILA_WEBHOOK_PUBLIC_KEY_SHA256);
    }

    #[test]
    fn verified_but_nonjson_body_fails_to_parse() {
        let (priv_key, key) = test_keypair();
        let body = b"not json";
        let sig = sign(&priv_key, body);
        let verified = verify_with_key(body, &sig, &key).unwrap();
        // Signature is valid, but the payload isn't an event (decode error, not an API error).
        Event::parse(verified).unwrap_err();
    }
}
