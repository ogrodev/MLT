//! mlt-adapters — concrete implementations of `mlt-core` ports.
//!
//! This is where IO is allowed. Each adapter is the *one* place a given side effect
//! (clock, http, keychain, …) touches the outside world, behind a core port.
//! See `docs/adr/0006-hexagonal-core.md`.

pub mod accounts;
pub mod anthropic;
pub mod claude;
pub mod clock;
pub mod codex;
pub mod consent;
pub mod http;
pub mod identity;
pub mod labels;
pub mod openai;
pub mod openrouter;
pub(crate) mod resilience;
pub mod secrets;
pub mod sources;

pub use accounts::discovered_accounts;
pub use anthropic::anthropic_strategy;
pub use claude::{claude_account_strategy, claude_strategy, ClaudeCredentials};
pub use clock::SystemClock;
pub use codex::codex_strategy;
pub use consent::FileConsentStore;
pub use http::ReqwestHttp;
pub use identity::FileIdentityStore;
pub use labels::FileLabelStore;
pub use openai::openai_strategy;
pub use openrouter::openrouter_strategy;
pub use secrets::KeyringSecretStore;
pub use sources::LocalSourceProbe;

/// Keychain service name under which MLT stores its own secrets.
pub const KEYCHAIN_SERVICE: &str = "com.bigshotpictures.mlt";

#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: parking_lot::Mutex<()> = parking_lot::const_mutex(());
