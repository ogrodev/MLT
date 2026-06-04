//! Live end-to-end check: read OpenRouter usage with a real key and print the snapshot. NOT a
//! CI test (hits the network). Provide the key via the OPENROUTER_API_KEY env var so the check
//! needs no keychain entry:
//!
//!   OPENROUTER_API_KEY=sk-or-v1-… cargo run -p mlt-adapters --example openrouter_live
use mlt_adapters::{ReqwestHttp, SystemClock};
use mlt_core::domain::ProviderId;
use mlt_core::ports::{PortError, SecretStore};
use mlt_core::providers::openrouter::OpenRouterStrategy;
use mlt_core::providers::{FetchContext, FetchStrategy};
use std::sync::Arc;

/// Serves the key from OPENROUTER_API_KEY, standing in for the OS keychain for this hand-run
/// check (the real app reads the user-entered key the same way, via the `SecretStore` port).
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
    let Ok(key) = std::env::var("OPENROUTER_API_KEY") else {
        eprintln!("set OPENROUTER_API_KEY to your sk-or-v1-… key");
        std::process::exit(1);
    };
    let strategy = OpenRouterStrategy {
        secrets: Arc::new(EnvKey(key)),
        http: Arc::new(ReqwestHttp::new()),
        clock: Arc::new(SystemClock),
    };
    let ctx = FetchContext {
        provider: ProviderId::new("openrouter"),
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
