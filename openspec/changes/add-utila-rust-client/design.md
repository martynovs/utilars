# Design: Utila Rust API Client

## Context

Utila's v2 API is a gRPC service exposed over REST via grpc-gateway, following
Google AIP conventions. Spike findings (all empirically validated):

| Property | Finding |
| --- | --- |
| Spec | OpenAPI 3.0.0, 209 KB, `https://docs.utila.io/openapi/latest_apidocs-v2-readme.openapi.json` |
| Surface | 42 operations (22 GET / 20 POST), 239 schemas, 11 tags |
| Server | `https://api.utila.io` |
| Auth | `Authorization: Bearer <JWT>` (bearerFormat JWT) |
| AIP markers | 23 `:customMethod` paths, `batchGet`/`batchCreate`, `pageToken`, `filter` |
| Composition | 0 `oneOf`/`anyOf`/`discriminator`; 180 single-`$ref` `allOf` (merged cleanly) |
| Numbers | 29 int64/uint64 fields serialized as JSON **strings**; 0 raw numeric money fields |
| Timestamps | mapped to `chrono::DateTime` (21 fields) |
| Codegen | progenitor 0.14 generates + compiles, **0 warnings** (both CLI and xtask paths) |

## Goals

- Idiomatic async Rust client covering the full v2 surface.
- Auth that "just works" from env/config, with central token refresh.
- Exact money handling (no lossy float), ergonomic pagination, typed errors.
- Low-maintenance regeneration as Utila evolves the spec.

## Non-goals

- gRPC/tonic client; fluent sub-client facade; webhook/co-signer flows (v1).

## Decisions

### D0. API modeling principles (cross-cutting)

These govern how every call signature and type in the facade is shaped:

- **Mutually-exclusive value-sets → `enum`.** When a call accepts several alternative
  *sets* of values and exactly one set is valid, model it as an enum where each variant
  is one complete valid combination. Makes illegal states unrepresentable. Applies to:
  `TransactionDetails` (D11), `SignerSource` (D4), `AssetTransfer` chain options.
- **Truly optional single values → `Option<T>`.** A value that may simply be absent is
  an `Option`, not an enum variant or a sentinel.
- **Independent optional modifiers → builder setters** (each backed by an `Option`
  field). When many optionals are *independently* valid in any combination, expose them
  as builder methods rather than an enum. Applies to: `initiate` (D11).
- **Settings are segmented by operation purpose.** Read and create/mutate operations
  have different needs, so the client carries per-class settings (retry, timeout, …)
  rather than one global blob (D14).

### D1. Generator: progenitor via an `xtask` (not the CLI, not a macro/build.rs)

The CLI proved feasibility but only emits a flat, positional, merged client. The
**builder `Generator` API** gives full control:

```rust
GenerationSettings::default()
    .with_interface(InterfaceStyle::Builder)   // chainable optional params
    .with_tag(TagStyle::Separate)              // per-area ext traits
    .with_inner_type(quote!{ Arc<TokenManager> })
    .with_pre_hook_async(quote!{ crate::auth::inject_bearer })
    .with_post_hook(quote!{ crate::auth::on_response })
    .with_derive("PartialEq");
```

- **Macro (`generate_api!`)** rejected: generated symbols invisible to IDE/docs.rs.
- **build.rs** rejected: consumers compile progenitor; output not reviewable.
- **Committed output** chosen: clean docs.rs, reviewable diffs on spec change, no
  build-time codegen for consumers.

### D2. Spec is vendored; URL is the source

`xtask pull` fetches the OpenAPI URL into a committed `openapi/utila.v2.json`
(provenance + reviewable diff). `xtask gen` regenerates `src/generated.rs` from the
vendored file (reproducible, hermetic, offline-capable). CI runs `gen` and fails if
the committed output drifts from a fresh regen.

### D3. Surface: hand-written grouped facade (decided via design interview)

The generated `TagStyle::Separate` output (per-tag extension traits,
`transactions_initiate_transaction()`) is the **internal base**. On top we hand-write
a **grouped facade** — `client.vaults()`, `client.transactions()`, `client.wallets()`,
`client.balances()`, … (~11 groups) — exposing fluent, typed methods:

```rust
client.vaults().list().page_size(50).send().await?;
client.transactions().initiate(vault_id, req).await?;
```

