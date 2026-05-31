//! mlt-adapters — concrete implementations of `mlt-core` ports.
//!
//! This is where IO is allowed. Each adapter is the *one* place a given side effect
//! (clock, http, keychain, …) touches the outside world, behind a core port.
//! See `docs/adr/0006-hexagonal-core.md`.

pub mod claude;
pub mod clock;
pub mod http;

pub use claude::{claude_strategy, detect_user_agent, ClaudeCredentials};
pub use clock::SystemClock;
pub use http::ReqwestHttp;
