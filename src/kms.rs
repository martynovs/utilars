//! Typed, validated KMS key references.
//!
//! A KMS key URL like `awskms:///arn:aws:kms:us-east-1:123:key/abc` carries real
//! structure (cloud, region, account, key id). Rather than pass it around as an opaque
//! string, parse it into a typed [`KmsKey`] whose variant selects the signing backend
//! and whose identifier is validated up front.

use std::ops::Range;

use crate::error::{Result, UtilaError};

/// A KMS key used for remote JWT signing. The cloud backend is explicit; each variant
/// holds a validated identifier, not a raw URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KmsKey {
    /// AWS KMS asymmetric (RSA) signing key.
    Aws(AwsKmsArn),
    // Future backends slot in as additional variants: Gcp(GcpKmsResource), Azure(..).
}

impl KmsKey {
    /// Parse a Tink-style KMS key URL, dispatching on the scheme.
    ///
    /// `awskms:///arn:aws:kms:<region>:<account>:key/<id>` → [`KmsKey::Aws`].
    pub fn parse(url: &str) -> Result<Self> {
        if let Some(rest) = url.strip_prefix("awskms://") {
            // Tink puts the ARN after the scheme, usually with a leading slash.
            let arn = rest.trim_start_matches('/');
            Ok(KmsKey::Aws(AwsKmsArn::parse(arn)?))
        } else {
            Err(UtilaError::Config(format!(
                "unsupported KMS key URL scheme: {url} (expected awskms://)"
            )))
        }
    }
}

impl std::fmt::Display for KmsKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KmsKey::Aws(arn) => write!(f, "awskms:///{}", arn.as_str()),
        }
    }
}

/// A validated AWS KMS key ARN: `arn:aws:kms:<region>:<account>:key/<key-id>`.
///
/// Stores the ARN string once and keeps byte ranges into it for each component, so the
/// accessors borrow slices without extra allocations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AwsKmsArn {
    arn: String,
    region: Range<usize>,
    account_id: Range<usize>,
    key_id: Range<usize>,
}

impl AwsKmsArn {
    /// Parse and validate an `arn:aws:kms:...:key/...` ARN.
    pub fn parse(arn: &str) -> Result<Self> {
        const PREFIX: &str = "arn:aws:kms:";
        let invalid = || {
            UtilaError::Config(format!(
                "invalid AWS KMS ARN: {arn} (expected arn:aws:kms:<region>:<account>:key/<id>)"
            ))
        };
        let rest = arn.strip_prefix(PREFIX).ok_or_else(invalid)?;
        // rest = "<region>:<account>:key/<id>"
        let mut segments = rest.splitn(3, ':');
        let region = segments
            .next()
            .filter(|s| !s.is_empty())
            .ok_or_else(invalid)?;
        let account = segments
            .next()
            .filter(|s| !s.is_empty())
            .ok_or_else(invalid)?;
        let resource = segments.next().ok_or_else(invalid)?;
        let key_id = resource
            .strip_prefix("key/")
            .filter(|s| !s.is_empty())
            .ok_or_else(invalid)?;

        // Byte offsets of each segment within `arn`, derived from segment lengths.
        let region_start = PREFIX.len();
        let account_start = region_start + region.len() + 1;
        let key_id_start = account_start + account.len() + 1 + "key/".len();
        Ok(Self {
            arn: arn.to_string(),
            region: region_start..region_start + region.len(),
            account_id: account_start..account_start + account.len(),
            key_id: key_id_start..key_id_start + key_id.len(),
        })
    }

    pub fn region(&self) -> &str {
        self.arn.get(self.region.clone()).unwrap_or_default()
    }
    pub fn account_id(&self) -> &str {
        self.arn.get(self.account_id.clone()).unwrap_or_default()
    }
    pub fn key_id(&self) -> &str {
        self.arn.get(self.key_id.clone()).unwrap_or_default()
    }

    /// The full ARN as a string slice.
    pub fn as_str(&self) -> &str {
        &self.arn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_aws_kms_url() {
        let key = KmsKey::parse(
            "awskms:///arn:aws:kms:us-east-1:1234567890:key/12345678-1234-1234-1234-123456789012",
        )
        .unwrap();
        let KmsKey::Aws(arn) = key;
        assert_eq!(arn.region(), "us-east-1");
        assert_eq!(arn.account_id(), "1234567890");
        assert_eq!(arn.key_id(), "12345678-1234-1234-1234-123456789012");
    }

    #[test]
    fn rejects_non_aws_scheme() {
        KmsKey::parse("gcpkms://projects/p/locations/l/keyRings/r/cryptoKeys/k").unwrap_err();
    }

    #[test]
    fn rejects_malformed_arn() {
        KmsKey::parse("awskms:///arn:aws:kms:us-east-1:123").unwrap_err();
        KmsKey::parse("awskms:///arn:aws:s3:::bucket").unwrap_err();
    }

    #[test]
    fn as_str_and_display() {
        let key = KmsKey::parse("awskms:///arn:aws:kms:us-east-1:123:key/abc").unwrap();
        let KmsKey::Aws(arn) = &key;
        assert_eq!(arn.as_str(), "arn:aws:kms:us-east-1:123:key/abc");
        assert_eq!(
            key.to_string(),
            "awskms:///arn:aws:kms:us-east-1:123:key/abc"
        );
    }
}
