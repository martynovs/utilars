//! A small demo CLI exercising the `utilars` client against the real v2 API.
//!
//! This is also a worked example of the intended credential pattern: the **library never
//! reads the environment** — the *application* (this CLI) reads its own secrets/config and
//! passes them to the builder.
//!
//! ## Run it
//!
//! ```text
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
use std::time::Duration;

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use futures::TryStreamExt as _;
use rust_decimal::Decimal;
use utilars::{
    AssetId, AssetRef, AssetTransfer, EvmTransaction, InitiateBuilder, ParseRef, SignerSource,
    SolanaRaw, StellarRaw, SuiRaw, TransactionId, TronTriggerSmartContract, UtilaClient, VaultId,
    XrplRaw,
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
    run(&client, cli.command).await
}

/// Build a client from environment configuration — the application's job, not the library's.
fn build_client() -> Result<UtilaClient> {
    let account =
        std::env::var("UTILA_ACCOUNT").context("set UTILA_ACCOUNT to the service-account email")?;
    let signer = signer_from_env()?;

    let mut builder = UtilaClient::builder()
        .credential(account, signer)
        .timeout(Duration::from_secs(30));
    if let Ok(base) = std::env::var("UTILA_BASE_URL") {
        builder = builder.base_url(base);
    }
    builder.build().context("building the client")
}

/// Resolve a signer from the environment, in priority order:
/// `UTILA_KMS_KEY_URL` → `UTILA_PRIVATE_KEY_PEM` (inline) → `UTILA_PRIVATE_KEY_PATH` (file).
/// Demonstrates that a local PEM can come straight from an env var, not just a file.
fn signer_from_env() -> Result<SignerSource> {
    if let Ok(url) = std::env::var("UTILA_KMS_KEY_URL") {
        return kms_signer(&url);
    }
    if let Ok(value) = std::env::var("UTILA_PRIVATE_KEY_PEM") {
        let pem = decode_pem_var(&value)?;
        return SignerSource::local_pem(&pem).context("parsing UTILA_PRIVATE_KEY_PEM");
    }
    let path = std::env::var("UTILA_PRIVATE_KEY_PATH")
        .context("set UTILA_PRIVATE_KEY_PATH, UTILA_PRIVATE_KEY_PEM, or UTILA_KMS_KEY_URL")?;
    let pem = std::fs::read(&path).with_context(|| format!("reading key file {path}"))?;
    SignerSource::local_pem(&pem).context("parsing the RSA private key")
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
async fn run(client: &UtilaClient, command: Command) -> Result<()> {
    match command {
        Command::Networks => {
            let networks: Vec<_> = client.networks().stream().try_collect().await?;
            for n in &networks {
                println!("{n:#?}");
            }
            println!("— {} network(s)", networks.len());
        }
        Command::Assets { ids } => {
            let assets = client
                .assets()
                .batch_get(ids.into_iter().map(parse_asset))
                .await?;
            for a in &assets {
                println!("{a:#?}");
            }
            println!("— {} asset(s)", assets.len());
        }
        Command::Vaults => {
            let vaults: Vec<_> = client.vaults().stream().try_collect().await?;
            for v in &vaults {
                println!("{v:#?}");
            }
            println!("— {} vault(s)", vaults.len());
        }
        Command::Vault { id } => {
            println!("{:#?}", client.vaults().get(VaultId::new(id)).await?);
        }
        Command::Wallets { vault } => {
            let wallets: Vec<_> = client
                .wallets()
                .list(VaultId::new(vault))
                .stream()
                .try_collect()
                .await?;
            for w in &wallets {
                println!("{w:#?}");
            }
            println!("— {} wallet(s)", wallets.len());
        }
        Command::Balances { vault } => {
            let balances = client.balances().query(VaultId::new(vault)).await?;
            for b in &balances {
                println!(
                    "{:>24}  {:<8}  {}",
                    b.amount,
                    b.asset.symbol().unwrap_or("?"),
                    b.asset.id()
                );
            }
            println!("— {} balance(s)", balances.len());
        }
        Command::Transactions { vault, limit } => {
            let page = client
                .transactions()
                .list(VaultId::new(vault))
                .page_size(limit)
                .send()
                .await?;
            for t in &page.transactions {
                println!("{t:#?}");
            }
            println!(
                "— {} of {} transaction(s)",
                page.transactions.len(),
                page.total_size
            );
        }
        Command::Transaction { vault, id } => {
            let tx = client
                .transactions()
                .get(VaultId::new(vault), TransactionId::new(id))
                .await?;
            println!("{tx:#?}");
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
async fn submit(builder: InitiateBuilder<'_>) -> Result<()> {
    let out = builder.send().await?;
    println!("initiated — request_id = {}", out.request_id);
    if let Some(tx) = out.transaction {
        println!("{tx:#?}");
    }
    Ok(())
}

fn verify_webhook(signature: &str) -> Result<()> {
    let mut body = Vec::new();
    std::io::stdin()
        .read_to_end(&mut body)
        .context("reading the webhook body from stdin")?;
    let verified = utilars::webhook::verify(&body, signature).context("verifying the signature")?;
    let event = utilars::webhook::Event::parse(verified).context("parsing the event")?;
    println!("signature OK");
    println!("{event:#?}");
    Ok(())
}
