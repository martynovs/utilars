# Tasks — normalize the resource-identifier contract

## 1. The Id↔Ref bridge (prefix lives only in a Ref)

- [x] 1.1 In `src/macros.rs`/`src/resource.rs`, add the bridge **only where used** (no `to_ref`
  clone method, no `parse`/`resource_name` on any `*Id`):
  - `From<Id> for Ref` (consuming → no copy) for `AssetRef`, `NetworkRef` — the emit path.
  - `From<Ref> for Id` (lossless, one-field ref) for `AssetRef`, `NetworkRef`, `VaultRef` — ingest.
- [x] 1.2 Add `AssetRef` in `src/resource.rs`: `prefix_ref!(AssetRef, "assets", asset: AssetId)`.
  Fix the `AssetId`/`AddressBookEntryId` doc comments to state the bare-segment contract.
- [x] 1.3 Unit-test the round-trip: `Ref::parse("{prefix}/x").map(Into::into)` → bare Id;
  `Ref::from(Id::new("x")).to_string()` → `"{prefix}/x"`, for `AssetRef`/`NetworkRef` (both
  directions) and `VaultRef` (ingest). Meet the ≥90% per-function coverage floor.

## 2. AssetId call sites

- [x] 2.1 Ingest → `AssetRef::parse(name).map(Into::into)`: `assets.rs` `ResolvedAsset::from`,
  `balances.rs:702`, `networks.rs:33` (native_asset), `transactions/api.rs` `Transfer.asset` +
  `BalanceChange.asset`.
- [x] 2.2 Emit resource name → `AssetRef::resource_name(&id)`: `assets.rs` `batch_get`
  names, `transactions/transfer.rs` the three `asset` fields, `balances.rs:251` refresh body.
- [x] 2.3 Path param → `as_str`: `assets.rs` `get`/`get_for_vault`; delete `asset_segment`.

## 3. NetworkId call sites

- [x] 3.1 Ingest → `NetworkRef::parse(name).map(Into::into)`: `wallets.rs:84`
  (`WalletAddress.network`), `wallets.rs:46-50` (`Wallet.networks` list), `address_book.rs:48`
  (entry network), and any networks-group projection.
- [x] 3.2 Emit resource name → `NetworkRef::resource_name(&id)`: **two** sites —
  `wallets.rs:567` (wallet-create `networks`; switch `.iter()` → `.into_iter()` to consume) and
  `wallets.rs:620` (`CreateAddressBuilder::send` `network`).

## 4. VaultId and AddressBookEntryId

- [x] 4.1 Replace `webhook_event.rs:88` `strip_prefix("vaults/")` with
  `VaultRef::parse(...).map(Into::into)`.
- [x] 4.2 `address_book.get_many`: build the full name from the `vault` arg + bare entry ids via
  the `AddressBookEntryRef` `Display`, instead of `n.to_string()`.

## 5. CLI

- [x] 5.1 `examples/cli.rs`: CLI args stay `String`; the `Assets` handler and `Transfer --asset`
  accept either form by normalizing through the Ref —
  `AssetRef::parse(&s).map(Into::into).unwrap_or_else(|| AssetId::new(s))` (prefixed → stripped,
  bare → taken as-is). Update help text / module header to show the prefix is optional.

## 5b. Type the batch boundaries (raw `Vec<String>` → typed id slices)

- [x] 5b.1 Add `AddressBookEntryGroupId` (`id_newtype!`) + `AddressBookEntryGroupRef`
  (`vault_ref!`); export both from `lib.rs`. The group arg was a raw `impl Into<String>`.
- [x] 5b.2 Retype the batch methods to typed **slices**, building names via `*Ref::resource_name`:
  `wallets().batch_get`/`batch_archive`/`batch_unarchive` → `&[WalletId]`;
  `batch_get_addresses(vault, wallet, &[AddressId])`; `transactions().batch_get` → `&[TransactionId]`;
  `address_book().get_many` → `&[AddressBookEntryId]`;
  `batch_add_to_group(vault, AddressBookEntryGroupId, &[AddressBookEntryId])`.
- [x] 5b.3 Flip the batch test call sites from `Vec<String>` resource names to typed id slices
  (`&[Id::new("w1")]`); wire (`names`) assertions unchanged.

## 6. Tests & gates

- [x] 6.1 Switch every fixture that builds an id from a prefixed literal (`Id::new("assets/…")`,
  `"networks/…".into()`) to a **bare** literal (`Id::new("native.ethereum-mainnet")`) or
  `Ref::parse("assets/…").map(Into::into)` — a prefixed literal would now double on emit. Affects
  `tests/assets.rs`, `tests/wallets.rs`, `tests/transactions.rs`, `tests/balances.rs`,
  `tests/address_book.rs`.
- [x] 6.2 Flip response-id assertions full → bare across the same test files + in-crate tests.
- [x] 6.3 Add the regression: a `batch_get` (and a wallet-create) case passing a **bare** id and
  asserting the wire carries the full `{prefix}/{id}` form.
- [x] 6.4 Run gates in order: `cargo clippy --workspace --all-targets` → `just crap-ci` →
  `cargo fmt --all`.
