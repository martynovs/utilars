//! A small demo CLI exercising the `utilars` client against the real v2 API.
//!
//! This is also a worked example of the intended credential pattern: the **library never
//! reads the environment** — the *application* (this CLI) reads its own secrets/config and
//! passes them to the builder.
//!
//! ## Zero-config default
//!
//! With no env vars set, the CLI falls back to a local service-account dir written by the
//! user (the same `~/.config/utila` the Utila CLI uses), so it "just works":
//!
//! ```text
//! ~/.config/utila/service-account/
//! ├── email             # the service-account email  → JWT `sub`
//! └── private_key.pem   # the RSA signing key         → local RS256 signer
//! ```
//!
//! Resolved via `$XDG_CONFIG_HOME/utila` (falling back to `$HOME/.config/utila`) — *not*
//! the platform config dir, which diverges on macOS. Any env var below overrides the dir.
//!
//! ## Run it
//!
//! ```text
//! # Either rely on ~/.config/utila/service-account/ (no env vars), or set them explicitly:
//! export UTILA_ACCOUNT='my-sa@vault-xxxxxxxx.utilaserviceaccount.io'
//! export UTILA_PRIVATE_KEY_PATH=/path/to/service-account-key.pem   # local RSA signing (file)
//! # …or the PEM inline (raw, `\n`-escaped, or base64-encoded):
//! # export UTILA_PRIVATE_KEY_PEM="$(base64 < key.pem)"
//! # …or the AWS KMS signer (build with `--features aws`):
//! # export UTILA_KMS_KEY_URL='awskms:///arn:aws:kms:us-east-1:123:key/abc'
//! # optional: export UTILA_BASE_URL='https://api.utila.io'
//!
//! cargo run --example utilars -- vaults
//! cargo run --example utilars -- balances <vault-id>
//! cargo run --example utilars -- transfer <vault-id> \
//!     --asset assets/native.ethereum-mainnet \
//!     --from vaults/<v>/wallets/<w> --to 0xabc... --amount 1.5
//! echo -n '<raw-body>' | cargo run --example utilars -- verify-webhook --signature <base64>
//! ```

use std::io::Read as _;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use futures::TryStreamExt as _;
use rust_decimal::Decimal;
use serde_json::{json, Value};
use utilars::webhook::Event;
use utilars::{
    Amount, AssetId, AssetRef, AssetTransfer, Balance, EvmTransaction, InitiateBuilder, Network,
    NetworkId, ParseRef, ResolvedAsset, SignerSource, SolanaRaw, StellarRaw, SuiRaw, Transaction,
    TransactionId, TronTriggerSmartContract, UtilaClient, Vault, VaultId, Wallet, WalletAddress,
    WalletId, XrplRaw,
};

/// Accept an asset id with or without the `assets/` prefix — the bare segment is canonical, but a
/// pasted resource name (`assets/native.ethereum-mainnet`) is normalized to it.
fn parse_asset(s: String) -> AssetId {
    AssetRef::parse(&s).map_or_else(|| AssetId::new(s), AssetId::from)
}

