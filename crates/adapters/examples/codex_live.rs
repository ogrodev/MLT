//! Live end-to-end check: reads your real Codex token (`~/.codex/auth.json`) and prints the
//! parsed usage snapshot. NOT a CI test (hits the network).
//!
//!   cargo run -p mlt-adapters --example codex_live
//!
//! Requires a logged-in Codex CLI on this machine (run `codex` once to sign in).
use mlt_adapters::{codex_strategy, FileIdentityStore};
use mlt_core::domain::ProviderId;
use mlt_core::providers::{FetchContext, FetchStrategy};
use std::sync::Arc;

#[tokio::main]
async fn main() {
    // Cache identity to a throwaway temp file so re-runs print the email without re-decoding.
    let identity = Arc::new(FileIdentityStore::load(
        std::env::temp_dir().join("mlt-codex-live-identity.json"),
    ));
    let strategy = codex_strategy(identity);
    let ctx = FetchContext {
        provider: ProviderId::new("codex"),
    };

    match strategy.fetch(&ctx).await {
        Ok(snapshot) => {
            println!("{}", serde_json::to_string_pretty(&snapshot).unwrap());
        }
        Err(e) => {
            eprintln!("codex usage fetch failed: {e}");
            std::process::exit(1);
        }
    }
}
