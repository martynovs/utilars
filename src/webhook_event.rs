//! Typed Utila webhook events — the payload decoded from a verified webhook body.
//!
//! [`crate::webhook::verify`] returns a [`crate::webhook::VerifiedPayload`]; [`Event::parse`]
//! decodes it into an [`Event`]. An event's `type`, `resourceType`, and `resource` are all
//! correlated (a `TRANSACTION_*` event always refers to a transaction, etc.), so they're modeled
//! as a single enum: each variant carries the ids and detail that its kind implies. A `resource`
//! that doesn't match its `type` is a decode error rather than a representable-but-invalid state.

use serde::Deserialize;

use crate::resource::{AddressId, ParseRef, TransactionId, VaultId, WalletId};
use crate::resource::{TransactionRef, VaultRef, WalletAddressRef, WalletRef};

/// A Utila webhook event. Every variant carries the envelope `id`; resource-bearing kinds carry
/// a typed resource reference ([`TransactionRef`]/[`WalletRef`]/[`WalletAddressRef`], each holding
/// the vault), and detail-bearing kinds their decoded detail.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Event {
    /// `TRANSACTION_CREATED`.
    TransactionCreated {
        id: String,
        transaction: TransactionRef,
    },
    /// `TRANSACTION_STATE_UPDATED`.
    TransactionStateUpdated {
        id: String,
        transaction: TransactionRef,
        new_state: TransactionState,
    },
    /// `TRANSACTION_AML_SCREENING_RESULT_READY`.
    TransactionAmlScreeningResultReady {
        id: String,
        transaction: TransactionRef,
        action: AmlAction,
    },
    /// `WALLET_CREATED`.
    WalletCreated { id: String, wallet: WalletRef },
    /// `WALLET_ADDRESS_CREATED`.
    WalletAddressCreated {
        id: String,
        address: WalletAddressRef,
    },
    /// `TEST`.
    Test { id: String, vault: VaultId },
    /// An event type not known to this crate version. Only the envelope `id` + `vault` are
    /// surfaced (keep `id` for idempotent dedup); the raw `type`/`resource`/details aren't copied
    /// here — the caller still holds the payload it parsed for logging/inspection.
    Unknown { id: String, vault: VaultId },
}

impl Event {
    /// Decode a webhook body into a typed [`Event`]. Accepts anything byte-like — in particular a
    /// [`crate::webhook::VerifiedPayload`], which impls `AsRef<[u8]>`, so a verified body parses
    /// directly: `Event::parse(verified)`.
    ///
    /// # Errors
    /// The [`serde_json::Error`] if the body is not a valid event payload. (Decoding is not an API
    /// call, so this is the parser's own error, not a [`crate::ApiError`].)
    pub fn parse(body: impl AsRef<[u8]>) -> serde_json::Result<Self> {
        serde_json::from_slice(body.as_ref())
    }
}

impl<'de> Deserialize<'de> for Event {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // `type`, `resourceType`, and `resource` arrive as separate top-level fields; fold them
        // into the correlated `Event` variant here. Borrow the string fields from the input
        // (webhook fields are escape-free slugs/paths); allocate only what `Event` keeps.
        #[derive(Deserialize)]
        struct Raw<'a> {
            id: &'a str,
            vault: &'a str,
            #[serde(rename = "type")]
            r#type: &'a str,
            #[serde(default, rename = "resourceType")]
            resource_type: Option<&'a str>,
            #[serde(default)]
            resource: Option<&'a str>,
            #[serde(default)]
            details: Option<serde_json::Map<String, serde_json::Value>>,
        }
        let raw = Raw::deserialize(deserializer)?;
        let id = raw.id.to_owned();
        let vault =
            VaultRef::parse(raw.vault).map_or_else(|| VaultId::new(raw.vault), VaultId::from);
        let rty = raw.resource_type;
        let resource = raw.resource;
        let details = raw.details.as_ref();
        // Borrow `vault` for the resource checks before moving it into the variant. The `*_of`
        // helpers also validate `resourceType` (when present) against the kind.
        // Each arm consumes `vault` exactly once (moved into the ref/variant — no clone).
        Ok(match raw.r#type {
            "TRANSACTION_CREATED" => Event::TransactionCreated {
                id,
                transaction: transaction_of(vault, rty, resource)?,
            },
            "TRANSACTION_STATE_UPDATED" => {
                let transaction = transaction_of(vault, rty, resource)?;
                let new_state = take_detail_field(details, "transactionStateUpdated", "newState")?;
                Event::TransactionStateUpdated {
                    id,
                    transaction,
                    new_state,
                }
            }
            "TRANSACTION_AML_SCREENING_RESULT_READY" => {
                let transaction = transaction_of(vault, rty, resource)?;
                let action =
                    take_detail_field(details, "transactionAmlScreeningResultReady", "action")?;
                Event::TransactionAmlScreeningResultReady {
                    id,
                    transaction,
                    action,
                }
            }
            "WALLET_CREATED" => Event::WalletCreated {
                id,
                wallet: wallet_of(vault, rty, resource)?,
            },
            "WALLET_ADDRESS_CREATED" => Event::WalletAddressCreated {
                id,
                address: wallet_address_of(vault, rty, resource)?,
            },
            "TEST" => Event::Test { id, vault },
            _ => Event::Unknown { id, vault },
        })
    }
}