- Group methods take **typed-ID newtypes** (D8) and return facade types where they add
  value (D9 pagination, D10 money), otherwise pass generated types through.
- **Tradeoff accepted**: the facade is hand-maintained and can drift from the spec —
  a new generated operation does not appear until added to a group. Mitigation:
  wrappers are thin pass-throughs; the regen diff (D2) flags new generated methods to
  surface. This was chosen over shipping progenitor's raw surface for caller ergonomics.

### D4. Authentication — self-signed RS256 JWT

Authoritative per Utila docs: the JWT **is** the credential; there is no token
exchange endpoint. Exactly three claims:

```json
{ "sub": "<service-account-email>", "aud": "https://api.utila.io/", "exp": <now+1h> }
```

Signed RS256 with the service-account RSA key. No `iss`/`iat`/`nbf`/`scope`.

- **`aud` is a fixed constant** `https://api.utila.io/` (trailing slash) — distinct
  from the request base URL `https://api.utila.io`; must not be derived from it.
- **`Signer` trait** abstracts signing because KMS cannot use `jsonwebtoken`
  (key never exposed); both local and KMS reduce to "sign these bytes":

  ```rust
  trait Signer: Send + Sync {
      async fn sign(&self, signing_input: &[u8]) -> Result<Vec<u8>>;
  }
  // default: LocalRsaSigner (PEM, PKCS#1-v1.5 / SHA-256, sync under the hood).
  // Feature `aws` (off by default): AwsKmsSigner via `aws-sdk-kms`
  // (RSASSA_PKCS1_V1_5_SHA_256). Native `async fn` in trait (static dispatch),
  // no `async-trait` crate.
  ```

- **`TokenManager`** caches the token and refreshes when within 5 min of expiry
  (margin defends against a fast local clock), single-flighting concurrent refreshes
  (matters for billed KMS calls).
- **Time is testable, never `SystemTime::now()` in non-test code.** Refresh timing runs
  on `tokio::time::Instant` (mockable via `#[tokio::test(start_paused)]` +
  `time::advance`); the wall clock is read **once** at construction (`chrono::Utc::now()`)
  to anchor the `exp` claim, with "now" reconstructed as `anchor + tokio_elapsed`. No
  hand-rolled `Clock` trait.
- **Injection**: `with_inner_type(Arc<TokenManager>)` staples it onto the generated
  `Client`; the async pre-hook reads it, mints/refreshes, sets the header. Refresh
  failure returns `Err` → the request aborts before the network. `on_response`
  post-hook invalidates the cache on 401.
- **Signer as an enum of *validated* values** (per D0 principle): the signing source is
  mutually exclusive → an enum, not optional fields. Each variant holds an
  already-validated value, so an invalid signer cannot be represented — parsing happens
  in the value constructors (which return `Result`), not at sign time. Because the value
  is pre-resolved there is **no separate "prepared" type**; `TokenManager` holds the
  `SignerSource` directly. There is also **no `Credential` wrapper struct** — account and
  signer are passed as two args (`TokenManager::new(account, signer)` /
  `builder.credential(account, signer)`).

  ```rust
  enum SignerSource {
      Local(jsonwebtoken::EncodingKey),  // parsed PEM (zeroized on drop), not raw bytes
      Kms(KmsKey),                        // typed/validated key ref; signs via feature `aws`
  }
  SignerSource::local_pem(pem)?           // validation lives here…
  // KMS keys are typed + validated per cloud, not opaque URLs:
  enum KmsKey { Aws(AwsKmsArn) /* Gcp/Azure later */ }   // KmsKey::parse / AwsKmsArn::parse
  ```

  `SignerSource` resolves to a `Signer` impl: `Local` → `LocalRsaSigner`; `Kms(Aws)` →
  `AwsKmsSigner` when built with the `aws` feature (without it, signing returns a typed
  `Auth` error pointing at the feature flag).

- **No `from_env`.** A library must not read `std::env` — the caller supplies the account
  and signer (reading its own env/secrets) and passes them to the builder. (Superseded
  the earlier `UtilaClient::from_env()` / `UTILA_*` idea.)

### D5. Errors — typed in the facade, not the post-hook

