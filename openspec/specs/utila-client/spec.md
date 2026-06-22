# utila-client Specification

## Purpose
TBD - created by archiving change normalize-resource-id-contract. Update Purpose after archive.
## Requirements
### Requirement: Resource identifier value contract
Every typed-ID newtype SHALL store the **bare leaf segment** of its resource name — the part
after the innermost `{collection}/`, carrying no resource-name prefix and no parent scope — and
`as_str()`/`Display` SHALL return that bare segment uniformly for every id type. This applies to
`VaultId`, `WalletId`, `AssetId`, `NetworkId`, `AddressId`, `AddressBookEntryId`, `UserId`,
`GasStationId`, `TransactionId`, `TransactionRequestId`, `SimulationId`, and `VaultActionId`. The
full AIP resource name is reconstructed only at the wire boundary; it is never stored.

This makes the stored value of an id mean the same kind of thing regardless of which id it is,
and removes per-type, ad-hoc prefix handling.

#### Scenario: Parsing a resource name strips the prefix
- **WHEN** an id is parsed from an API resource-name string (e.g. `assets/native.ethereum-mainnet`,
  `networks/ethereum-mainnet`, or `vaults/abc`)
- **THEN** the stored value is the bare segment (`native.ethereum-mainnet`, `ethereum-mainnet`, `abc`)
- **AND** `as_str()` returns that bare segment

#### Scenario: An id constructed from a bare segment is stored as-is
- **WHEN** an id is constructed directly from its bare segment (e.g. `AssetId::new("native.ethereum-mainnet")`)
- **THEN** the stored value is that bare segment, unchanged
- **AND** the low-level constructors (`new`/`From`) do not strip or add a prefix — normalizing an
  arbitrary resource name is the job of the resource's `*Ref` parse, not the id constructor

#### Scenario: A bare id is accepted and produces the prefixed wire form
- **WHEN** a caller passes a bare id (e.g. `AssetId::new("native.ethereum-mainnet")`) to a facade
  method whose wire field is a resource name (`assets:batchGet` `names`, an `assetTransfer`
  `asset`, a wallet-create `networks` entry)
- **THEN** the request carries the full `{collection}/{id}` form (`assets/native.ethereum-mainnet`)
- **AND** the caller is not required to include the prefix — bare is the canonical accepted input

#### Scenario: A path parameter uses the bare segment
- **WHEN** an id is emitted as a single-resource path parameter (e.g. `GET /v2/assets/{asset_id}`)
- **THEN** the path carries only the bare segment

#### Scenario: Response-derived ids are bare for every id type
- **WHEN** a response is projected into a facade type holding ids (e.g. `ResolvedAsset.id`,
  `WalletAddress.network`, a transaction `Transfer.asset`, an address-book entry's network)
- **THEN** each id's `as_str()` returns the bare segment, consistent across all of them

#### Scenario: Normalization is uniform, not per-type ad-hoc
- **WHEN** any standalone-prefixed id (`AssetId`, `NetworkId`, `VaultId`) is ingested from a response
- **THEN** it is normalized through its `*Ref` parse, the same way for each
- **AND** no call site hand-rolls `strip_prefix("vaults/")` or similar

#### Scenario: Assets and networks are global single-prefix resources
- **WHEN** an asset or network resource name is read from any response (including the vault-scoped
  asset endpoint, where the vault appears only in the URL path)
- **THEN** the name is the global `assets/{id}` / `networks/{id}` form, never `vaults/.../assets/...`
- **AND** `AssetRef`/`NetworkRef` parse it to a bare id without a vault-scoped special case

### Requirement: Batch operations take typed ids, not resource-name strings
Batch methods SHALL accept typed ids — a slice of the child id (`&[WalletId]`, `&[TransactionId]`,
`&[AddressId]`, `&[AddressBookEntryId]`) plus the typed parent ids — never a raw `Vec<String>` of
resource names or an `impl Into<String>` group. The full `vaults/{…}/{collection}/{id}` wire names
the `:batchGet`/`:batchArchive`/`:batchAdd` endpoints require are reconstructed internally via the
resource's `*Ref::resource_name(&parent…, &id)` builder, so a caller never hand-builds one. Every
resource that appears as such an id has a typed id + `*Ref` (including `AddressBookEntryGroupId` /
`AddressBookEntryGroupRef`, added for the group-membership endpoint).

#### Scenario: Batch-get by id reconstructs the wire names
- **WHEN** a caller invokes `wallets().batch_get(vault, &[WalletId])` (or the transaction / address
  / address-book-entry equivalents)
- **THEN** the request's `names` carry the full `vaults/{vault}/wallets/{id}` form, built internally
- **AND** the caller passes only the vault and bare ids — no resource-name strings

#### Scenario: Address batch-get is single-wallet by type
- **WHEN** a caller invokes `wallets().batch_get_addresses(vault, wallet, &[AddressId])`
- **THEN** every requested name shares the path's `wallet_id` (the API rejects cross-wallet names)
- **AND** the single-wallet constraint is enforced by the signature, not left to the caller

#### Scenario: Group membership uses a typed group id and existing entry ids
- **WHEN** a caller invokes `address_book().batch_add_to_group(vault, group, &[AddressBookEntryId])`
- **THEN** `group` is an `AddressBookEntryGroupId` (not a raw string), reconstructed to
  `vaults/{vault}/addressBookEntryGroups/{group}`
- **AND** the entry ids reference **existing** entries added to the group

