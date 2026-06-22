# Design — resource-identifier contract

## The contract

**A `*Id` stores the bare leaf segment.** The part after the innermost `{collection}/`, with no
prefix and no parent scope. The full AIP resource name is a *wire encoding*, reconstructed at the
boundary, never stored. This is the direction chosen in interview ("it should be id without
prefix").

The alternative — store the full resource name — was rejected: it duplicates what the `*Ref`
types already do (a `*Ref` exists precisely to carry the full `vaults/{v}/…` name), and it makes
`as_str()` mean "wire name" for some ids and "segment" for others.

## Taxonomy

```
standalone-prefix ids        prefix       full name        get a parse/resource_name seam
  AssetId                     assets/      assets/{id}      yes  (no *Ref exists for assets)
  NetworkId                   networks/    networks/{id}    yes
  VaultId                     vaults/      vaults/{id}      yes  (replaces hand-strip)
  UserId                      users/       users/{id}       yes

vault-scoped leaf ids         full name = the *Ref; the id is just the trailing segment
  WalletId, TransactionId, GasStationId, AddressId,
  TransactionRequestId, SimulationId, VaultActionId,
  AddressBookEntryId          vaults/{v}/{collection}/{id}  no standalone seam — see below
```

- **Standalone-prefix ids** are read from / written to fields that are single-prefix resource
  names. The strip/prepend lives in their one-field Ref (`AssetRef`, `NetworkRef`), never on the Id.
  **Assets and networks are global entities** — their resource name is *always* `assets/{id}` /
  `networks/{id}`, never vault-scoped (the vault-scoped asset endpoint only puts the vault in the
  URL path; the returned `name` is still global). So `AssetRef`/`NetworkRef::parse` is total over
  real responses — the 2-segment match always holds, and there is no `vaults/.../assets/...` form
  to mis-handle.
- **Vault-scoped leaf ids** are already produced bare by `ParseRef` (which splits the whole
  `vaults/{v}/…/{id}` and keeps the leaf). Their full name needs the vault too, so it is built
  from the method's `vault` argument (or the `*Ref`), not from the id alone. `AddressBookEntryId`
  is the one currently mis-handled here.

## Mechanism

**The prefix lives only in a Ref — never on an Id.** Ids stay bare with no prefix knowledge; they
keep `new`/`From`/`as_str`/`Display` (all bare). There is **no** `parse`/`resource_name` on an Id.

A single-prefix resource gets a one-field Ref via the existing `prefix_ref!` macro, which already
renders `{prefix}/{id}` and parses it back (stripping the prefix → bare Id inside):

```
prefix_ref!(/// assets/{id} */ AssetRef, "assets", asset: AssetId);   // NEW
// NetworkRef, VaultRef, UserRef already exist
```

Two helpers, with different scoping rules:

- **`Ref::resource_name(&…) -> String`** — the emit path. A **`pub`, `#[must_use]`, borrowing**
  builder (no clone) generated for **every** ref by both macros (and hand-written on
  `WalletAddressRef`): `prefix_ref!` → `resource_name(&id)`; `vault_ref!` →
  `resource_name(&vault, &id)`. `pub` because it is a genuine public helper — and because that is
  the only warning-free way to ship it on refs with no call site yet (`pub` items don't trip
  `dead_code`; the workspace forbids `#[allow]`, and `#[expect(dead_code)]` mis-fires on the used
  ones). The format string stays single-sourced in the macro.
- **`From<Ref> for Id`** — the ingest path, lossless only for a one-field ref. Generated **only
  where a call site ingests** (the `, from_ref` flag): `AssetRef`, `NetworkRef`, `VaultRef`. Used as
  `Ref::parse(name).map(Into::into)`. Not added to `vault_ref!` (two ids → reducing to one drops the
  vault).

The Ref's `Display`/`resource_name` are the only places a prefix is written; `ParseRef::parse` the
only place one is stripped. No `From<Id> for Ref` and no `to_ref` clone method — emit borrows.

Round-trip: **string → `Ref::parse` → `Into` bare Id → store → `Ref::resource_name(&id)` → string.**

## Scope finding — who actually needs the ingest `From`

Traced against the code:
- **Emit** (`resource_name`) is exercised today by `AssetRef`/`NetworkRef`/`AddressBookEntryRef`;
  it's generated for every ref regardless (uniform builder, `pub`).
- **Ingest** (`From<Ref> for Id`) is needed for `AssetRef`/`NetworkRef` (projections) and `VaultRef`
  (the webhook strip). `VaultId` otherwise reaches the wire only as a path param (`.vault_id(as_str)`);
  `UserId` is response-only via `ResourceName<UserRef>`.

## Call-site rules

| Boundary | Accessor |
|---|---|
| Ingest a single-prefix resource name from a response | `Ref::parse(s).map(Into::into)` |
| Emit into a resource-name field (batchGet names, transfer asset, wallet networks/address) | `Ref::resource_name(&id)` |
| Emit as a single-resource path parameter | `id.as_str()` |
| Compose a vault-scoped full name | `Ref::resource_name(&vault, &id)` |

## Consequences

- Response-derived ids flip full → bare: `ResolvedAsset.id`, `WalletAddress.network`, wallet
  `networks: Vec<NetworkId>`, transaction `Transfer.asset`, `BalanceChange.asset`, address-book
  entry network. Pre-release, so no SemVer concern; tests update.
- **Three** `NetworkId` emit sites re-add the prefix (the third, `address_book.rs` `NewAddressBookEntry`,
  was missed by the paper audit and caught during implementation): wallet-create `networks`,
  `CreateAddressBuilder::send`, and the address-book create request — all `NetworkRef::resource_name(&n)`.
- `address_book.get_many` / `batch_add_to_group` build the full name from `vault` + bare entry ids
  via `AddressBookEntryRef::resource_name(&vault, entry)` (borrowing `.iter()`, no clone).
- `webhook_event.rs`'s `strip_prefix("vaults/")` becomes `VaultRef::parse(...).map(Into::into)`.
- `as_str()` is now safe to compare/log/round-trip uniformly; a `*Ref` is the single way to get a
  wire resource name.

## Resolved: fields stay bare Ids

The earlier open question (`WalletAddress.network` as `NetworkId` vs a Ref) dissolves: name-bearing
id fields stay **bare `*Id`**. This change makes them contract-correct but does NOT unify how a
network is modeled — `Network.name` is a `ResourceName<NetworkRef>`, while `Wallet.networks` /
`WalletAddress.network` stay `NetworkId`. Promoting those to `ResourceName<NetworkRef>` is a
separate, later ergonomics change, deliberately out of scope here.

## Batch boundaries — typed ids (the second half of the same defect)

The original audit chased ids that *stored* a prefix, but missed the dual: API methods that took
**raw resource-name strings** instead of typed ids. The `:batchGet`/`:batchArchive`/`:batchAdd`
methods on `wallets`/`transactions`/`address_book` took `Vec<String>` (and `batch_add_to_group` an
`impl Into<String>` group) — the caller hand-built `vaults/v/wallets/w`. That's the same
untyped-surface problem and belongs to this change.

Fix: take typed **slices** (`&[WalletId]`, `&[TransactionId]`, `&[AddressId]`, `&[AddressBookEntryId]`)
plus typed parent ids, and build the wire `names` internally with `*Ref::resource_name(&parent…, &id)`
— the borrowing builder now present on every ref. Slices, not `Vec`, because these only *read* ids
to build names (no ownership needed). Two resources gained types: the address-book entry **group**
(`AddressBookEntryGroupId`/`AddressBookEntryGroupRef`).

Bonus: the types now encode real API constraints the strings hid — `batch_get_addresses` is
single-wallet (the `wallet_id` is a path param, so all names share it), and `batch_add_to_group`
takes *existing* entry ids. The wire output is byte-identical, so only call-site construction
changed; every mock `names` assertion stayed.

## Decided

- **Borrowing `resource_name` for emit, no owned-ref construction.** Emit borrows the id
  (`Ref::resource_name(&id)`) rather than moving it into an owned `*Ref` to `to_string()` — no
  clone, and it works from `.iter()`. No `From<Id> for Ref`, no `to_ref` method. A zero-copy
  *borrowing-ref-type* family was still rejected (a second lifetime-parameterized type family
  distinct from the owned `*Ref`s that `ParseRef`/`ResourceName<R>` require); `resource_name`
  delivers the no-clone emit without it.
- **`resource_name` is `pub` and generated for every ref** (uniform builder, available even where
  unused). `pub` is both correct (a genuine public helper) and the only warning-free way to ship an
  unused one: the workspace forbids `#[allow]`, `#[expect(dead_code)]` mis-fires on the used ones,
  and `pub` items don't trip `dead_code`. The ingest `From<Ref> for Id` stays scoped to where used.
- Because asset/network names are always the global `{prefix}/{id}` form, `Ref::parse` is total on
  real responses; ingest is just `Ref::parse(name).map(Into::into)`. Keep the existing empty-field
  handling (`.filter(|s| !s.is_empty())` / `Option`) per site — no extra bare-input fallback is
  warranted, since a non-empty name is always prefixed.