#[derive(Parser)]
#[command(
    name = "utilars-cli",
    about = "Demo CLI exercising the utilars Utila API client"
)]
struct Cli {
    /// Emit machine-readable JSON instead of human tables.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List the blockchain networks Utila supports.
    Networks,
    /// List metadata for one or more assets by id (batch-get).
    Assets {
        /// One or more asset ids; the `assets/` prefix is optional
        /// (`native.ethereum-mainnet` or `assets/native.ethereum-mainnet`).
        #[arg(required = true)]
        ids: Vec<String>,
    },
    /// Get one vault by id.
    Vault { id: String },
    /// List every vault (walks all pages via the stream API).
    Vaults,
    /// List the wallets in a vault.
    Wallets { vault: String },
    /// List the addresses of a wallet.
    Addresses { vault: String, wallet: String },
    /// Create a new address in a wallet for a network.
    CreateAddress {
        vault: String,
        wallet: String,
        /// Network id, e.g. `ethereum-mainnet` (the segment after `networks/`).
        #[arg(long)]
        network: String,
        #[arg(long)]
        display_name: Option<String>,
        #[arg(long)]
        note: Option<String>,
    },
    /// Query a vault's balances (asset metadata resolved + enriched).
    Balances { vault: String },
    /// List a vault's recent transactions (one page).
    Transactions {
        vault: String,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Get one transaction by id.
    Transaction { vault: String, id: String },
    /// Move tokens — cross-chain `assetTransfer`, the normal path (auto idempotency key).
    ///
    /// You give an asset + amount + source/destination; Utila picks the chain from the asset
    /// and builds the on-chain transaction for you. Works on every network with no
    /// chain-specific input. For contract calls or pre-built transactions, use `send` instead.
    Transfer {
        /// Vault id (the segment after `vaults/`).
        vault: String,
        /// Asset id; the `assets/` prefix is optional, e.g. `native.ethereum-mainnet`.
        #[arg(long)]
        asset: String,
        /// Source: a wallet resource name (`vaults/<v>/wallets/<w>`) or an address.
        #[arg(long)]
        from: String,
        /// Destination address or resource name.
        #[arg(long)]
        to: String,
        /// Amount in display units (whole tokens), e.g. `1.5`.
        #[arg(long)]
        amount: Decimal,
        /// Optional note recorded on the transaction.
        #[arg(long)]
        note: Option<String>,
        /// Sponsor (gas) wallet to pay the network fee — a native sponsored transfer.
        #[arg(long)]
        sponsor: Option<String>,
        /// EVM-only: pay the network fee out of the transfer amount.
        #[arg(long)]
        pay_fee_from_amount: bool,
    },
    /// Send a network-specific transaction you build yourself — one example per network.
    ///
    /// Lower-level than `transfer`: you hand Utila a chain-specific transaction (EVM calldata,
    /// a Tron contract call, or a pre-serialized Solana/Stellar/Sui/XRPL blob) and it signs
    /// and submits it. Use when `transfer` can't express what you need; otherwise use `transfer`.
    Send {
        #[command(subcommand)]
        kind: SendKind,
    },
    /// Verify a webhook signature over the raw body read from stdin.
    VerifyWebhook {
        #[arg(long)]
        signature: String,
    },
}

/// A network-specific transaction kind. (`transfer` above is the *cross-chain* asset move;
/// these are the per-network transaction types.) Pre-serialized chains take a raw blob.
#[derive(Subcommand)]
enum SendKind {
    /// A raw EVM transaction.
    Evm {
        /// Vault id (the segment after `vaults/`).
        vault: String,
        /// Sender address.
        #[arg(long)]
        from: String,
        /// Network id, e.g. `ethereum-mainnet`.
        #[arg(long)]
        network: String,
        /// Recipient / contract address.
        #[arg(long)]
        to: Option<String>,
        /// Value in wei.
        #[arg(long)]
        value: Option<String>,
        /// Hex calldata.
        #[arg(long)]
        data: Option<String>,
    },
    /// A Tron smart-contract call (`triggerSmartContract`).
    Tron {
        /// Vault id (the segment after `vaults/`).
        vault: String,
        /// Network id, e.g. `tron-mainnet`.
        #[arg(long)]
        network: String,
        /// Caller (owner) address.
        #[arg(long)]
        owner: String,
        /// Target contract address.
        #[arg(long)]
        contract: String,
        /// Hex-encoded call data.
        #[arg(long)]
        data: Option<String>,
        /// TRX to send with the call, in SUN.
        #[arg(long)]
        call_value: Option<String>,
    },
    /// A pre-serialized Solana transaction (base64).
    Solana {
        /// Vault id (the segment after `vaults/`).
        vault: String,
        /// Network id, e.g. `solana-mainnet`.
        #[arg(long)]
        network: String,
        /// Base64-encoded serialized transaction.
        #[arg(long)]
        raw_tx: String,
    },
    /// A pre-built Stellar transaction envelope (base64 XDR).
    Stellar {
        /// Vault id (the segment after `vaults/`).
        vault: String,
        /// Network id, e.g. `stellar-mainnet`.
        #[arg(long)]
        network: String,
        /// Source account address.
        #[arg(long)]
        source: String,
        /// Base64 XDR transaction envelope.
        #[arg(long)]
        xdr: String,
    },
    /// A pre-serialized Sui transaction (base64 BCS bytes).
    Sui {
        /// Vault id (the segment after `vaults/`).
        vault: String,
        /// Network id, e.g. `sui-mainnet`.
        #[arg(long)]
        network: String,
        /// Sender address.
        #[arg(long)]
        sender: String,
        /// Base64-encoded BCS transaction bytes.
        #[arg(long)]
        bcs: String,
    },
    /// A raw XRPL transaction.
    Xrpl {
        /// Vault id (the segment after `vaults/`).
        vault: String,
        /// Network id, e.g. `xrpl-mainnet`.
        #[arg(long)]
        network: String,
        /// Sender address.
        #[arg(long)]
        sender: String,
        /// The transaction as a JSON object.
        #[arg(long)]
        json: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    // The only command that needs no client / credentials.
    if let Command::VerifyWebhook { signature } = &cli.command {
        return verify_webhook(signature);
    }
    let client = build_client()?;
    run(&client, cli.command, cli.json).await
}

/// Build a client from configuration — the application's job, not the library's. Account and
/// signer each prefer an env var and fall back to the local `service-account/` config dir.
fn build_client() -> Result<UtilaClient> {
    let dir = service_account_dir();
    let account = account_from_env_or_config(dir.as_deref())?;
    let signer = signer_from_env_or_config(dir.as_deref())?;

    let mut builder = UtilaClient::builder()
        .credential(account, signer)
        .timeout(Duration::from_secs(30));
    if let Ok(base) = std::env::var("UTILA_BASE_URL") {
        builder = builder.base_url(base);
    }
    builder.build().context("building the client")
}

/// The local Utila service-account config dir: `$XDG_CONFIG_HOME/utila/service-account`,
/// falling back to `$HOME/.config/utila/service-account`. Deliberately *not* the platform
/// config dir (`dirs::config_dir()`), which on macOS resolves to `~/Library/Application
/// Support` — the Utila CLI uses `~/.config` on every platform. `None` if neither var is set.
fn service_account_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("utila").join("service-account"))
}

