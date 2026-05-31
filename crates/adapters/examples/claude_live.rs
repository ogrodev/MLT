//! Live end-to-end check: reads your real Claude Code token (file/Keychain) and prints the
//! parsed usage snapshot. NOT a CI test (hits the network + Keychain).
//!
//!   cargo run -p mlt-adapters --example claude_live
//!
//! A macOS Keychain prompt may appear the first time — click "Always Allow".
use mlt_adapters::claude_strategy;
use mlt_core::domain::ProviderId;
use mlt_core::providers::{FetchContext, FetchStrategy};

#[tokio::main]
async fn main() {
    let strategy = claude_strategy();
    let ctx = FetchContext { provider: ProviderId::new("claude-code") };

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
