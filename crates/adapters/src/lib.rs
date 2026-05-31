//! mlt-adapters — concrete implementations of `mlt-core` ports.
//!
//! This is where IO is allowed. Each adapter is the *one* place a given side effect
//! (clock, http, sqlite, keychain, …) touches the outside world, behind a core port.
//! See `docs/adr/0006-hexagonal-core.md`.

use std::time::{SystemTime, UNIX_EPOCH};

use mlt_core::domain::Timestamp;
use mlt_core::ports::Clock;

/// The real system clock — the single place core's `Clock` port reads the OS time.
/// (Core itself is forbidden from calling `SystemTime::now()`.)
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        Timestamp(ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_returns_a_positive_instant() {
        // We don't assert an exact value (that's the point of the port) — just that the
        // adapter produces a plausible, post-epoch timestamp.
        assert!(SystemClock.now().0 > 0);
    }
}