The spec declares only `200`, so progenitor returns
`Error::UnexpectedResponse(reqwest::Response)` for failures. The post-hook gets only
a shared `&result` and cannot re-type the return, so typed errors live in the
**facade**: parse `GooglerpcStatus { code, message, details: [protobufAny] }` from
the error body into `UtilaError`.

### D6. Amounts — `Amount` (exact) + `Money` (enriched), decided via interview

The 29 money fields are integer base-units (wei-style) as `String`. `rust_decimal`
caps at ~7.9e28 and can overflow large 18-decimal balances, so the source of truth is
always an **exact integer**. Two types:

- **`Amount`** — wraps the base-unit string; `to_u128()`/`as_str()` exact; the
  low-level type used wherever an asset context is absent.
- **`Money`** — `{ base_units: Amount, asset: Option<AssetMeta> }`. Always exposes
  exact `base_units()`; `to_decimal()` and `symbol()` return `Option` (None when the
  asset is unresolved). Optionality is pushed to the derived views, never the exact
  value.

```rust
struct Money { base_units: Amount, asset: Option<AssetMeta> }
// base_units() -> &Amount (always); to_decimal() -> Option<Decimal>; symbol() -> Option<&str>
```

### D8. Resource identifiers — typed newtypes (decided via interview)

`VaultId`, `WalletId`, `AssetId`, `AddressId`, etc. as distinct newtypes implementing
`AsRef<str>`/`From<String>`, so they pass into generated `&str` setters with no
adaptation while preventing id-type mix-ups at compile time. The facade methods accept
these; the AIP resource-name form (`vaults/{id}`) is constructed internally.

### D9. Pagination — raw page + Stream (decided via interview)

Each List group exposes both: a single-page builder (`.list().page_size(n)
.page_token(t).send()`) for explicit control, and `.stream()` returning
`impl Stream<Item = Result<T, UtilaError>>` that walks `pageToken` to exhaustion
(`.try_collect()` for all-at-once). Applies to the 6 List endpoints.

### D10. Balance auto-enrichment — batched, cached (decided via interview)

`balances().query(VaultId)` returns `Vec<Balance>` where each `Balance.money` is a
`Money`. Enrichment: collect the distinct asset ids in the response, resolve their
`{decimals, symbol}` via **one batched `assets:batchGet`**, and cache them for the
process lifetime (decimals are immutable per asset). If an asset cannot be resolved,
`Money` still carries exact `base_units` and its `to_decimal()`/`symbol()` return
`None` (per D6) — the query never fails over an unlisted/quirky asset. The cache is
`Send + Sync`; lookups are keyed by asset id.

### D7. Pagination — Stream over pageToken

The 6 List endpoints (`ListVaults`, `ListTransactions`, `ListWallets`,
`ListWalletAddresses`, `ListNetworks`/`ListVaultNetworks`, `ListAddressBookEntries`)
use `pageSize`/`pageToken`. Provide an async `Stream<Item = Result<T, UtilaError>>`
adapter that walks `next_page_token` until exhausted, plus raw single-page access.

### D11. Transaction initiation — per-kind `send_*` terminals (revised)

`details` is a 14-variant protobuf `oneof` the API renders as one all-`Option` struct.
Per D0 it becomes a real **`TransactionDetails` enum** (one variant per message:
`AssetTransfer`, `BatchAssetTransfer`, `EvmTransaction`, `EvmPersonalSign`,
`EvmSignTypedDataV4`, `EvmAccountDelegation`, `ExchangeWithdrawal`,
`SolanaSerializedTransaction`, `StellarTransaction`, `StellarRawTransaction`,
`SuiRawTransaction`, `TronTransaction`, `TronTriggerSmartContract`,
`XrplRawTransaction` — 14 total), serialized to exactly one field. We hand-map the
variants (drift risk on new ones; the regen diff surfaces them). `TransactionDetails`
stays the single internal representation that `map_details` feeds to the wire; the
public surface is the per-kind methods below (the enum need not appear in a call site).

**Reversed from the earlier draft** (which had one `initiate(vault, TransactionDetails)`
builder + `IntoFuture` + a `SendAmount` type): the write surface is now **one method per
kind**, and `send_*` is the *terminal* that produces the future.

