//! Live end-to-end check: read Anthropic **API** usage with a real key and print the snapshot.
//! NOT a CI test (hits the network). Provide the key via the ANTHROPIC_API_KEY env var so the
//! check needs no keychain entry:
//!
//!   ANTHROPIC_API_KEY=sk-ant-api-… cargo run -p mlt-adapters --example anthropic_live
//!
//! With a normal (non-admin) key, expect the honest-limitation note rather than a spend figure —
//! that is the point of task 008.
use mlt_adapters::{ReqwestHttp, SystemClock};
use mlt_core::domain::ProviderId;
use mlt_core::ports::{PortError, SecretStore};
use mlt_core::providers::anthropic::AnthropicStrategy;
use mlt_core::providers::{FetchContext, FetchStrategy};
use std::sync::Arc;

/// Serves the key from ANTHROPIC_API_KEY, standing in for the OS keychain for this hand-run
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
    let Ok(key) = std::env::var("ANTHROPIC_API_KEY") else {
        eprintln!("set ANTHROPIC_API_KEY to your sk-ant-api-… key");
        std::process::exit(1);
    };
    let strategy = AnthropicStrategy {
        secrets: Arc::new(EnvKey(key)),
        http: Arc::new(ReqwestHttp::new()),
        clock: Arc::new(SystemClock),
    };
    let ctx = FetchContext {
        provider: ProviderId::new("anthropic"),
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