/// Resolve the service-account email: `UTILA_ACCOUNT`, else the `email` file in the config dir.
fn account_from_env_or_config(dir: Option<&std::path::Path>) -> Result<String> {
    if let Ok(account) = std::env::var("UTILA_ACCOUNT") {
        return Ok(account);
    }
    if let Some(path) = dir.map(|d| d.join("email")).filter(|p| p.exists()) {
        let email = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let email = email.trim();
        if !email.is_empty() {
            return Ok(email.to_string());
        }
    }
    anyhow::bail!(
        "no account: set UTILA_ACCOUNT or write the email to <config>/utila/service-account/email"
    )
}

/// Resolve a signer, in priority order: `UTILA_KMS_KEY_URL` → `UTILA_PRIVATE_KEY_PEM` (inline)
/// → `UTILA_PRIVATE_KEY_PATH` (file) → `private_key.pem` in the local service-account config dir.
/// Demonstrates that a local PEM can come straight from an env var, not just a file.
fn signer_from_env_or_config(dir: Option<&std::path::Path>) -> Result<SignerSource> {
    if let Ok(url) = std::env::var("UTILA_KMS_KEY_URL") {
        return kms_signer(&url);
    }
    if let Ok(value) = std::env::var("UTILA_PRIVATE_KEY_PEM") {
        let pem = decode_pem_var(&value)?;
        return SignerSource::local_pem(&pem).context("parsing UTILA_PRIVATE_KEY_PEM");
    }
    if let Ok(path) = std::env::var("UTILA_PRIVATE_KEY_PATH") {
        let pem = std::fs::read(&path).with_context(|| format!("reading key file {path}"))?;
        return SignerSource::local_pem(&pem).context("parsing the RSA private key");
    }
    if let Some(path) = dir
        .map(|d| d.join("private_key.pem"))
        .filter(|p| p.exists())
    {
        let pem = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        return SignerSource::local_pem(&pem)
            .with_context(|| format!("parsing {}", path.display()));
    }
    anyhow::bail!(
        "no signer: set UTILA_KMS_KEY_URL, UTILA_PRIVATE_KEY_PEM, or UTILA_PRIVATE_KEY_PATH, \
         or place a private_key.pem in <config>/utila/service-account/"
    )
}

/// Decode an inline `UTILA_PRIVATE_KEY_PEM`: a raw PEM (with real or `\n`-escaped newlines) is
/// used as-is; anything else is treated as base64-encoded PEM (the common single-line form for
/// secret stores). PEM is multi-line, so storing it as base64 sidesteps env-var newline pain.
fn decode_pem_var(value: &str) -> Result<Vec<u8>> {
    if value.contains("-----BEGIN") {
        Ok(value.replace("\\n", "\n").into_bytes())
    } else {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD
            .decode(value.trim())
            .context("UTILA_PRIVATE_KEY_PEM is neither raw PEM nor valid base64")
    }
}

