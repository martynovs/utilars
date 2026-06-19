//! Typed resource identifiers.
//!
//! Distinct newtypes so a wallet id can't be passed where a vault id is expected.
//! Each is `AsRef<str>` / `From<String>` / `Display`, so it drops into request URLs
//! with no ceremony.

macro_rules! id_newtype {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }
        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_string())
            }
        }
        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_newtype!(
    /// A vault id — the segment after `vaults/`, e.g. `1b25635a5b3f`.
    VaultId
);
id_newtype!(
    /// A wallet id — the segment after `wallets/`.
    WalletId
);
id_newtype!(
    /// An asset resource name, e.g. `assets/native.ethereum-mainnet`.
    AssetId
);
id_newtype!(
    /// A wallet address id.
    AddressId
);
id_newtype!(
    /// A network id — the segment after `networks/`, e.g. `ethereum-mainnet`.
    NetworkId
);
id_newtype!(
    /// A transaction id — the segment after `transactions/`.
    TransactionId
);
id_newtype!(
    /// An address-book entry resource name, e.g. `vaults/{v}/addressBookEntries/{id}`.
    AddressBookEntryId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newtype_display_and_as_str() {
        assert_eq!(VaultId::new("v1").to_string(), "v1");
        assert_eq!(AssetId::from("assets/x").as_str(), "assets/x");
        let id = VaultId::new("v1");
        let r: &str = id.as_ref();
        assert_eq!(r, "v1");
    }
}
