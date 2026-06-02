//! Live end-to-end check: reads your real Claude Code token (file/Keychain) and prints the
//! parsed usage snapshot. NOT a CI test (hits the network + Keychain).
//!
//!   cargo run -p mlt-adapters --example claude_live
//!
//! A macOS Keychain prompt may appear the first time — click "Always Allow".
use mlt_adapters::{claude_strategy, FileIdentityStore};
use mlt_core::domain::ProviderId;
use mlt_core::providers::{FetchContext, FetchStrategy};
use std::sync::Arc;

#[tokio::main]
async fn main() {
    // Cache identity to a throwaway temp file so re-runs print the email without re-fetching.
    let identity = Arc::new(FileIdentityStore::load(
        std::env::temp_dir().join("mlt-claude-live-identity.json"),
    ));
    let strategy = claude_strategy(identity);
    let ctx = FetchContext {
        provider: ProviderId::new("claude-code"),
    };

    match strategy.fetch(&ctx).await {
        Ok(snapshot) => {
            println!("{}", serde_json::to_string_pretty(&snapshot).unwrap());
        }
        Err(e) => {
            eprintln!("claude usage fetch failed: {e}");
            std::process::exit(1);
        }
    }
}