#[cfg(feature = "aws")]
fn kms_signer(url: &str) -> Result<SignerSource> {
    let key = utilars::KmsKey::parse(url).context("parsing UTILA_KMS_KEY_URL")?;
    Ok(SignerSource::Kms(key))
}

#[cfg(not(feature = "aws"))]
fn kms_signer(_url: &str) -> Result<SignerSource> {
    anyhow::bail!("UTILA_KMS_KEY_URL is set but the CLI was built without `--features aws`")
}

#[expect(
    clippy::too_many_lines,
    reason = "flat per-subcommand dispatch; clearer than extracting one-liner handlers"
)]
async fn run(client: &UtilaClient, command: Command, json: bool) -> Result<()> {
    match command {
        Command::Networks => {
            let networks: Vec<_> = client.networks().stream().try_collect().await?;
            let items: Vec<Value> = networks.into_iter().map(network_json).collect();
            emit_list(
                json,
                &[
                    ("NETWORK ID", "id"),
                    ("DISPLAY NAME", "display_name"),
                    ("NATIVE ASSET", "native_asset"),
                    ("STATUS", "status"),
                    ("TESTNET", "testnet"),
                ],
                &items,
            )?;
            footer(json, items.len(), "network");
        }
        Command::Assets { ids } => {
            let assets = client
                .assets()
                .batch_get(ids.into_iter().map(parse_asset))
                .await?;
            let items: Vec<Value> = assets.into_iter().map(asset_json).collect();
            emit_list(
                json,
                &[
                    ("ASSET ID", "id"),
                    ("SYMBOL", "symbol"),
                    ("DECIMALS", "decimals"),
                ],
                &items,
            )?;
            footer(json, items.len(), "asset");
        }
        Command::Vaults => {
            let vaults: Vec<_> = client.vaults().stream().try_collect().await?;
            let items: Vec<Value> = vaults.into_iter().map(vault_json).collect();
            emit_list(
                json,
                &[
                    ("VAULT ID", "id"),
                    ("DISPLAY NAME", "display_name"),
                    ("ARCHIVED", "archived"),
                    ("CREATED", "created"),
                ],
                &items,
            )?;
            footer(json, items.len(), "vault");
        }
        Command::Vault { id } => {
            let vault = client.vaults().get(VaultId::new(id)).await?;
            emit_one(&vault_json(vault))?;
        }
        Command::Wallets { vault } => {
            let wallets: Vec<_> = client
                .wallets()
                .list(VaultId::new(vault))
                .stream()
                .try_collect()
                .await?;
            let items: Vec<Value> = wallets.into_iter().map(wallet_json).collect();
            emit_list(
                json,
                &[
                    ("WALLET ID", "id"),
                    ("DISPLAY NAME", "display_name"),
                    ("NETWORKS", "networks"),
                    ("ARCHIVED", "archived"),
                ],
                &items,
            )?;
            footer(json, items.len(), "wallet");
        }
        Command::Addresses { vault, wallet } => {
            let addresses: Vec<_> = client
                .wallets()
                .list_addresses(VaultId::new(vault), WalletId::new(wallet))
                .stream()
                .try_collect()
                .await?;
            let items: Vec<Value> = addresses.into_iter().map(wallet_address_json).collect();
            emit_list(
                json,
                &[
                    ("ADDRESS ID", "id"),
                    ("ADDRESS", "address"),
                    ("NETWORK", "network"),
                    ("DISPLAY NAME", "display_name"),
                    ("TYPE", "type"),
                ],
                &items,
            )?;
            footer(json, items.len(), "address");
        }
        Command::CreateAddress {
            vault,
            wallet,
            network,
            display_name,
            note,
        } => {
            let mut builder = client.wallets().create_address(
                VaultId::new(vault),
                WalletId::new(wallet),
                NetworkId::new(network),
            );
            if let Some(name) = display_name {
                builder = builder.display_name(name);
            }
            if let Some(note) = note {
                builder = builder.note(note);
            }
            let address = builder.send().await?;
            emit_one(&wallet_address_json(address))?;
        }
        Command::Balances { vault } => {
            let balances = client.balances().query(VaultId::new(vault)).await?;
            let items: Vec<Value> = balances.into_iter().map(balance_json).collect();
            emit_list(
                json,
                &[
                    ("AMOUNT", "amount"),
                    ("FROZEN", "frozen"),
                    ("SYMBOL", "symbol"),
                    ("ASSET", "asset"),
                ],
                &items,
            )?;
            footer(json, items.len(), "balance");
        }
        Command::Transactions { vault, limit } => {
            let page = client
                .transactions()
                .list(VaultId::new(vault))
                .page_size(limit)
                .send()
                .await?;
            let total = page.total_size;
            let items: Vec<Value> = page
                .transactions
                .into_iter()
                .map(transaction_json)
                .collect();
            emit_list(
                json,
                &[
                    ("TRANSACTION ID", "id"),
                    ("NETWORK", "network"),
                    ("STATE", "state"),
                    ("KIND", "kind"),
                    ("CREATED", "created"),
                ],
                &items,
            )?;
            if !json {
                println!("— {} of {total} transaction(s)", items.len());
            }
        }
        Command::Transaction { vault, id } => {
            let tx = client
                .transactions()
                .get(VaultId::new(vault), TransactionId::new(id))
                .await?;
            emit_one(&transaction_json(tx))?;
        }
        Command::Transfer {
            vault,
            asset,
            from,
            to,
            amount,
            note,
            sponsor,
            pay_fee_from_amount,
        } => {
            let mut builder = client.transactions().asset_transfer(
                VaultId::new(vault),
                AssetTransfer {
                    asset: parse_asset(asset),
                    source: from.into(),
                    destination: to.into(),
                    amount,
                    memo: note.clone(),
                    sponsor: sponsor.map(Into::into),
                    // Send the flag only when set (the server omits the default).
                    pay_fee_from_amount: pay_fee_from_amount.then_some(true),
                    stellar_memo: None,
                    xrpl_destination_tag: None,
                },
            );
            if let Some(note) = note {
                builder = builder.note(note);
            }
            submit(builder).await?;
        }
        Command::Send { kind } => run_send(client, kind).await?,
        // Normally dispatched in `main` before a client is built (no creds needed); handled
        // here too so the match is exhaustive without a panic.
        Command::VerifyWebhook { signature } => verify_webhook(&signature)?,
    }
    Ok(())
}

