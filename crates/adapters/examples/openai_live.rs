//! Live end-to-end check: read OpenAI usage with a real key and print the snapshot. NOT a CI test
//! (hits the network). Provide the key via the OPENAI_API_KEY env var so the check needs no
//! keychain entry:
//!
//!   OPENAI_API_KEY=sk-… cargo run -p mlt-adapters --example openai_live
//!
//! A normal key (not an `sk-admin…` org key) is expected to hit the honest org-usage limitation —
//! the snapshot still prints, carrying the limitation note instead of a fabricated percentage.
use mlt_adapters::{ReqwestHttp, SystemClock};
use mlt_core::domain::ProviderId;
use mlt_core::ports::{PortError, SecretStore};
use mlt_core::providers::openai::OpenAiStrategy;
use mlt_core::providers::{FetchContext, FetchStrategy};
use std::sync::Arc;

/// Serves the key from OPENAI_API_KEY, standing in for the OS keychain for this hand-run check
/// (the real app reads the user-entered key the same way, via the `SecretStore` port).
struct EnvKey(String);
impl SecretStore for EnvKey {
    fn get(&self, _key: &str) -> Result<Option<String>, PortError> {
        Ok(Some(self.0.clone()))
    }
    fn set(&self, _key: &str, _value: &str) -> Result<(), PortError> {
        Ok(())
    }
    fn delete(&self, _key: &str) -> Result<(), PortError> {
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let Ok(key) = std::env::var("OPENAI_API_KEY") else {
        eprintln!("set OPENAI_API_KEY to your sk-… key");
        std::process::exit(1);
    };
    let strategy = OpenAiStrategy {
        secrets: Arc::new(EnvKey(key)),
        http: Arc::new(ReqwestHttp::new()),
        clock: Arc::new(SystemClock),
    };
    let ctx = FetchContext {
        provider: ProviderId::new("openai"),
    };
    match strategy.fetch(&ctx).await {
        Ok(snapshot) => match serde_json::to_string_pretty(&snapshot) {
            Ok(json) => println!("{json}"),
            Err(e) => {
                eprintln!("failed to serialize snapshot: {e}");
                std::process::exit(1);
            }
        },
        Err(e) => {
            eprintln!("usage fetch failed: {e}");
            std::process::exit(1);
        }
    }
}
