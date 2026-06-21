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
macro_rules! vault_ref {
    ($(#[$m:meta])* $name:ident, $collection:literal, $field:ident: $id:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $name {
            pub vault: VaultId,
            pub $field: $id,
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
macro_rules! prefix_ref {
    ($(#[$m:meta])* $name:ident, $prefix:literal, $field:ident: $id:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $name {
            pub $field: $id,
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
    };
}
pub(crate) use prefix_ref;