/// Build the per-network transaction and submit it. One example per network — the curated
/// per-kind methods (`evm`, `tron_trigger_contract`, `solana_raw`, …) map onto the matching
/// `transactions:initiate` detail.
#[expect(
    clippy::too_many_lines,
    reason = "flat per-network dispatch; one arm per network reads clearer than indirection"
)]
async fn run_send(client: &UtilaClient, kind: SendKind) -> Result<()> {
    let builder = match kind {
        SendKind::Evm {
            vault,
            from,
            network,
            to,
            value,
            data,
        } => client.transactions().evm(
            VaultId::new(vault),
            EvmTransaction {
                from_address: from,
                network: network.into(),
                to,
                value,
                data,
                publish: None,
            },
        ),
        SendKind::Tron {
            vault,
            network,
            owner,
            contract,
            data,
            call_value,
        } => client.transactions().tron_trigger_contract(
            VaultId::new(vault),
            TronTriggerSmartContract {
                network: network.into(),
                owner_address: owner,
                contract_address: contract,
                data,
                call_value,
            },
        ),
        SendKind::Solana {
            vault,
            network,
            raw_tx,
        } => client.transactions().solana_raw(
            VaultId::new(vault),
            SolanaRaw {
                network: network.into(),
                raw_transaction: raw_tx,
                publish: None,
                replace_blockhash: None,
                try_replace_blockhash: None,
            },
        ),
        SendKind::Stellar {
            vault,
            network,
            source,
            xdr,
        } => client.transactions().stellar_raw(
            VaultId::new(vault),
            StellarRaw {
                network: network.into(),
                source_address: source,
                xdr_envelope: xdr,
                publish: None,
                use_latest_sequence_number: None,
            },
        ),
        SendKind::Sui {
            vault,
            network,
            sender,
            bcs,
        } => client.transactions().sui_raw(
            VaultId::new(vault),
            SuiRaw {
                network: network.into(),
                sender,
                tx_bcs_bytes: bcs,
                publish: None,
            },
        ),
        SendKind::Xrpl {
            vault,
            network,
            sender,
            json,
        } => {
            let json_transaction_data =
                serde_json::from_str(&json).context("parsing --json as a JSON object")?;
            client.transactions().xrpl_raw(
                VaultId::new(vault),
                XrplRaw {
                    network: network.into(),
                    sender,
                    json_transaction_data,
                    publish: None,
                },
            )
        }
    };
    submit(builder).await
}

