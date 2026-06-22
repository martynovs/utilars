//! Typed resource identifiers and references.
//!
//! The `*Id` newtypes are distinct so a wallet id can't be passed where a vault id is expected;
//! each is `AsRef<str>` / `From<String>` / `Display`. The `*Ref` composites are `vault`-scoped
//! resource references shared across the API surface (transfer endpoints) and webhook events; each
//! `Display`s as its canonical resource name (`vaults/{vault}/…`), the wire form.

use crate::macros::{id_newtype, prefix_ref, vault_ref};

id_newtype!(
    /// A vault id — the segment after `vaults/`, e.g. `1b25635a5b3f`.
    VaultId
);
id_newtype!(
    /// A wallet id — the segment after `wallets/`.
    WalletId
);
id_newtype!(
    /// An asset id — the bare segment after `assets/`, e.g. `native.ethereum-mainnet`. Assets are
    /// global; the full `assets/{id}` resource name is built via [`AssetRef`], never stored here.
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
    /// An address-book entry id — the bare segment after `addressBookEntries/`. The full
    /// `vaults/{v}/addressBookEntries/{id}` name is built via [`AddressBookEntryRef`].
    AddressBookEntryId
);
id_newtype!(
    /// A gas-station id — the segment after `gasStations/`.
    GasStationId
);
id_newtype!(
    /// A transaction-request id — the segment after `transactionRequests/`.
    TransactionRequestId
);
id_newtype!(
    /// A transaction-simulation id — the segment after `transactionSimulations/`.
    SimulationId
);
id_newtype!(
    /// A user id — the segment after `users/`.
    UserId
);
id_newtype!(
    /// A vault-action id — the segment after `actions/`.
    VaultActionId
);
id_newtype!(
    /// An address-book-entry-group id — the bare segment after `addressBookEntryGroups/`.
    AddressBookEntryGroupId
);

/// A typed resource reference parseable from its canonical resource name.
pub trait ParseRef: Sized {
    /// Parse the resource name into the typed reference, or `None` if it doesn't match the shape.
    fn parse(resource_name: &str) -> Option<Self>;
}

/// A resource name as returned by the API: the parsed typed reference `R` when it matches the
/// canonical shape, otherwise the raw string verbatim. The API always sends the canonical form, so
/// `Raw` is a forward-compatible fallback (no value is ever lost or faked). `Display`s identically
/// either way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceName<R> {
    Ref(R),
    Raw(String),
}

impl<R: ParseRef> ResourceName<R> {
    /// Classify a resource name: parse it to the typed `R`, or keep it raw.
    pub fn parse(name: impl Into<String>) -> Self {
        let name = name.into();
        match R::parse(&name) {
            Some(r) => Self::Ref(r),
            None => Self::Raw(name),
        }
    }
}

impl<R> ResourceName<R> {
    /// The contained reference, if the name parsed.
    #[must_use]
    pub fn as_ref(&self) -> Option<&R> {
        match self {
            Self::Ref(r) => Some(r),
            Self::Raw(_) => None,
        }
    }
}

impl<R: std::fmt::Display> std::fmt::Display for ResourceName<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ref(r) => r.fmt(f),
            Self::Raw(s) => f.write_str(s),
        }
    }
}

vault_ref!(
    /// A transaction: `vaults/{vault}/transactions/{transaction}`.
    TransactionRef, "transactions", transaction: TransactionId
);
vault_ref!(
    /// A wallet: `vaults/{vault}/wallets/{wallet}`.
    WalletRef, "wallets", wallet: WalletId
);
vault_ref!(
    /// A gas station: `vaults/{vault}/gasStations/{gas_station}`.
    GasStationRef, "gasStations", gas_station: GasStationId
);
vault_ref!(
    /// An address-book entry: `vaults/{vault}/addressBookEntries/{entry}`.
    AddressBookEntryRef, "addressBookEntries", entry: AddressBookEntryId
);
vault_ref!(
    /// A transaction request: `vaults/{vault}/transactionRequests/{request}`.
    TransactionRequestRef, "transactionRequests", request: TransactionRequestId
);
vault_ref!(
    /// A transaction simulation: `vaults/{vault}/transactionSimulations/{simulation}`.
    SimulationRef, "transactionSimulations", simulation: SimulationId
);
vault_ref!(
    /// A pending vault (quorum) action: `vaults/{vault}/actions/{action}`.
    VaultActionRef, "actions", action: VaultActionId
);
vault_ref!(
    /// An address-book entry group: `vaults/{vault}/addressBookEntryGroups/{group}`.
    AddressBookEntryGroupRef, "addressBookEntryGroups", group: AddressBookEntryGroupId
);

