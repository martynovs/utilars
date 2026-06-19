# Tasks: Utila Rust API Client

Status legend: `[x]` done · `[~]` partial · `[ ]` not started.
Notes (→) record where the build intentionally diverged from the original plan.

## Phase 0 — Scaffolding & generation harness
- [x] 0.1 Cargo package + `xtask`; edition, `Apache-2.0 OR MIT`.
  → Diverged: **one published crate** `utila-api-client` (lib name `utila`) with `xtask` as a
    dev-only workspace member — not a two-crate `utila` + `xtask` workspace.
- [x] 0.2 `xtask pull`: OpenAPI URL → committed `openapi/utila.v2.json`.
- [x] 0.3 `xtask gen`: vendored spec → `progenitor::Generator` (Builder + Separate tags + derives) → `src/generated.rs`.
  → No rustfmt step; `#[rustfmt::skip] mod generated;` keeps fmt off the generated file.
- [x] 0.4 `with_inner_type` (`Arc<crate::auth::TokenManager>`) + `with_pre_hook_async` (`crate::auth::inject_bearer`) wired.
  → `with_post_hook` placeholder not wired (see 1.6).
- [x] 0.5 CI: regen and fail on `git diff` drift; run `clippy`/`fmt`.
  → `.github/workflows/ci.yml`: `clippy`/`fmt`/`test`/`msrv`/`crap` jobs, plus a `gen-drift` job
    that runs `cargo xtask gen` (hermetic — reads the committed spec, no `pull`) and
    `git diff --exit-code` on `src/generated.rs` + `openapi/utila.v2.meta.json`. Added a
    `.cargo/config.toml` `xtask` alias so the README-documented `cargo xtask gen` actually
    resolves (locally and in CI) — previously only `cargo run -p xtask --` worked.

## Phase 1 — Authentication
- [x] 1.0 `SignerSource` enum of validated values (`Local(EncodingKey)` | `Kms(KmsKey)`); `KmsKey::Aws(AwsKmsArn)` typed+validated; redacting `Debug`.
- [x] 1.1 **Signer abstraction landed**: async `Signer` trait (`sign(&[u8]) -> Result<String>`,
  native async fn, static dispatch — no `async-trait`/`dyn`). `LocalRsaSigner` (jsonwebtoken
  `crypto::sign`) + `AwsKmsSigner` (feature `aws`). `TokenManager` now assembles the JWT
  manually (`base64url(header).claims.sig`) and dispatches signing to the backend.
- [x] 1.2 Local RSA (RS256) from PEM via `SignerSource::local_pem`; tested.
- [x] 1.3 JWT assembler: header + claims (`sub`/`aud`/`exp`), base64url, signature.
- [x] 1.4 Test asserting `aud == "https://api.utila.io/"` (trailing slash) exactly.
- [x] 1.5 `TokenManager`: cache `{jwt, exp}`, refresh at `exp - 5min`.
  → **Single-flight now real**: minting happens while holding the tokio write guard, so
    concurrent refreshes coalesce to one signature (matters for billed KMS). `mint()` covered.
- [x] 1.6 Auth hooks.
  → `inject_bearer` pre-hook + `on_response` post-hook (`with_post_hook_async`, regenerated):
    a `401 Unauthorized` drops the cached token so the next request re-mints (clock skew /
    rotated key / revocation). Never fails the request; 2xx and transport errors are no-ops.
    Tested across all three branches.
