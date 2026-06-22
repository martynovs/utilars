# Normalize the resource-identifier contract

## Why

The typed-ID newtypes (design D8) were introduced to prevent id-type mix-ups, but D8 never
pinned down **what a `*Id` stores** — the bare leaf segment (`native.ethereum-mainnet`) or the
full AIP resource name (`assets/native.ethereum-mainnet`). The implementation drifted as a
result:

- `AssetId` and `NetworkId` store the **full** resource name (`assets/...`, `networks/...`).
- Their sibling ids (`WalletId`, `TransactionId`, …) store the **bare** segment, because they
  are produced by `*Ref` parsing which splits on `/`.
- `VaultId` is bare, but `webhook_event.rs` strips `vaults/` by hand in exactly one spot.
- `AddressBookEntryId` is documented as a full resource name yet is the leaf of a vault-scoped
  `*Ref`.

So `id.as_str()` returns a different *kind* of value depending on which id you hold, and callers
are forced to remember the `assets/` prefix on some paths but not others. The trigger was the CLI
`assets` command requiring `assets/native.ethereum-mainnet`; the root cause is the missing
contract.

The sharpest illustration: a **network** is already modeled three ways in the same crate —
`Network.name` is a `ResourceName<NetworkRef>` (typed, correct), while `Wallet.networks` is a
`Vec<NetworkId>` and `WalletAddress.network` a `NetworkId`, both storing the full `networks/…`
string inside an id that is documented as the bare segment.

## What Changes

- **Define one contract** (new spec requirement): every `*Id` stores the bare leaf segment;
  `as_str()`/`Display` are bare for all id types; the full resource name is reconstructed only at
  the wire boundary.
- **A single normalizer, the `*Ref`**: a resource-name string is parsed to a bare id through its
  `*Ref` (the only place a prefix is stripped); emitting prepends the prefix via the Ref's `Display`
  (the only place one is written); path parameters use the bare segment. The id constructors
  (`new`/`From`) stay verbatim — bare in, bare out — so the bare segment is the canonical input and
  the caller is never required to type `assets/`.
- **Fix the violators** so the contract holds everywhere: `AssetId` (assets, balances, transfers,
  balance changes, network native asset), `NetworkId` (wallet addresses/networks, address-book
  entries), `AddressBookEntryId` (`get_many`), and generalize the ad-hoc `VaultId` strip.
- **No new public concept for the caller**: ids remain typed newtypes; passing the bare or the
  prefixed form both work.
- **Type the batch boundaries** (the same untyped-surface defect): the `:batchGet`/`:batchArchive`/
  `:batchAdd` methods that took raw `Vec<String>` resource names (`wallets`, `transactions`,
  `address_book`) now take typed id slices (`&[WalletId]`, …) and build the wire names internally,
  adding `AddressBookEntryGroupId`/`AddressBookEntryGroupRef` for the previously-`String` group arg.

## Impact

- Affected spec: `utila-client` (ADDED *Resource identifier value contract*).
- Affected code: `src/resource.rs`, `src/macros.rs`, `src/assets.rs`, `src/networks.rs`,
  `src/wallets.rs`, `src/address_book.rs`, `src/balances.rs`, `src/transactions/{api,transfer}.rs`,
  `src/webhook_event.rs`, `examples/cli.rs`.
- **Behavior change (pre-release, no compat guarantee yet):** response-derived ids that are
  currently full (`WalletAddress.network`, wallet `networks`, `ResolvedAsset.id`, transaction
  `Transfer.asset`) become bare. Many `*Id` test fixtures/assertions update accordingly.
- Builds on the in-progress `add-utila-rust-client` change; should land before first publish so the
  contract ships correct from v0.
