//! Live end-to-end check: discover every per-account login on this machine (Codex + Claude
//! Code, from the Oh My Pi store and vendor CLIs, deduped) and print each account's usage
//! snapshot through its provider's real strategy. NOT a CI test (hits the network).
//!
//!   cargo run -p mlt-adapters --example accounts_live
use mlt_adapters::{
    claude_account_strategy, codex_strategy, discovered_accounts, FileIdentityStore,
};
use mlt_core::domain::ProviderId;
use mlt_core::providers::{FetchContext, FetchStrategy};
use mlt_core::sources::account_source_id;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let accounts = discovered_accounts();
    if accounts.is_empty() {
        eprintln!("no Codex/Claude logins found (checked Oh My Pi profiles and the vendor CLIs)");
        std::process::exit(1);
    }
    println!("discovered {} account(s):", accounts.len());
    for a in &accounts {
        let who = a.email.as_deref().unwrap_or(&a.account_id);
        println!("  - [{}] {who}  [{}]", a.base.as_str(), a.origin);
    }

    let identity = Arc::new(FileIdentityStore::load(
        std::env::temp_dir().join("mlt-accounts-live-identity.json"),
    ));
    for a in &accounts {
        let ctx = FetchContext {
            provider: ProviderId::new(account_source_id(a.base.as_str(), &a.account_id)),
        };
        let who = a.email.as_deref().unwrap_or(&a.account_id);
        // Route to the provider's strategy the same way the app's fetch_for does.
        let result = match a.base.as_str() {
            "codex" => {
                codex_strategy(&a.account_id, identity.clone())
                    .fetch(&ctx)
                    .await
            }
            "claude-code" => {
                claude_account_strategy(&a.account_id, identity.clone())
                    .fetch(&ctx)
                    .await
            }
            other => {
                eprintln!("# {who}: no strategy wired for base {other}");
                continue;
            }
        };
        match result {
            Ok(snapshot) => match serde_json::to_string_pretty(&snapshot) {
                Ok(json) => println!("\n# [{}] {who}\n{json}", a.base.as_str()),
                Err(e) => eprintln!(
                    "\n# [{}] {who}: snapshot serialization failed: {e}",
                    a.base.as_str()
                ),
            },
            Err(e) => eprintln!("\n# [{}] {who}: usage fetch failed: {e}", a.base.as_str()),
        }
    }
}
