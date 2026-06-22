//! Internal macros, imported by path (`use crate::macros::id_newtype;`).

/// Define a transparent string newtype id: distinct type with `new`/`as_str`, `From<String>` /
/// `From<&str>`, `AsRef<str>`, `Display`, and serde transparency.
macro_rules! id_newtype {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// The empty id — a blank sentinel to construct or compare against.
            pub const EMPTY: Self = Self(String::new());

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

pub(crate) use id_newtype;

/// Define a vault-scoped resource reference `vaults/{vault}/<collection>/{id}`: a struct with
/// `vault` + the child id, plus `Display` (canonical name) and [`crate::resource::ParseRef`].
///
/// Every ref also gets `Ref::resource_name(&vault, &id)`, which builds the full wire name from
/// borrowed parts — no clone, format single-sourced here. (No `From<Ref> for Id`: a vault-scoped
/// ref holds two ids, so reducing it to one would silently drop the vault.)
macro_rules! vault_ref {
    ($(#[$m:meta])* $name:ident, $collection:literal, $field:ident: $id:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $name {
            pub vault: VaultId,
            pub $field: $id,
        }
        impl $name {
            /// The full `vaults/{vault}/<collection>/{id}` resource name, from borrowed parts —
            /// the no-clone builder, available on every ref whether or not the crate uses it yet.
            #[must_use]
            pub fn resource_name(vault: &VaultId, $field: &$id) -> String {
                format!(concat!("vaults/{}/", $collection, "/{}"), vault, $field)
            }
        }
        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                write!(f, concat!("vaults/{}/", $collection, "/{}"), self.vault, self.$field)
            }
        }
        impl ParseRef for $name {
            fn parse(name: &str) -> ::std::option::Option<Self> {
                match name.split('/').collect::<::std::vec::Vec<_>>().as_slice() {
                    ["vaults", v, $collection, x] => ::std::option::Option::Some(Self {
                        vault: VaultId::new(*v),
                        $field: $id::new(*x),
                    }),
                    _ => ::std::option::Option::None,
                }
            }
        }
    };
}
pub(crate) use vault_ref;

/// Define a single-segment resource reference `<prefix>/{id}` (e.g. `networks/{id}`,
/// `users/{id}`): a one-field struct with `Display` and [`crate::resource::ParseRef`].
///
/// Every ref gets both halves of the Id↔name bridge: `Ref::resource_name(&id)` (emit: build
/// `<prefix>/{id}` from a borrowed id, no clone) and `From<Ref> for Id` (ingest:
/// `Ref::parse(name).map(Id::from)`) — lossless since a single-prefix ref holds exactly one id.
macro_rules! prefix_ref {
    ($(#[$m:meta])* $name:ident, $prefix:literal, $field:ident: $id:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $name {
            pub $field: $id,
        }
        impl $name {
            /// The full `<prefix>/{id}` resource name, built from a borrowed id — no clone.
            #[must_use]
            pub fn resource_name($field: &$id) -> String {
                format!(concat!($prefix, "/{}"), $field)
            }
        }
        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                write!(f, concat!($prefix, "/{}"), self.$field)
            }
        }
        impl ParseRef for $name {
            fn parse(name: &str) -> ::std::option::Option<Self> {
                match name.split('/').collect::<::std::vec::Vec<_>>().as_slice() {
                    [$prefix, x] => ::std::option::Option::Some(Self { $field: $id::new(*x) }),
                    _ => ::std::option::Option::None,
                }
            }
        }
        impl ::std::convert::From<$name> for $id {
            fn from(r: $name) -> Self { r.$field }
        }
    };
}
pub(crate) use prefix_ref;
