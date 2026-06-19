//! Shared fixtures for the integration tests (each `tests/<group>.rs` mirrors `src/<group>.rs`).
//!
//! Compiled into every `tests/*.rs` crate; not every crate uses every helper, so the
//! per-crate dead-code lint is silenced here. `#[expect]` can't be used — it would be
//! unfulfilled in the crates that *do* use the item.
#![allow(
    dead_code,
    reason = "shared fixtures compiled into every test crate; not all are used by each"
)]

use utilars::{SignerSource, UtilaClient};

pub const ACCOUNT: &str = "my-sa@vault-a1b2c3d4.utilaserviceaccount.io";
pub const TEST_KEY: &[u8] = include_bytes!("../test_key.pem");

/// A client pointed at a mock server, authenticated with the test service-account key.
pub fn client(base_url: &str) -> UtilaClient {
    UtilaClient::builder()
        .credential(
            ACCOUNT,
            SignerSource::local_pem(TEST_KEY).expect("valid key"),
        )
        .base_url(base_url)
        .build()
        .expect("client builds")
}
