//! Anthropic **API** adapter: assemble the API-key usage strategy from our keychain + HTTP client.
//!
//! This is the Anthropic *API* provider (ADR 0014/0016) — distinct from the `"claude-code"`
//! subscription provider. There is no login to discover and no OAuth to refresh: the user pastes
//! a normal `sk-ant-api…` key (stored by task 003 under our own keychain service), which the
//! strategy reads via the `SecretStore` port and uses to poll Anthropic's org cost endpoint. All
//! IO lives here; the parsing and honest-note mapping are pure in [`mlt_core::providers::anthropic`].
use std::sync::Arc;

use mlt_core::providers::anthropic::AnthropicStrategy;

use crate::{KeyringSecretStore, ReqwestHttp, SystemClock, KEYCHAIN_SERVICE};

/// Build a ready-to-run Anthropic **API** usage strategy: read the stored API key from our
/// keychain and poll the org cost endpoint. The key is only ever read — never written back.
pub fn anthropic_strategy() -> AnthropicStrategy {
    AnthropicStrategy {
        secrets: Arc::new(KeyringSecretStore::new(KEYCHAIN_SERVICE)),
        http: Arc::new(ReqwestHttp::new()),
        clock: Arc::new(SystemClock),
    }
}