/// Submit a prepared initiate builder and print the idempotency key + resulting transaction.
/// The result is a single rich object, so it always prints as pretty JSON.
async fn submit(builder: InitiateBuilder<'_>) -> Result<()> {
    let out = builder.send().await?;
    emit_one(&json!({
        "request_id": out.request_id,
        "transaction": out.transaction.map(transaction_json),
    }))
}

fn verify_webhook(signature: &str) -> Result<()> {
    let mut body = Vec::new();
    std::io::stdin()
        .read_to_end(&mut body)
        .context("reading the webhook body from stdin")?;
    let verified = utilars::webhook::verify(&body, signature).context("verifying the signature")?;
    let event = Event::parse(verified).context("parsing the event")?;
    emit_one(&json!({ "verified": true, "event": event_json(&event) }))
}

/// Print a left-aligned text table (kubectl-style): an uppercase header row followed by body
/// rows, each column padded to its widest cell. No separator lines; columns split by two spaces.
fn print_table(headers: &[&str], rows: &[Vec<String>]) {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if let Some(w) = widths.get_mut(i) {
                *w = (*w).max(cell.chars().count());
            }
        }
    }
    let render = |cells: &[String]| {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{c:<width$}", width = widths.get(i).copied().unwrap_or(0)))
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_string()
    };
    let header_cells: Vec<String> = headers.iter().map(|h| (*h).to_string()).collect();
    println!("{}", render(&header_cells));
    for row in rows {
        println!("{}", render(row));
    }
}

/// Emit a list of curated rows — always a list shape, even for a single element: a pretty JSON
/// array under `--json`, else a table whose columns are the `(header, json-key)` pairs, pulling
/// each cell out of the row object by key. (Single-object JSON is only for commands that inherently
/// return one thing — see [`emit_one`].)
fn emit_list(json: bool, columns: &[(&str, &str)], items: &[Value]) -> Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(items).context("serializing JSON")?
        );
    } else {
        let headers: Vec<&str> = columns.iter().map(|(h, _)| *h).collect();
        let rows: Vec<Vec<String>> = items
            .iter()
            .map(|it| columns.iter().map(|(_, k)| cell(it.get(*k))).collect())
            .collect();
        print_table(&headers, &rows);
    }
    Ok(())
}

/// Emit a single curated object as pretty JSON. Used for `get`-one and rich results, which don't
/// fit columns — they print as JSON in both modes (the `--json` flag only toggles list rendering).
fn emit_one(value: &Value) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(value).context("serializing JSON")?
    );
    Ok(())
}

/// The human-mode trailing count line (`— N foo(s)`), suppressed under `--json` to keep the
/// output a single parseable document.
fn footer(json: bool, count: usize, noun: &str) {
    if !json {
        println!("— {count} {noun}(s)");
    }
}

/// Format a JSON value as a table cell: scalars verbatim, arrays comma-joined, null/absent blank.
fn cell(value: Option<&Value>) -> String {
    match value {
        None => String::new(),
        Some(Value::Array(items)) => items.iter().map(scalar_str).collect::<Vec<_>>().join(", "),
        Some(other) => scalar_str(other),
    }
}

