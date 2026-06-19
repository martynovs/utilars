# utilars

[![CI](https://github.com/martynovs/utilars/actions/workflows/ci.yml/badge.svg)](https://github.com/martynovs/utilars/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/utilars.svg)](https://crates.io/crates/utilars)
[![Docs.rs](https://docs.rs/utilars/badge.svg)](https://docs.rs/utilars)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![MSRV 1.91](https://img.shields.io/badge/MSRV-1.91-blue.svg)](#minimum-supported-rust-version)

A typed, async Rust client for the [Utila](https://utila.io) v2 API (crypto custody:
vaults, wallets, balances, transactions, networks, address book, assets, and webhooks).

> [!NOTE]
> **Unofficial / community project.** This is an independent client and is **not affiliated
> with, maintained, or endorsed by Utila**. "Utila" is a trademark of its respective owner;
> this crate merely consumes their public v2 API. Use at your own risk.

> **Install.**
>
> ```toml
> [dependencies]
> utilars = "0.1"
> ```
> ```rust
> use utilars::UtilaClient;
> ```
>
> Until the first crates.io release, depend on it straight from git:
>
> ```toml
> [dependencies]
> utilars = { git = "https://github.com/martynovs/utilars" }
> ```

## Quick start

The library **never reads the environment** — your application supplies the service-account
email and a signer (reading its own secrets/config), and passes them to the builder.

```rust
use utilars::{UtilaClient, SignerSource, VaultId};
use futures::TryStreamExt;

let client = UtilaClient::builder()
    .credential(
        "my-sa@vault-a1b2c3d4.utilaserviceaccount.io",
        SignerSource::local_pem(service_account_pem)?, // your PEM bytes
    )
    .build()?;

// one page, or stream every page
let page = client.vaults().list().page_size(50).send().await?;
let all: Vec<_> = client.vaults().stream().try_collect().await?;

// balances, with each asset's decimals/symbol resolved + cached
for bal in client.balances().query(VaultId::new("abc")).await? {
    println!("{} {:?}", bal.amount, bal.asset.symbol());
}
```

## Authentication

Auth is a self-signed **RS256 JWT** bearing the service-account email (`sub`), the fixed
audience `https://api.utila.io/`, and a 1-hour expiry. The token is minted, cached, and
refreshed automatically (single-flighted, so concurrent requests mint at most one token).

Two signer backends:

- **Local RSA key (default):** `SignerSource::local_pem(pem_bytes)` — pure-Rust, no native
  dependencies.
- **AWS KMS (`aws` feature):** keep the key in KMS and never expose it. Enable the feature and
  pass a typed KMS key reference; signing happens remotely via `RSASSA_PKCS1_V15_SHA_256`.

  ```toml
  utilars = { version = "0.1", features = ["aws"] }
  ```

  The AWS SDK is **off by default** — non-KMS users pull none of it.

## The API surface

Everything hangs off `UtilaClient` as grouped, typed sub-clients:

| Group | Examples |
|---|---|
| `client.vaults()` | `list` · `stream` · `get` |
| `client.wallets()` | `list` · `stream` · `get` · `create` · `list_addresses` · `create_address` · `archive` |
| `client.balances()` | `query` · `query_wallet_balances` · `query_wallet_address_balances` · `query_wallet_utxos` · `refresh_asset_address_balance` |
| `client.transactions()` | per-kind sends (`asset_transfer`, `evm`, `tron`, …) · `get` · `list`/`stream` · `batch_get` · `cancel` · `publish` · `replace` · `vote` · `estimate_fee` · `latest_simulation` · `aml_screening` |
| `client.networks()` | `list`/`stream` · `get` · vault-scoped variants |
| `client.address_book()` | `list`/`stream` · `get` · `create` · `delete` |
| `client.assets()` | `get` · `get_for_vault` · `batch_get` |

Resource ids are **distinct newtypes** (`VaultId`, `WalletId`, `AssetId`, `NetworkId`,
`TransactionId`, …) so a wallet id can't be passed where a vault id is expected.

### Pagination

Every list endpoint offers both a single page and a stream:

```rust
use futures::TryStreamExt;

let page = client.transactions().list(vault.clone()).page_size(100).send().await?;
let all: Vec<_> = client.transactions().list(vault).stream().try_collect().await?;
```

### Amounts & assets

Monetary values are exact integer base units (`Amount`), never lossy floats. A `Balance`
carries an `Asset` that is either `Resolved { decimals, symbol }` or `Unresolved(AssetId)` —
`balances().query()` resolves them via one batched, cached `assets:batchGet`, and degrades
gracefully (an unlisted asset still reports its exact base-unit amount).

## Transactions: `transfer` vs. `send`

Two layers, mirroring the API:

- **`transfer` — move tokens (cross-chain).** You give an asset + amount + source/destination
  and Utila builds the on-chain transaction. Works on every network. This is the normal path,
  and where **sponsored transfers** and chain riders live.

  ```rust
  use utilars::{AssetTransfer, VaultId, Priority};
  use rust_decimal::dec;

  let out = client.transactions()
      .asset_transfer(VaultId::new("abc"), AssetTransfer {
          asset: "assets/native.ethereum-mainnet".into(),
          source: "vaults/abc/wallets/w1".into(),
          destination: "0xabc…".into(),
          amount: dec!(1.5),
          memo: None,
          sponsor: Some("vaults/abc/wallets/gas".into()), // sponsored: gas wallet pays the fee
          pay_fee_from_amount: None,
          stellar_memo: None,
          xrpl_destination_tag: None,
      })
      .priority(Priority::High)   // optional modifiers, then…
      .send().await?;             // terminal: one request; fails only on real API/transport errors
  println!("request_id = {}", out.request_id); // idempotency key, auto-generated
  ```

- **`send` — a network-specific transaction you build yourself.** Lower-level: a raw EVM call,
  a Tron `triggerSmartContract`, or a pre-serialized Solana/Stellar/Sui/XRPL transaction.

  ```rust
  use utilars::{EvmTransaction, VaultId};

  client.transactions()
      .evm(VaultId::new("abc"), EvmTransaction {
          from_address: "0x…".into(),
          network: "ethereum-mainnet".into(),
          to: Some("0xContract".into()),
          value: None,
          data: Some("0xa9059cbb…".into()),
          publish: None,
      })
      .send().await?;
  ```

Each transaction kind is a per-kind method (`asset_transfer`, `evm`, `tron`, `solana_raw`,
`stellar`, `xrpl_raw`, …) returning a modifier builder with a terminal `.send()`. Required
fields are enforced up front (struct literals for flat kinds; `::builder().build()?` for the
deeply-nested Tron/Stellar kinds), so `send()` never fails for a forgotten field. Every
initiation gets an auto-generated idempotency `requestId` you can override for safe retries.

## Webhooks

Receiver-side verification of inbound webhooks — **RSA-4096 / SHA-512 / PSS** over the raw
body against Utila's published key (bundled, overridable), then a typed event:

```rust
use utilars::webhook::{self, EventKind};

let verified = webhook::verify(raw_body, x_utila_signature)?; // checks the signature
match verified.parse()?.kind {
    EventKind::TransactionStateUpdated => { /* … */ }
    EventKind::WalletAddressCreated    => { /* … */ }
    _ => {}
}
```

## Retrying

Retry is intentionally **not** built in — each call issues exactly one request. To retry,
wrap an operation with a retry crate (e.g. [`backon`](https://docs.rs/backon)) and gate it on
`UtilaError::is_retryable`, which classifies transient transport + server errors (timeouts,
connect failures, gRPC `4`/`8`/`14`, HTTP `429`/`5xx`):

```rust
use backon::{ExponentialBuilder, Retryable};
use utilars::UtilaError;

let vault = (|| client.vaults().get(id.clone()))
    .retry(ExponentialBuilder::default())
    .when(UtilaError::is_retryable)
    .await?;
```

## Errors

All fallible calls return `Result<T, UtilaError>`. `UtilaError::Api { code, message, details }`
carries the gRPC status (including `details`); other variants cover auth, transport (`Http`),
amount, and config failures.

## Cargo features

| Feature | Default | Effect |
|---|---|---|
| *(none)* | ✅ | local-PEM signing; pure-Rust, no native deps |
| `aws` | | AWS KMS signer via `aws-sdk-kms` (off by default — opt in for remote signing) |

## Security

Webhook verification uses the pure-Rust [`rsa`](https://docs.rs/rsa) crate. `rsa 0.9` carries
advisory **RUSTSEC-2023-0071** (the "Marvin" timing side-channel), which affects RSA
**decryption** — an operation this crate never performs. It uses `rsa` only for PSS signature
**verification** (webhooks) and signing in tests, so the advisory does not apply to this usage.
The fix lands in `rsa 0.10`, currently a pre-release.

## Demo CLI

`examples/cli.rs` is a worked example compiled as the binary **`utilars`**:

```sh
export UTILA_ACCOUNT='my-sa@vault-xxxxxxxx.utilaserviceaccount.io'
export UTILA_PRIVATE_KEY_PATH=/path/to/key.pem        # local PEM (file)
# …or inline (raw / `\n`-escaped / base64): export UTILA_PRIVATE_KEY_PEM="$(base64 < key.pem)"
# …or AWS KMS (build with --features aws):  export UTILA_KMS_KEY_URL='awskms:///arn:aws:kms:…'

cargo run --example utilars -- vaults
cargo run --example utilars -- balances <vault-id>
cargo run --example utilars -- transfer <vault-id> --asset … --from … --to … --amount 1.5
cargo run --example utilars -- send evm <vault-id> --from … --network … --to 0xContract --data 0x…
```

## How the client is generated

The low-level transport (`src/generated.rs`) is generated from Utila's OpenAPI spec with
[progenitor](https://docs.rs/progenitor); the hand-written facade in `src/*.rs` wraps it with
typed inputs/outputs, pagination, and asset enrichment. The committed generated code is stamped
with the spec revision it came from (see its header and `openapi/utila.v2.meta.json`). Regenerate
with `cargo xtask pull` (fetch the spec) and `cargo xtask gen` (regenerate).

## Minimum supported Rust version

Rust **1.91** or newer. The binding constraints are `Duration::from_hours`/`from_mins` used in
`const` items (stable in `const` context since 1.91) and the `zeroize` dependency (edition 2024,
needs ≥1.85); native `async fn` in traits (1.75) is also used.

## Contributing

Issues and pull requests are welcome at
[github.com/martynovs/utilars](https://github.com/martynovs/utilars).

The hand-written facade lives in `src/*.rs`; `src/generated.rs` is codegen output — don't edit
it by hand (regenerate with `cargo xtask gen`). Before opening a PR, run the gates in order:

```sh
cargo clippy --workspace --all-targets   # pedantic; must be warning-free
just crap-ci                             # complexity/coverage gate (threshold 30, ≥90% per-fn)
cargo fmt --all                          # always last
```

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion
in this crate by you, as defined in the Apache-2.0 license, shall be dual licensed as above,
without any additional terms or conditions.