prefix_ref!(
    /// An asset: `assets/{asset}`. Assets are global (never vault-scoped), so this single-prefix
    /// ref is the canonical full name; the bridge re-adds/strips the `assets/` prefix.
    AssetRef, "assets", asset: AssetId
);
prefix_ref!(
    /// A network: `networks/{network}`.
    NetworkRef, "networks", network: NetworkId
);
prefix_ref!(
    /// A vault: `vaults/{vault}`.
    VaultRef, "vaults", vault: VaultId
);
prefix_ref!(
    /// A user: `users/{user}`.
    UserRef, "users", user: UserId
);

impl UserRef {
    /// Build a user reference from the segment after `users/` (a user id or an email).
    pub fn new(user: impl Into<UserId>) -> Self {
        Self { user: user.into() }
    }
}

/// A specific wallet address: `vaults/{vault}/wallets/{wallet}/addresses/{address}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletAddressRef {
    pub vault: VaultId,
    pub wallet: WalletId,
    pub address: AddressId,
}

impl WalletAddressRef {
    /// The full `vaults/{vault}/wallets/{wallet}/addresses/{address}` resource name, from borrowed
    /// parts — the no-clone builder (uniform with the macro-generated refs).
    #[must_use]
    pub fn resource_name(vault: &VaultId, wallet: &WalletId, address: &AddressId) -> String {
        format!("vaults/{vault}/wallets/{wallet}/addresses/{address}")
    }
}

impl ParseRef for WalletAddressRef {
    fn parse(resource_name: &str) -> Option<Self> {
        match resource_name.split('/').collect::<Vec<_>>().as_slice() {
            ["vaults", v, "wallets", w, "addresses", a] => Some(Self {
                vault: VaultId::new(*v),
                wallet: WalletId::new(*w),
                address: AddressId::new(*a),
            }),
            _ => None,
        }
    }
}