/// One JSON scalar as a display string (nested arrays/objects fall back to compact JSON).
fn scalar_str(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

/// The bare id from a resource name, falling back to the full name when it isn't the canonical
/// shape. (Local helper so each builder stays a flat `json!`.)
fn vault_json(v: Vault) -> Value {
    let Vault {
        name,
        display_name,
        archived,
        create_time,
    } = v;
    let id = name
        .as_ref()
        .map_or_else(|| name.to_string(), |r| r.vault.to_string());
    json!({
        "id": id,
        "display_name": display_name,
        "archived": archived,
        "created": create_time.map(|t| t.to_rfc3339()),
    })
}

fn network_json(n: Network) -> Value {
    let id = n
        .name
        .as_ref()
        .map_or_else(|| n.name.to_string(), |r| r.network.to_string());
    json!({
        "id": id,
        "display_name": n.display_name,
        "native_asset": n.native_asset.map(|a| a.as_str().to_string()),
        "status": n.status.map(|s| format!("{s:?}")),
        "testnet": n.testnet,
        "custom": n.custom,
    })
}

fn wallet_json(w: Wallet) -> Value {
    let Wallet {
        name,
        display_name,
        networks,
        archived,
        external,
        has_frozen_assets: _,
    } = w;
    let id = name
        .as_ref()
        .map_or_else(|| name.to_string(), |r| r.wallet.to_string());
    let networks: Vec<String> = networks.iter().map(|n| n.as_str().to_string()).collect();
    json!({
        "id": id,
        "display_name": display_name,
        "networks": networks,
        "archived": archived,
        "external": external,
    })
}

fn wallet_address_json(a: WalletAddress) -> Value {
    let WalletAddress {
        name,
        address,
        display_name,
        network,
        format,
        kind,
        key,
        note,
    } = a;
    let id = name
        .as_ref()
        .map_or_else(|| name.to_string(), |r| r.address.to_string());
    json!({
        "id": id,
        "address": address,
        "network": network.as_str(),
        "display_name": display_name,
        "format": format,
        "type": kind,
        "key": key,
        "note": note,
    })
}

fn asset_json(a: ResolvedAsset) -> Value {
    let ResolvedAsset {
        id,
        decimals,
        symbol,
    } = a;
    json!({
        "id": id.as_str(),
        "symbol": symbol,
        "decimals": decimals,
    })
}

fn balance_json(b: Balance) -> Value {
    let Balance {
        asset,
        amount,
        frozen,
    } = b;
    let decimals = asset.decimals();
    json!({
        "asset": asset.id().as_str(),
        "symbol": asset.symbol(),
        "amount": amount_str(amount, decimals),
        "frozen": amount_str(frozen, decimals),
    })
}

/// Render a base-unit [`Amount`] as a human decimal using the asset's `decimals`. Falls back to
/// the exact base-unit integer when decimals are unknown (asset unresolved) or the value overflows
/// `Decimal`. Trailing zeros are trimmed (`225.000000` → `225`).
fn amount_str(amount: Amount, decimals: Option<u32>) -> String {
    match decimals {
        Some(d) => amount
            .to_decimal(d)
            .map_or_else(|_| amount.to_string(), |dec| dec.normalize().to_string()),
        None => amount.to_string(),
    }
}

fn transaction_json(t: Transaction) -> Value {
    let id = t
        .name
        .as_ref()
        .map_or_else(|| t.name.to_string(), |r| r.transaction.to_string());
    json!({
        "id": id,
        "network": t.network.map(|n| n.as_str().to_string()),
        "state": t.state.map(|s| format!("{s:?}")),
        "kind": t.kind.map(|k| format!("{k:?}")),
        "hash": t.hash,
        "created": t.create_time.map(|x| x.to_rfc3339()),
    })
}

/// Curate a webhook event into a flat object (`type` + envelope id + the referenced resource).
fn event_json(e: &Event) -> Value {
    match e {
        Event::TransactionCreated { id, transaction } => {
            json!({ "type": "TRANSACTION_CREATED", "id": id, "transaction": transaction.to_string() })
        }
        Event::TransactionStateUpdated {
            id,
            transaction,
            new_state,
        } => json!({
            "type": "TRANSACTION_STATE_UPDATED",
            "id": id,
            "transaction": transaction.to_string(),
            "new_state": format!("{new_state:?}"),
        }),
        Event::TransactionAmlScreeningResultReady {
            id,
            transaction,
            action,
        } => json!({
            "type": "TRANSACTION_AML_SCREENING_RESULT_READY",
            "id": id,
            "transaction": transaction.to_string(),
            "action": format!("{action:?}"),
        }),
        Event::WalletCreated { id, wallet } => {
            json!({ "type": "WALLET_CREATED", "id": id, "wallet": wallet.to_string() })
        }
        Event::WalletAddressCreated { id, address } => {
            json!({ "type": "WALLET_ADDRESS_CREATED", "id": id, "address": address.to_string() })
        }
        Event::Test { id, vault } => json!({ "type": "TEST", "id": id, "vault": vault.as_str() }),
        Event::Unknown { id, vault } => {
            json!({ "type": "UNKNOWN", "id": id, "vault": vault.as_str() })
        }
        // `Event` is `#[non_exhaustive]`: a future variant this CLI predates.
        _ => json!({ "type": "OTHER" }),
    }
}
