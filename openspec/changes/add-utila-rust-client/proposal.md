# Add Utila Rust API Client

## Why

Utila exposes a v2 REST API (a gRPC service surfaced via grpc-gateway, Google AIP
conventions) but ships no Rust SDK. Teams integrating Utila from Rust currently
hand-roll reqwest calls, JWT minting, and pagination. This change introduces a
published, open-source Rust client crate that wraps the full v2 API with an
ergonomic, idiomatic layer.

A spike has already validated feasibility end-to-end (see `design.md`): the
OpenAPI spec generates cleanly with [progenitor](https://github.com/oxidecomputer/progenitor),
compiles with zero warnings, and the AIP `:customMethod` paths, auth seam, and
builder ergonomics all work. The remaining work is the value-add layer that makes
the crate worth publishing rather than thin.

## What Changes

- **New crate `utila`** (Apache-2.0 OR MIT), published to crates.io, covering all
  42 v2 operations across 11 resource groups (Vaults, Wallets, Transactions,
  Balances, Address Book, Assets, Blockchains, Policy, Webhooks, Gas Stations).
- **Generated core** via progenitor, driven by an `xtask` (builder API:
  `InterfaceStyle::Builder` + `TagStyle::Separate`). Generated output is
  **committed** to the repo; the OpenAPI spec is **vendored** and refreshed via a
  separate `xtask pull`.
- **Authentication layer**: service-account self-signed RS256 JWT (`sub`/`aud`/
  `exp`), minted, cached, and refreshed centrally; injected into every request via
  progenitor's async pre-hook. Pluggable `Signer` (local RSA PEM in v1; KMS behind
  a feature flag).
- **Ergonomic layer** (shaped via design interview): a hand-written **grouped facade**
  (`client.vaults()`, `client.transactions()`, …) with **typed-ID newtypes**; typed
  `UtilaError` from `GooglerpcStatus`; **`Money`** (always-exact base units + optional
  decimal/symbol views) with **auto-enriched, batched, cached** asset metadata on
  balances; **raw page + `Stream`** pagination over `pageToken`. Call signatures follow
  a modeling principle (design D0): mutually-exclusive value-sets are **enums**,
  truly-optional values are `Option`, independent modifiers are builder setters.
- **Transactions, idempotency, retry, webhooks**: typed 14-variant `TransactionDetails`
  enum with a builder `initiate` (display-unit `SendAmount`); auto-generated,
  overridable `requestId` idempotency keys; retry **segmented by operation purpose**
  (read vs create), off by default; receiver-side webhook **signature verification**
  (RSA-4096/SHA-512/PSS) with typed events.

## Non-goals

- Native gRPC (`tonic`) client — Utila does not publish `.proto`s; REST is the
  supported surface.
- Webhook *configuration* (no v2 API; done in console) and co-signer signing flows.
  Webhook *signature verification* (receiver-side) IS in scope (see design D13).

## Open Decisions (defaults chosen, confirm before publish)

- Crate name: `utila` (alt: `utila-sdk`).
- License: `Apache-2.0 OR MIT` (Rust ecosystem standard).
- KMS signer: trait in v1, `KmsSigner` impl behind a `kms` feature (AWS first).

## Impact

- New crate; no existing code affected (greenfield repo).
- Adds a spec-drift CI check (committed generated code must match a fresh regen).
- Affected spec capability: `utila-client` (new).