impl std::fmt::Display for WalletAddressRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&Self::resource_name(
            &self.vault,
            &self.wallet,
            &self.address,
        ))
    }
}

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

    #[test]
    fn resource_name_parses_ref_or_keeps_raw() {
        let parsed: ResourceName<TransactionRef> = ResourceName::parse("vaults/v/transactions/t");
        assert_eq!(
            parsed,
            ResourceName::Ref(TransactionRef {
                vault: VaultId::new("v"),
                transaction: TransactionId::new("t"),
            })
        );
        assert!(parsed.as_ref().is_some());
        assert_eq!(parsed.to_string(), "vaults/v/transactions/t");

        let raw: ResourceName<TransactionRef> = ResourceName::parse("not-a-name");
        assert_eq!(raw, ResourceName::Raw("not-a-name".to_string()));
        assert!(raw.as_ref().is_none());
        assert_eq!(raw.to_string(), "not-a-name");
    }

    #[test]
    fn single_prefix_ref_bridge_round_trips() {
        // Emit: a bare id becomes the full `{prefix}/{id}` wire name (one prefix, no doubling).
        assert_eq!(
            AssetRef::resource_name(&AssetId::new("native.ethereum-mainnet")),
            "assets/native.ethereum-mainnet"
        );
        assert_eq!(
            NetworkRef::resource_name(&NetworkId::new("ethereum-mainnet")),
            "networks/ethereum-mainnet"
        );

        // Ingest: parse a prefixed wire name into a bare id via `Into`.
        let asset: AssetId = AssetRef::parse("assets/native.ethereum-mainnet")
            .unwrap()
            .into();
        assert_eq!(asset.as_str(), "native.ethereum-mainnet");
        let network: NetworkId = NetworkRef::parse("networks/ethereum-mainnet")
            .unwrap()
            .into();
        assert_eq!(network.as_str(), "ethereum-mainnet");
        let vault: VaultId = VaultRef::parse("vaults/abc").unwrap().into();
        assert_eq!(vault.as_str(), "abc");
        let user: UserId = UserRef::parse("users/u").unwrap().into();
        assert_eq!(user.as_str(), "u");

        // A value with no prefix does not parse as a resource name.
        assert!(AssetRef::parse("native.ethereum-mainnet").is_none());
    }

    #[test]
    fn every_ref_builds_its_resource_name_from_borrows() {
        let v = VaultId::new("v");
        // Vault-scoped refs.
        assert_eq!(
            TransactionRef::resource_name(&v, &TransactionId::new("t")),
            "vaults/v/transactions/t"
        );
        assert_eq!(
            WalletRef::resource_name(&v, &WalletId::new("w")),
            "vaults/v/wallets/w"
        );
        assert_eq!(
            GasStationRef::resource_name(&v, &GasStationId::new("g")),
            "vaults/v/gasStations/g"
        );
        assert_eq!(
            AddressBookEntryRef::resource_name(&v, &AddressBookEntryId::new("e")),
            "vaults/v/addressBookEntries/e"
        );
        assert_eq!(
            TransactionRequestRef::resource_name(&v, &TransactionRequestId::new("r")),
            "vaults/v/transactionRequests/r"
        );
        assert_eq!(
            SimulationRef::resource_name(&v, &SimulationId::new("s")),
            "vaults/v/transactionSimulations/s"
        );
        assert_eq!(
            VaultActionRef::resource_name(&v, &VaultActionId::new("a")),
            "vaults/v/actions/a"
        );
        assert_eq!(
            WalletAddressRef::resource_name(&v, &WalletId::new("w"), &AddressId::new("a")),
            "vaults/v/wallets/w/addresses/a"
        );
        // Single-prefix refs (Asset/Network covered in the bridge round-trip test).
        assert_eq!(VaultRef::resource_name(&v), "vaults/v");
        assert_eq!(UserRef::resource_name(&UserId::new("u")), "users/u");
    }

    #[test]
    fn new_refs_round_trip() {
        assert_eq!(
            NetworkRef::parse("networks/eth").unwrap().to_string(),
            "networks/eth"
        );
        assert_eq!(VaultRef::parse("vaults/v").unwrap().to_string(), "vaults/v");
        assert_eq!(UserRef::parse("users/u").unwrap().to_string(), "users/u");
        assert_eq!(
            AddressBookEntryRef::parse("vaults/v/addressBookEntries/e")
                .unwrap()
                .to_string(),
            "vaults/v/addressBookEntries/e"
        );
        assert_eq!(
            WalletAddressRef::parse("vaults/v/wallets/w/addresses/a")
                .unwrap()
                .to_string(),
            "vaults/v/wallets/w/addresses/a"
        );
        assert!(NetworkRef::parse("bad").is_none());
        assert!(WalletAddressRef::parse("vaults/v/wallets/w").is_none());
    }

    #[test]
    fn transaction_ref_parse_round_trips_and_rejects() {
        let r = TransactionRef::parse("vaults/v/transactions/t").unwrap();
        assert_eq!(
            r,
            TransactionRef {
                vault: VaultId::new("v"),
                transaction: TransactionId::new("t"),
            }
        );
        assert_eq!(r.to_string(), "vaults/v/transactions/t");
        assert!(TransactionRef::parse("vaults/v/wallets/w").is_none());
        assert!(TransactionRef::parse("").is_none());
    }

    #[test]
    fn refs_display_as_resource_names() {
        assert_eq!(
            TransactionRef {
                vault: VaultId::new("v"),
                transaction: TransactionId::new("t"),
            }
            .to_string(),
            "vaults/v/transactions/t"
        );
        assert_eq!(
            WalletRef {
                vault: VaultId::new("v"),
                wallet: WalletId::new("w"),
            }
            .to_string(),
            "vaults/v/wallets/w"
        );
        assert_eq!(
            WalletAddressRef {
                vault: VaultId::new("v"),
                wallet: WalletId::new("w"),
                address: AddressId::new("a"),
            }
            .to_string(),
            "vaults/v/wallets/w/addresses/a"
        );
        assert_eq!(
            GasStationRef {
                vault: VaultId::new("v"),
                gas_station: GasStationId::new("g"),
            }
            .to_string(),
            "vaults/v/gasStations/g"
        );
    }
}