- **`client.transactions().<kind>(vault, value)` returns a modifier builder; `send()`
  is terminal.** The body's independent optional modifiers (`priority`, `note`,
  `designatedSigners`, `validateOnly`, `runSimulation`, `externalId`, `expireTime`,
  `includeReferencedResources`, `requestId`) are builder setters that come **before**
  `send()`:

  ```rust
  client.transactions()
      .tron(vault, tron)         // or .evm(vault, evm), .asset_transfer(vault, t), …
      .priority(Priority::High)  // all optional — fine to skip
      .request_id(idem)
      .send().await?;            // terminal; fails ONLY on real API/transport errors
  ```

- **Required inputs are never validated inside `send()`** (no "failed future because you
  forgot a field"). Required-ness is enforced *upstream of `send()`*:
  - **flat kinds** (EVM tx/sign, exchange withdrawal, asset/batch transfer, the raw-blob
    chains) are plain structs with non-`Option` required fields — a struct literal cannot
    omit them;
  - **deeply-nested kinds** (`TronTransaction` with its 11 sub-action `oneof`,
    `StellarTransaction` with operations/memo/time-bounds) use a `::builder()…​.build()?`
    whose **`Result` is the single explicit place** a missing field is reported, distinct
    from the network call. `send()` therefore only ever fails for API/transport reasons.
- **Modifier state lives on the group** (`Transactions` *is* the builder): the read
  methods ignore the dormant `Option` modifier fields, so there is no `.initiate()`
  gateway word and the 14 `send_*` terminals are not duplicated across two types — they
  funnel through one private `submit(vault, opts, details)`. (Considered: a physically
  separate builder with an `.initiate()` gateway; rejected — the extra word buys little
  and a no-gateway separate builder would double the trivial terminals under the
  per-function coverage floor.)
- **Amounts are display units** (`"1.5"`, asset-implied) — NOT base units like balances.
  Carried as plain `rust_decimal::Decimal` on the transfer structs (the earlier
  `SendAmount` newtype was dropped; the field name + docs carry the display-unit
  contract).
- `estimate_fee` reuses the same `TransactionDetails` mapping for the subset the
  estimation endpoint accepts.

### D12. Idempotency (decided via interview)

`requestId` is a UUID the server deduplicates for ≥60 min. The client **auto-generates**
one per `initiate` and surfaces it on the result; callers may **override** via
`.request_id(uuid)` to make a retry idempotent. Adds a `uuid` dependency. `externalId`
remains a separate, caller-owned correlation id (plain optional).

### D13. Webhook verification (decided via interview)

Receiver-side only (no config API in v2). A framework-agnostic
`webhook::verify(body, signature) -> Result<VerifiedEvent>` implementing the documented
scheme: `x-utila-signature` header, **RSA-4096 / SHA-512 / PSS**, base64, verified over
the raw body against Utila's published webhook public key (bundled as default,
overridable). `VerifiedEvent::parse()` deserializes into a typed `Event` enum. Note this
is a *different* RSA scheme from auth signing (PKCS#1-v1.5 / SHA-256, D4) — the `rsa`
crate covers both.

### D14. Retry / settings segmented by operation purpose (decided via interview)

No global auto-retry by default; retry is **configurable and segmented by op class**
(D0): read operations and create/mutate operations carry independent settings.

```rust
UtilaClient::builder()
    .read_settings(OpSettings::default().retry(RetryPolicy::exp(3)))   // GET: safe to retry
    .write_settings(OpSettings::default().retry(RetryPolicy::none()))  // mutations: opt-in only
    .build()?;
```

`OpSettings` holds retry policy, timeout, and similar per-class knobs. Writes are never
auto-retried unless the caller opts in *and* the op is idempotent (initiate via D12).
`RetryPolicy` honors `Retry-After` on 429. Auth/token refresh is client-wide (D4),
applied to every class.

## Risks / Open Questions

- **R1** Spec is generated and may change without notice → mitigated by vendoring +
  CI drift check; regen is a reviewable diff.
- **R2** `aud` trailing-slash mismatch is an easy, silent auth bug → covered by an
  explicit unit test asserting the exact audience constant.
- **R3** Large-balance Decimal overflow → mitigated by exact-integer source of truth
  (D6); Decimal is display-only.
- **R4** Token clock skew (only `exp` sent) → 5-minute refresh margin + 401
  invalidation.
- **Q1** Confirm crate name / license / KMS-in-v1 before publish.