/// Split a `resource` path into its `/`-separated segments (`None` if the field was absent).
fn segments(resource: Option<&str>) -> Option<Vec<&str>> {
    resource.map(|r| r.split('/').collect())
}

/// Validate the wire `resourceType` (when present) against the kind's expected value.
fn expect_type<E: serde::de::Error>(expected: &str, got: Option<&str>) -> Result<(), E> {
    match got {
        Some(t) if t != expected => Err(E::custom(format!(
            "resourceType `{t}` does not match the event kind (expected `{expected}`)"
        ))),
        _ => Ok(()),
    }
}

/// Error for a `resource` that doesn't match the shape/vault its event requires. `expected` is the
/// canonical pattern (with the event's vault already filled in by the caller).
fn resource_mismatch<E: serde::de::Error>(expected: &str, got: Option<&str>) -> E {
    E::custom(format!(
        "webhook resource {:?} does not match the event (expected `{expected}`)",
        got.unwrap_or("<absent>"),
    ))
}

/// `vaults/{vault}/transactions/{transaction}` → [`TransactionId`]. The path's vault must equal the
/// event's `vault`.
fn transaction_of<E: serde::de::Error>(
    vault: VaultId,
    resource_type: Option<&str>,
    resource: Option<&str>,
) -> Result<TransactionRef, E> {
    expect_type("TRANSACTION", resource_type)?;
    if let Some(["vaults", v, "transactions", tx]) = segments(resource).as_deref() {
        if *v == vault.as_str() {
            return Ok(TransactionRef {
                vault,
                transaction: TransactionId::new(*tx),
            });
        }
    }
    Err(resource_mismatch(
        &format!("vaults/{vault}/transactions/{{txId}}"),
        resource,
    ))
}

/// `vaults/{vault}/wallets/{wallet}` → [`WalletRef`]. The path's vault must equal the event's `vault`.
fn wallet_of<E: serde::de::Error>(
    vault: VaultId,
    resource_type: Option<&str>,
    resource: Option<&str>,
) -> Result<WalletRef, E> {
    expect_type("WALLET", resource_type)?;
    if let Some(["vaults", v, "wallets", wallet]) = segments(resource).as_deref() {
        if *v == vault.as_str() {
            return Ok(WalletRef {
                vault,
                wallet: WalletId::new(*wallet),
            });
        }
    }
    Err(resource_mismatch(
        &format!("vaults/{vault}/wallets/{{walletId}}"),
        resource,
    ))
}

/// `vaults/{vault}/wallets/{wallet}/addresses/{address}` → [`WalletAddressRef`]. The path's vault
/// must equal the event's `vault`.
fn wallet_address_of<E: serde::de::Error>(
    vault: VaultId,
    resource_type: Option<&str>,
    resource: Option<&str>,
) -> Result<WalletAddressRef, E> {
    expect_type("WALLET_ADDRESS", resource_type)?;
    if let Some(["vaults", v, "wallets", wallet, "addresses", address]) =
        segments(resource).as_deref()
    {
        if *v == vault.as_str() {
            return Ok(WalletAddressRef {
                vault,
                wallet: WalletId::new(*wallet),
                address: AddressId::new(*address),
            });
        }
    }
    Err(resource_mismatch(
        &format!("vaults/{vault}/wallets/{{walletId}}/addresses/{{addressId}}"),
        resource,
    ))
}

