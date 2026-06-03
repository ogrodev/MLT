//! OpenRouter adapter: assemble the API-key usage strategy from our keychain + HTTP client.
//!
//! OpenRouter is an API-key provider (ADR 0014/0016), so there is no login to discover and no
//! OAuth to refresh — the strategy reads the user-entered key (stored by task 003 under our own
//! keychain service) via the `SecretStore` port and polls OpenRouter. All IO lives here; the
//! parsing and window mapping are pure in [`mlt_core::providers::openrouter`].
use std::sync::Arc;

use mlt_core::providers::openrouter::OpenRouterStrategy;

use crate::{KeyringSecretStore, ReqwestHttp, SystemClock, KEYCHAIN_SERVICE};

/// Build a ready-to-run OpenRouter usage strategy: read the stored API key from our keychain and
/// poll the key + credits endpoints. The key is only ever read — never written back.
pub fn openrouter_strategy() -> OpenRouterStrategy {
    OpenRouterStrategy {
        secrets: Arc::new(KeyringSecretStore::new(KEYCHAIN_SERVICE)),
        http: Arc::new(ReqwestHttp::new()),
        clock: Arc::new(SystemClock),
    }
}
