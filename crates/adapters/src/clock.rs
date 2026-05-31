//! System clock adapter.
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
        assert!(SystemClock.now().0 > 0);
    }
}
