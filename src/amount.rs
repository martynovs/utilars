use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{Result, UtilaError};

/// An exact, integer amount in an asset's smallest base unit (e.g. wei, satoshi).
///
/// Held as a `u128`, which covers every realistic asset (max ~3.4e38 base units ≈ 340
/// quintillion 18-decimal tokens). Values that don't fit — non-integers, negatives, or
/// adversarial ERC-20 balances near `uint256::MAX` — are rejected at [`Amount::parse`]
/// rather than truncated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Amount(u128);

impl Amount {
    pub const ZERO: Amount = Amount(0);

    /// Parse a base-unit integer string as received from the API. Errors on
    /// non-integers and on values that exceed `u128`.
    pub fn parse(s: &str) -> Result<Self> {
        s.parse::<u128>()
            .map(Amount)
            .map_err(|e| UtilaError::Amount(format!("invalid base-unit amount {s:?}: {e}")))
    }

    /// Wrap a base-unit value you already hold as an integer.
    pub const fn from_base_units(v: u128) -> Self {
        Amount(v)
    }

    /// The exact base-unit value.
    pub const fn value(&self) -> u128 {
        self.0
    }

    /// Project to a human-readable [`Decimal`] given the asset's `decimals`. Errors if
    /// the value exceeds `Decimal`'s ~7.9e28 mantissa (the exact `value()` is still
    /// available).
    pub fn to_decimal(&self, decimals: u32) -> Result<Decimal> {
        let mantissa = i128::try_from(self.0).map_err(|e| {
            UtilaError::Amount(format!(
                "amount {} too large to project to Decimal: {e}",
                self.0
            ))
        })?;
        Decimal::try_from_i128_with_scale(mantissa, decimals)
            .map_err(|e| UtilaError::Amount(format!("does not fit Decimal: {e}")))
    }
}

impl std::fmt::Display for Amount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// The API encodes these integers as JSON *strings* (protojson). Serialize/Deserialize
// as a string so `Amount` can be a field the JSON parser fills and validates directly.
impl Serialize for Amount {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.collect_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Amount {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        if s.is_empty() {
            return Ok(Amount::ZERO); // omitted/empty ⇒ zero
        }
        Amount::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;

    #[test]
    fn parse_roundtrips_and_rejects_junk() {
        let wei = "123456789012345678901234567890"; // 30 digits, fits u128
        let a = Amount::parse(wei).unwrap();
        assert_eq!(a.value(), 123_456_789_012_345_678_901_234_567_890_u128);
        assert_eq!(a.to_string(), wei);

        Amount::parse("1.5").unwrap_err();
        Amount::parse("-1").unwrap_err();
        Amount::parse("abc").unwrap_err();
        // 40-digit adversarial value > u128::MAX
        Amount::parse("1234567890123456789012345678901234567890").unwrap_err();
    }

    #[test]
    fn amount_projects_to_decimal_with_asset_decimals() {
        let a = Amount::parse("1000000000000000000").unwrap();
        assert_eq!(a.to_decimal(18).unwrap(), Decimal::ONE);
        assert_eq!(
            a.to_decimal(6).unwrap(),
            Decimal::from(1_000_000_000_000u64)
        );
    }

    #[test]
    fn decimal_send_amount_serializes_as_json_string() {
        // sanity check that plain Decimal gives the API's string form
        assert_eq!(serde_json::to_string(&dec!(1.5)).unwrap(), "\"1.5\"");
    }

    #[test]
    fn from_base_units_and_serialize() {
        let a = Amount::from_base_units(5);
        assert_eq!(a.value(), 5);
        assert_eq!(serde_json::to_string(&a).unwrap(), "\"5\"");
    }

    #[test]
    fn to_decimal_errors_on_values_that_dont_fit() {
        // > i128::MAX: cannot even be taken as a Decimal mantissa.
        Amount::from_base_units(u128::MAX)
            .to_decimal(0)
            .unwrap_err();
        // Fits i128 but overflows Decimal's ~7.9e28 mantissa (1e30 here).
        Amount::parse("1000000000000000000000000000000")
            .unwrap()
            .to_decimal(0)
            .unwrap_err();
    }

    #[test]
    fn deserializes_from_json_string() {
        let a: Amount = serde_json::from_str("\"42\"").unwrap();
        assert_eq!(a.value(), 42);
        // protojson omits zero-valued fields ⇒ empty string decodes to zero
        let zero: Amount = serde_json::from_str("\"\"").unwrap();
        assert_eq!(zero, Amount::ZERO);
        serde_json::from_str::<Amount>("\"bad\"").unwrap_err();
    }
}
