//! Live end-to-end check: discover every Codex login on this machine (Codex CLI +
//! Oh My Pi profiles, deduped) and print each account's usage snapshot. NOT a CI test (hits the
//! network).
//!
//!   cargo run -p mlt-adapters --example codex_live
use mlt_adapters::{codex_accounts, codex_strategy, FileIdentityStore};
use mlt_core::domain::ProviderId;
use mlt_core::providers::{FetchContext, FetchStrategy};
use mlt_core::sources::codex_source_id;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let accounts = codex_accounts();
    if accounts.is_empty() {
        eprintln!("no Codex logins found (checked the Codex CLI and Oh My Pi profiles)");
        std::process::exit(1);
    }
    println!("discovered {} Codex account(s):", accounts.len());
    for account in &accounts {
        let who = account.email.as_deref().unwrap_or(&account.account_id);
        println!("  - {who}  [{}]", account.origin);
    }

    let identity = Arc::new(FileIdentityStore::load(
        std::env::temp_dir().join("mlt-codex-live-identity.json"),
    ));
    for account in &accounts {
        let strategy = codex_strategy(&account.account_id, identity.clone());
        let ctx = FetchContext {
            provider: ProviderId::new(codex_source_id(&account.account_id)),
        };
        let who = account.email.as_deref().unwrap_or(&account.account_id);
        match strategy.fetch(&ctx).await {
            Ok(snapshot) => println!(
                "\n# {who}\n{}",
                serde_json::to_string_pretty(&snapshot).unwrap()
            ),
            Err(e) => eprintln!("\n# {who}: usage fetch failed: {e}"),
        }
    }
}