- [x] 1.7 **`aws` feature + AWS KMS signer** (`RSASSA_PKCS1_V15_SHA_256` via `aws-sdk-kms`,
  `MessageType::Raw`; lazily-built shared client keyed by the ARN's region). Off by default;
  without it the KMS path returns a typed error pointing at the flag. Compiles clean under
  `--features aws`; signer code is `cfg`-gated so the default coverage floor is unaffected.

## Phase 2 — Facade, typed IDs & errors
- [x] 2.1 `UtilaClient` wraps the generated `Client`. **FORK RESOLVED**: the reqwest
    transport is gone; `vaults`/`balances`/`transactions` call the generated `*Ext` ops
    (auth pre-hook fires inside each), funnel the result through `recv` which maps
    `Error<()>` → `UtilaError`. Each call issues exactly one request — **retry is external**
    (see 5b).
- [x] 2.2 `UtilaClientBuilder` (`.credential(account, signer)` / `.base_url` / `.timeout` /
  `.connect_timeout` — timeouts wired into the generated client's `reqwest::Client` via
  `new_with_client`). No `from_env` — a library must not read `std::env`; the caller supplies
  the account + signer (reading its own env/secrets) and passes them to the builder.
- [x] 2.3 Typed-ID newtypes (`VaultId`, `WalletId`, `AssetId`, `AddressId`, `NetworkId`,
  `TransactionId`, `AddressBookEntryId`).
- [x] 2.4 **Full curated facade — all 42 ops across 7 groups**: `vaults()` (2), `wallets()` (12),
  `transactions()` (11), `balances()` (5), `address_book()` (5), `networks()` (4), `assets()` (3).
  Every op returns a hand-mapped curated type (no raw `types::V2*` in the public surface). Built
  via a 5-way workflow fan-out off the `networks()` reference pattern; integrated + gated serially.
- [x] 2.5 `UtilaError::Api { code, message, details }` parsed from the gRPC status body.
  → `details: Vec<serde_json::Value>` now surfaced verbatim from `google.rpc.Status.details`.
    Synthetic "field missing" errors centralized in `UtilaError::missing()` (empty details).
- [x] 2.6 Map transport/auth/deserialization failures into `UtilaError` (`map_error`: auth
  pre-hook → `Auth`, transport → `Http`, payload/status → `Api`); unit-tested.

## Phase 3 — Amounts & assets
- [x] 3.1 `Amount` newtype over base units (`u128`, errors on adversarial/overflow); round-trips.
- [x] 3.2 ~~`Money { base_units, asset: Option<AssetMeta> }`~~ → **superseded** by the `Asset`
  enum (`Resolved(ResolvedAsset)` | `Unresolved(AssetId)`) — makes the "no decimals known" state a
  variant instead of an `Option`. Transfers use plain `rust_decimal::Decimal`.
- [x] 3.3 `ResolvedAsset { decimals, symbol }` + `Send + Sync` `AssetCache` (`Mutex<HashMap>`, process-lifetime).
- [x] 3.4 `balances().query()` enrichment: collect ids → one `assets:batchGet` → cache → attach; unresolved degrade, never fail.

## Phase 4 — Pagination
- [x] 4.1 Single-page builder: `.list().page_size(n).page_token(t).send()`.
  → `filter` dropped from `vaults().list()` — the generated `ListVaults` op has no filter param.
- [x] 4.2 `.stream()` → `Stream<Item = Result<T>>` over `pageToken`; `.try_collect()`.
- [x] 4.3 Applied to every list endpoint (vaults, wallets, wallet addresses, transactions,
  networks, vault networks, address book, balances variants, UTXOs) — `.send()` + `.stream()`.
- [x] 4.4 Stream tests: multi-page walk, missing-token, **empty page (yields nothing)**, and
  **mid-stream error** (page-1 items surface, then the page-2 failure is yielded as `Err` —
  asserted via `[Ok(_), Err(_)]` ordering, not swallowed). Exercised on the vaults stream
  (representative of the shared `try_unfold` pagination pattern).

## Phase 5 — Transactions & idempotency
- [x] 5a.1 **All 14 `details` variants curated** in `src/tx_details.rs` (`TransactionDetails`
  enum → generated `details` via `map_details`, sets exactly one field; `details.len()==1`
  asserted per variant). Flat/blob kinds are plain structs; the two deeply-nested kinds
  (`TronTransaction` — 9 staking actions; `StellarTransaction` — operations/memo/time-bounds
  with a `raw` op escape) use `::builder()…​.build()?`. Public surface is the **per-kind
  methods** on `transactions()` (`tron`/`evm`/`asset_transfer`/… → modifier builder →
  terminal `.send().await`); see revised D11. `estimate_fee` reuses the mapping for the
  subset the endpoint accepts.
- [x] 5a.2 ~~`SendAmount`~~ → **dropped**; transfer amounts are plain `rust_decimal::Decimal`.
- [x] 5a.3 `initiate(vault_id, details)` builder with independent optional setters.
  → Diverged: explicit `.send().await` only; `IntoFuture` removed (more readable per review).
- [x] 5a.4 Idempotency: auto-generate `requestId` UUID, surface on result, `.request_id()` override.

## Phase 5b — Retry (revised: external, not built in)
- [x] 5b.1 **Reversed D14.** Retry is no longer in the client. Rationale: progenitor's
  generated `Client` owns a concrete `reqwest::Client` (no middleware seam), and op class
  can't be derived from HTTP method (`queryBalances` is a read over POST) — so a built-in
  loop would be both awkward and reinventing a solved problem. Dropped `OpSettings`,
  `OpClass`, `RetryPolicy`, and the `run`/`backoff` machinery.
- [x] 5b.2 The library's contribution is **`UtilaError::is_retryable()`** — classifies
  transient transport + gRPC/HTTP server errors (4/8/14, 429, 5xx), shaped as
  `fn(&UtilaError) -> bool`. Callers wrap a whole operation with a retry crate
  (**recommend [`backon`](https://docs.rs/backon)**, the RustSec-endorsed successor to the
  unmaintained `backoff`) and gate on it: `op.retry(..).when(UtilaError::is_retryable)`.
  Documented in the crate docs with a runnable example.
- [n/a] 5b.3 Read-vs-write retry policy — now the caller's choice (they pick what to wrap),
  which also resolves the POST-but-read ambiguity cleanly.

## Phase 5c — Webhooks (receiver-side)
- [x] 5c.1 `webhook::verify(body, signature)` + `verify_with_key` — RSA-4096 / SHA-512 / PSS
  (salt = digest, matching Utila's `RSA_PSS_SALTLEN_DIGEST`) over the raw body via the `rsa`
  crate; Utila's public key bundled as `UTILA_WEBHOOK_PUBLIC_KEY` (overridable).
- [x] 5c.2 Typed `Event` { id, vault, kind: `EventKind`, resource(_type), details } +
  `VerifiedEvent::parse()` (verify and parse split; unknown event types → `EventKind::Unknown`).
- [x] 5c.3 Tests: authentic round-trip (sign with the committed test key) / tampered body /
  wrong key / bad-base64 / bad PEM / each event kind / verified-but-non-JSON. Bundled key
  asserted to parse as a valid RSA public key.

## Phase 6 — Polish & publish
- [x] 6.1 Demo CLI example — binary `utila` (source `examples/cli.rs`, `cargo run --example
  utila -- …`). Subcommands exercise the real surfaces: vaults (get + stream), wallets,
  enriched balances, transactions (list + get), `transfer` (per-kind `asset_transfer` +
  idempotency), networks, assets, `verify-webhook` (stdin body). Reads creds from the env
  (`UTILA_ACCOUNT`, `UTILA_PRIVATE_KEY_PATH` / `UTILA_KMS_KEY_URL` for KMS under `--features
  aws`) — modeling the "app handles env, library doesn't" pattern. clippy-clean under the
  pedantic+restriction lints (no unwrap/index/panic; anyhow `?` throughout).
- [x] 6.2 **README + crate docs + LICENSE files.** `README.md` (callout for the
  `utila-api-client` → `use utila` split; quickstart; auth + `aws` feature; grouped facade;
  `transfer` vs `send` + sponsored transfers; pagination; webhooks; external retry; errors;
  security note on the `rsa` advisory; MSRV 1.82). `LICENSE-APACHE` + `LICENSE-MIT` added
  (the `Apache-2.0 OR MIT` the manifest already declared, referenced by `include`).
  Crate-level rustdoc refreshed (dropped "strawman" + the removed `Money` link; fixed the
  broken `crate::retry` link), `readme`/`documentation` manifest fields set → the
  "no documentation" publish warning is gone. `cargo doc` builds clean; package now ships
  26 files and build-verifies.
- [x] 6.3a Name + license decided: package `utila-api-client`, lib `utila`, `Apache-2.0 OR MIT`.
- [~] 6.3b `cargo publish --dry-run` run; **package contents fixed** + **KMS scope confirmed**.
  → The dry-run was shipping dev junk (`.claude/`, `.vscode/`, `openspec/`, `CLAUDE.md`,
    `justfile`, clippy/CRAP configs); added an `include` whitelist → `cargo package` now ships
    **23 files** (`src/` + `examples/cli.rs`) and **build-verifies** clean. No `openapi/` is
    shipped — the spec is codegen-only and its provenance already rides in the `generated.rs`
    header; the excluded `tests/` are `#[cfg(test)]`-only, so a consumer build never needs them.
    KMS scope confirmed: **0** `aws-sdk-kms` in the default dep tree, present only under
    `--features aws`. Remaining before a real publish (deferred): flip `publish = false`, add
    README/LICENSE-APACHE/LICENSE-MIT + `repository`, settle the version.
- [x] 6.4 **Generated-code provenance** in `xtask`: `provenance()` derives source URL, spec
  title/`version` and a **sha256 of the committed spec** (the real revision id — the spec has
  no date and only a coarse `v2`). `pull`/`gen` write `openapi/utila.v2.meta.json` (repo-only),
  and `gen` stamps a `// spec: Utila API v2 (openapi 3.0.0) · sha256:137f4fd12f4b · source: …`
  line into `src/generated.rs` (the shipped provenance). Deterministic (same spec → same hash).

## Verification
- [~] V1 `cargo test` green (25 passing) + `clippy` clean (0 warnings).
  → Workspace lints are `warn`, not `-D warnings`; CI should flip to deny.
- [ ] V2 Live smoke test against a sandbox service account (gated, manual).
- [x] V3 Spec-drift CI check on a clean regen (same as 0.5) — the `gen-drift` job above.