/// Pull `details.<outer>.<inner>` out and deserialize that inner value into `T`. The documented
/// detail object and its field are always present for the kinds that carry one, so a missing
/// object, missing field, or wrong-shaped value is a hard error (the event is malformed).
fn take_detail_field<T, E>(
    details: Option<&serde_json::Map<String, serde_json::Value>>,
    outer: &str,
    inner: &str,
) -> Result<T, E>
where
    T: for<'d> Deserialize<'d>,
    E: serde::de::Error,
{
    let value = details
        .and_then(|m| m.get(outer))
        .and_then(|d| d.get(inner))
        .ok_or_else(|| E::custom(format!("missing webhook detail field `{outer}.{inner}`")))?;
    // Keep serde's "what was unexpected" (e.g. `invalid type: integer, expected a string`) but
    // prefix the JSON path so the error always says *where* the bad value is.
    T::deserialize(value).map_err(|e| {
        E::custom(format!(
            "invalid webhook detail field `{outer}.{inner}`: {e}"
        ))
    })
}

/// Defines a `#[non_exhaustive]` wire enum: known variants map from their server strings, and any
/// unrecognized value becomes the unit `Unknown` rather than failing deserialization — so a
/// server-side addition can't break a deployed receiver. The raw string isn't stored (the caller
/// holds the payload for logging). Conversions from the generated `V2*Enum` are written by hand
/// below (a direct variant-to-variant match).
macro_rules! wire_enum {
    ($(#[$m:meta])* $name:ident { $($variant:ident => $wire:literal,)+ }) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Eq)]
        #[non_exhaustive]
        pub enum $name {
            $(
                #[doc = concat!("Wire value `", $wire, "`.")]
                $variant,
            )+
            /// A value not known to this version of the crate.
            Unknown,
        }

        impl ::std::convert::From<&str> for $name {
            fn from(s: &str) -> Self {
                match s {
                    $($wire => Self::$variant,)+
                    _ => Self::Unknown,
                }
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D>(d: D) -> ::std::result::Result<Self, D::Error>
            where
                D: ::serde::Deserializer<'de>,
            {
                let s = <::std::string::String as ::serde::Deserialize>::deserialize(d)?;
                Ok(Self::from(s.as_str()))
            }
        }
    };
}

wire_enum! {
    /// A transaction's lifecycle state (mirrors the API's `v2TransactionStateEnum`).
    TransactionState {
        AwaitingApproval => "AWAITING_APPROVAL",
        AwaitingAmlPolicyCheck => "AWAITING_AML_POLICY_CHECK",
        DeclinedByAmlPolicy => "DECLINED_BY_AML_POLICY",
        AwaitingPolicyCheck => "AWAITING_POLICY_CHECK",
        AwaitingSignature => "AWAITING_SIGNATURE",
        Signed => "SIGNED",
        AwaitingPublish => "AWAITING_PUBLISH",
        Published => "PUBLISHED",
        Mined => "MINED",
        MinedFailed => "MINED_FAILED",
        Failed => "FAILED",
        Declined => "DECLINED",
        Replaced => "REPLACED",
        Canceled => "CANCELED",
        Dropped => "DROPPED",
        Confirmed => "CONFIRMED",
        Expired => "EXPIRED",
    }
}

wire_enum! {
    /// An AML screening decision (mirrors the API's `v2AMLActionEnum`).
    AmlAction {
        Deny => "DENY",
        Allow => "ALLOW",
        Alert => "ALERT",
    }
}

impl From<crate::generated::types::V2TransactionStateEnum> for TransactionState {
    fn from(v: crate::generated::types::V2TransactionStateEnum) -> Self {
        use crate::generated::types::V2TransactionStateEnum as G;
        match v {
            G::AwaitingApproval => Self::AwaitingApproval,
            G::AwaitingAmlPolicyCheck => Self::AwaitingAmlPolicyCheck,
            G::DeclinedByAmlPolicy => Self::DeclinedByAmlPolicy,
            G::AwaitingPolicyCheck => Self::AwaitingPolicyCheck,
            G::AwaitingSignature => Self::AwaitingSignature,
            G::Signed => Self::Signed,
            G::AwaitingPublish => Self::AwaitingPublish,
            G::Published => Self::Published,
            G::Mined => Self::Mined,
            G::MinedFailed => Self::MinedFailed,
            G::Failed => Self::Failed,
            G::Declined => Self::Declined,
            G::Replaced => Self::Replaced,
            G::Canceled => Self::Canceled,
            G::Dropped => Self::Dropped,
            G::Confirmed => Self::Confirmed,
            G::Expired => Self::Expired,
            // Proto unspecified sentinel → the macro's forward-compat catch-all.
            G::EnumUnspecified => Self::Unknown,
        }
    }
}

impl From<crate::generated::types::V2AmlActionEnum> for AmlAction {
    fn from(v: crate::generated::types::V2AmlActionEnum) -> Self {
        use crate::generated::types::V2AmlActionEnum as G;
        match v {
            G::Deny => Self::Deny,
            G::Allow => Self::Allow,
            G::Alert => Self::Alert,
            // Proto unspecified sentinel → the macro's forward-compat catch-all.
            G::EnumUnspecified => Self::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decode an `Event` straight from JSON — signature verification (covered in `webhook.rs`) is
    /// orthogonal to the typed decoding exercised here.
    fn parse(body: &str) -> Event {
        Event::parse(body).unwrap()
    }

    #[test]
    fn parses_full_events() {
        assert_eq!(
            parse(
                r#"{"id":"e1","vault":"vaults/v1","type":"TRANSACTION_CREATED","resourceType":"TRANSACTION","resource":"vaults/v1/transactions/t1"}"#
            ),
            Event::TransactionCreated {
                id: "e1".into(),
                transaction: TransactionRef {
                    vault: VaultId::new("v1"),
                    transaction: TransactionId::new("t1"),
                },
            }
        );
        assert_eq!(
            parse(
                r#"{"id":"e2","vault":"vaults/v1","type":"TRANSACTION_STATE_UPDATED","resource":"vaults/v1/transactions/t1","details":{"transactionStateUpdated":{"newState":"CONFIRMED"}}}"#
            ),
            Event::TransactionStateUpdated {
                id: "e2".into(),
                transaction: TransactionRef {
                    vault: VaultId::new("v1"),
                    transaction: TransactionId::new("t1"),
                },
                new_state: TransactionState::Confirmed,
            }
        );
        assert_eq!(
            parse(
                r#"{"id":"e3","vault":"vaults/v1","type":"TRANSACTION_AML_SCREENING_RESULT_READY","resource":"vaults/v1/transactions/t1","details":{"transactionAmlScreeningResultReady":{"action":"ALLOW"}}}"#
            ),
            Event::TransactionAmlScreeningResultReady {
                id: "e3".into(),
                transaction: TransactionRef {
                    vault: VaultId::new("v1"),
                    transaction: TransactionId::new("t1"),
                },
                action: AmlAction::Allow,
            }
        );
        assert_eq!(
            parse(
                r#"{"id":"e4","vault":"vaults/v1","type":"WALLET_CREATED","resource":"vaults/v1/wallets/w1"}"#
            ),
            Event::WalletCreated {
                id: "e4".into(),
                wallet: WalletRef {
                    vault: VaultId::new("v1"),
                    wallet: WalletId::new("w1"),
                },
            }
        );
        assert_eq!(
            parse(
                r#"{"id":"e5","vault":"vaults/v1","type":"WALLET_ADDRESS_CREATED","resource":"vaults/v1/wallets/w1/addresses/a1"}"#
            ),
            Event::WalletAddressCreated {
                id: "e5".into(),
                address: WalletAddressRef {
                    vault: VaultId::new("v1"),
                    wallet: WalletId::new("w1"),
                    address: AddressId::new("a1"),
                },
            }
        );
        assert_eq!(
            parse(r#"{"id":"e6","vault":"vaults/v1","type":"TEST"}"#),
            Event::Test {
                id: "e6".into(),
                vault: VaultId::new("v1"),
            }
        );
    }

    #[test]
    fn unknown_event_type_is_captured() {
        // Unrecognized type → `Unknown` with just the envelope (id kept for dedup); the raw
        // type/resource stay in the caller's payload. A bad-shaped resource doesn't error here
        // because an unknown kind imposes no resource format.
        assert_eq!(
            parse(
                r#"{"id":"e7","vault":"vaults/v1","type":"POLICY_UPDATED","resource":"vaults/v1/policies/p1"}"#
            ),
            Event::Unknown {
                id: "e7".into(),
                vault: VaultId::new("v1"),
            }
        );
    }

    #[test]
    fn resource_must_match_kind() {
        // A known kind whose `resource` is the wrong shape (or absent) is a decode error, so an
        // illegal kind/resource pairing is never representable. Covers every `*_of` error arm.
        for body in [
            // transaction kind, wallet resource
            r#"{"id":"1","vault":"v","type":"TRANSACTION_CREATED","resource":"vaults/v/wallets/w"}"#,
            // transaction kind, no resource
            r#"{"id":"1","vault":"v","type":"TRANSACTION_CREATED"}"#,
            // right shape, but the resource's vault differs from the event's `vault`
            r#"{"id":"1","vault":"v","type":"TRANSACTION_CREATED","resource":"vaults/other/transactions/t"}"#,
            // resourceType contradicts the event kind
            r#"{"id":"1","vault":"v","type":"TRANSACTION_CREATED","resourceType":"WALLET","resource":"vaults/v/transactions/t"}"#,
            // wallet kind, transaction resource
            r#"{"id":"1","vault":"v","type":"WALLET_CREATED","resource":"vaults/v/transactions/t"}"#,
            // wallet kind, right shape but mismatched vault
            r#"{"id":"1","vault":"v","type":"WALLET_CREATED","resource":"vaults/other/wallets/w"}"#,
            // wallet-address kind, wallet (too-short) resource
            r#"{"id":"1","vault":"v","type":"WALLET_ADDRESS_CREATED","resource":"vaults/v/wallets/w"}"#,
            // wallet-address kind, right shape but mismatched vault
            r#"{"id":"1","vault":"v","type":"WALLET_ADDRESS_CREATED","resource":"vaults/other/wallets/w/addresses/a"}"#,
        ] {
            assert!(serde_json::from_str::<Event>(body).is_err(), "body: {body}");
        }
    }

    #[test]
    fn missing_or_malformed_detail_is_error() {
        // Detail-bearing kinds with a valid resource but absent/wrong-shaped detail are decode
        // errors. Covers both `take_detail_field` error arms for both kinds.
        for body in [
            r#"{"id":"1","vault":"v","type":"TRANSACTION_STATE_UPDATED","resource":"vaults/v/transactions/t"}"#,
            r#"{"id":"1","vault":"v","type":"TRANSACTION_AML_SCREENING_RESULT_READY","resource":"vaults/v/transactions/t"}"#,
            r#"{"id":"1","vault":"v","type":"TRANSACTION_STATE_UPDATED","resource":"vaults/v/transactions/t","details":{"transactionStateUpdated":123}}"#,
            r#"{"id":"1","vault":"v","type":"TRANSACTION_AML_SCREENING_RESULT_READY","resource":"vaults/v/transactions/t","details":{"transactionAmlScreeningResultReady":123}}"#,
        ] {
            assert!(serde_json::from_str::<Event>(body).is_err(), "body: {body}");
        }
    }

    #[test]
    fn detail_errors_name_the_offending_path() {
        // Missing inner field → the error names the full `outer.inner` path.
        let err = serde_json::from_str::<Event>(
            r#"{"id":"1","vault":"v","type":"TRANSACTION_STATE_UPDATED","resource":"vaults/v/transactions/t","details":{"transactionStateUpdated":{}}}"#,
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("transactionStateUpdated.newState"),
            "missing-field error should name the path, got: {err}"
        );

        // Wrong-typed inner value → the error names the path *and* what was unexpected.
        let err = serde_json::from_str::<Event>(
            r#"{"id":"1","vault":"v","type":"TRANSACTION_AML_SCREENING_RESULT_READY","resource":"vaults/v/transactions/t","details":{"transactionAmlScreeningResultReady":{"action":123}}}"#,
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("transactionAmlScreeningResultReady.action"),
            "wrong-type error should name the path, got: {err}"
        );
        assert!(
            err.contains("integer"),
            "wrong-type error should name the unexpected value, got: {err}"
        );
    }

    #[test]
    fn wire_enums_map_every_known_value_and_capture_unknown() {
        let states = [
            ("AWAITING_APPROVAL", TransactionState::AwaitingApproval),
            (
                "AWAITING_AML_POLICY_CHECK",
                TransactionState::AwaitingAmlPolicyCheck,
            ),
            (
                "DECLINED_BY_AML_POLICY",
                TransactionState::DeclinedByAmlPolicy,
            ),
            (
                "AWAITING_POLICY_CHECK",
                TransactionState::AwaitingPolicyCheck,
            ),
            ("AWAITING_SIGNATURE", TransactionState::AwaitingSignature),
            ("SIGNED", TransactionState::Signed),
            ("AWAITING_PUBLISH", TransactionState::AwaitingPublish),
            ("PUBLISHED", TransactionState::Published),
            ("MINED", TransactionState::Mined),
            ("MINED_FAILED", TransactionState::MinedFailed),
            ("FAILED", TransactionState::Failed),
            ("DECLINED", TransactionState::Declined),
            ("REPLACED", TransactionState::Replaced),
            ("CANCELED", TransactionState::Canceled),
            ("DROPPED", TransactionState::Dropped),
            ("CONFIRMED", TransactionState::Confirmed),
            ("EXPIRED", TransactionState::Expired),
        ];
        for (wire, expected) in states {
            assert_eq!(TransactionState::from(wire), expected, "{wire}");
        }
        assert_eq!(
            TransactionState::from("SOMETHING_NEW"),
            TransactionState::Unknown
        );

        let actions = [
            ("DENY", AmlAction::Deny),
            ("ALLOW", AmlAction::Allow),
            ("ALERT", AmlAction::Alert),
        ];
        for (wire, expected) in actions {
            assert_eq!(AmlAction::from(wire), expected, "{wire}");
        }
        assert_eq!(AmlAction::from("FUTURE"), AmlAction::Unknown);
    }

    #[test]
    fn from_generated_enums_maps_to_curated() {
        use crate::generated::types::{V2AmlActionEnum as G, V2TransactionStateEnum as S};
        let states = [
            (S::AwaitingApproval, TransactionState::AwaitingApproval),
            (
                S::AwaitingAmlPolicyCheck,
                TransactionState::AwaitingAmlPolicyCheck,
            ),
            (
                S::DeclinedByAmlPolicy,
                TransactionState::DeclinedByAmlPolicy,
            ),
            (
                S::AwaitingPolicyCheck,
                TransactionState::AwaitingPolicyCheck,
            ),
            (S::AwaitingSignature, TransactionState::AwaitingSignature),
            (S::Signed, TransactionState::Signed),
            (S::AwaitingPublish, TransactionState::AwaitingPublish),
            (S::Published, TransactionState::Published),
            (S::Mined, TransactionState::Mined),
            (S::MinedFailed, TransactionState::MinedFailed),
            (S::Failed, TransactionState::Failed),
            (S::Declined, TransactionState::Declined),
            (S::Replaced, TransactionState::Replaced),
            (S::Canceled, TransactionState::Canceled),
            (S::Dropped, TransactionState::Dropped),
            (S::Confirmed, TransactionState::Confirmed),
            (S::Expired, TransactionState::Expired),
            // Proto unspecified sentinel maps to the forward-compat catch-all.
            (S::EnumUnspecified, TransactionState::Unknown),
        ];
        for (gen, expected) in states {
            assert_eq!(TransactionState::from(gen), expected);
        }
        let actions = [
            (G::Deny, AmlAction::Deny),
            (G::Allow, AmlAction::Allow),
            (G::Alert, AmlAction::Alert),
            (G::EnumUnspecified, AmlAction::Unknown),
        ];
        for (gen, expected) in actions {
            assert_eq!(AmlAction::from(gen), expected);
        }
    }
}
