//! mlt-adapters — concrete implementations of `mlt-core` ports.
//!
//! This is where IO is allowed. Each adapter is the *one* place a given side effect
//! (clock, http, keychain, …) touches the outside world, behind a core port.
//! See `docs/adr/0006-hexagonal-core.md`.

pub mod claude;
pub mod clock;
pub mod consent;
pub mod http;
pub mod secrets;
pub mod sources;

pub use claude::{claude_strategy, detect_user_agent, ClaudeCredentials};
pub use clock::SystemClock;
pub use consent::FileConsentStore;
pub use http::ReqwestHttp;
pub use secrets::KeyringSecretStore;
pub use sources::LocalSourceProbe;

/// Keychain service name under which MLT stores its own secrets.
pub const KEYCHAIN_SERVICE: &str = "com.bigshotpictures.mlt";
