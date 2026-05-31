//! mlt-core — the pure heart of MLT: domain types + ports, with **zero IO**.
//!
//! No network, disk, OS, clock, or randomness lives here. Every side effect enters
//! through a trait in [`ports`]; adapters (crate `mlt-adapters`) implement them, and the
//! Tauri app wires them together. This is what makes the logic testable in milliseconds
//! with fakes (see the `tests` module) and keeps OS-specific code out of the core.
//! See `docs/adr/0006-hexagonal-core.md`.

pub mod domain {
    //! Provider-agnostic domain types. No behavior that needs IO.
    use serde::{Deserialize, Serialize};

    /// Unix epoch milliseconds. Time only enters core via the [`super::ports::Clock`] port —
    /// core code never calls `SystemTime::now()`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
    pub struct Timestamp(pub i64);

    /// Stable provider slug, e.g. `"codex"`, `"openrouter"`.
    #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct ProviderId(pub String);

    impl ProviderId {
        pub fn new(id: impl Into<String>) -> Self {
            Self(id.into())
        }
        pub fn as_str(&self) -> &str {
            &self.0
        }
    }

    /// Which usage window a measurement belongs to (typed, not positional — improving on
    /// CodexBar's primary/secondary/tertiary; see docs/research/PROVIDERS.md).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum WindowKind {
        Session,
        Weekly,
        Monthly,
        Custom,
    }

    /// The unit a window is measured in.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum Unit {
        Tokens,
        Requests,
        Usd,
        Percent,
    }

    /// Freshness of a snapshot. We surface `Stale`/`Error` rather than crashing (ADR 0015).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum Status {
        Ok,
        Stale,
        Error,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct UsageWindow {
        pub kind: WindowKind,
        pub used_percent: f64,
        pub window_minutes: Option<i64>,
        pub resets_at: Option<Timestamp>,
        pub reset_description: Option<String>,
    }

    impl UsageWindow {
        /// Remaining headroom in the window, clamped to `[0, 100]`. Pure logic — the kind
        /// of thing unit-tested without any IO.
        pub fn remaining_percent(&self) -> f64 {
            (100.0 - self.used_percent).clamp(0.0, 100.0)
        }
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct UsageSnapshot {
        pub provider: ProviderId,
        pub windows: Vec<UsageWindow>,
        pub status: Status,
        pub fetched_at: Timestamp,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct CalendarEvent {
        pub title: String,
        pub start: Timestamp,
        pub end: Timestamp,
    }
}

pub mod ports {
    //! The IO contracts. Adapters implement these; core only ever sees the traits.
    use super::domain::*;
    use async_trait::async_trait;

    #[derive(Debug, thiserror::Error)]
    pub enum PortError {
        #[error("io error: {0}")]
        Io(String),
        #[error("not found")]
        NotFound,
    }

    /// The only way core obtains the current instant. Inject a fake in tests.
    pub trait Clock: Send + Sync {
        fn now(&self) -> Timestamp;
    }

    pub struct HttpRequest {
        pub method: String,
        pub url: String,
        pub headers: Vec<(String, String)>,
        pub body: Option<Vec<u8>>,
    }

    pub struct HttpResponse {
        pub status: u16,
        pub body: Vec<u8>,
    }

    #[async_trait]
    pub trait HttpPort: Send + Sync {
        async fn send(&self, req: HttpRequest) -> Result<HttpResponse, PortError>;
    }

    /// Secrets live in the OS keychain only — never in the DB or logs (ADR 0012).
    pub trait SecretStore: Send + Sync {
        fn get(&self, key: &str) -> Result<Option<String>, PortError>;
        fn set(&self, key: &str, value: &str) -> Result<(), PortError>;
        fn delete(&self, key: &str) -> Result<(), PortError>;
    }

    #[async_trait]
    pub trait UsageRepo: Send + Sync {
        async fn save(&self, snapshot: &UsageSnapshot) -> Result<(), PortError>;
        async fn latest(&self, provider: &ProviderId) -> Result<Option<UsageSnapshot>, PortError>;
    }

    #[async_trait]
    pub trait Notifier: Send + Sync {
        async fn notify(&self, title: &str, body: &str) -> Result<(), PortError>;
    }

    #[async_trait]
    pub trait CalendarPort: Send + Sync {
        async fn events(&self, from: Timestamp, to: Timestamp)
            -> Result<Vec<CalendarEvent>, PortError>;
    }
}

pub mod providers {
    //! The provider contract: a descriptor + an ordered chain of typed fetch strategies
    //! tried until one succeeds (ADR 0005, refined from CodexBar's pipeline).
    use super::domain::*;
    use super::ports::*;
    use async_trait::async_trait;
    use std::sync::Arc;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum FetchKind {
        Cli,
        OAuth,
        Cookie,
        ApiToken,
        LocalProbe,
        WebDashboard,
    }

    #[derive(Debug, thiserror::Error)]
    pub enum FetchError {
        #[error("credentials unavailable")]
        Unavailable,
        #[error("rate limited")]
        RateLimited,
        #[error("upstream error: {0}")]
        Upstream(String),
        #[error(transparent)]
        Port(#[from] PortError),
    }

    /// Everything a strategy needs to attempt a fetch. Stub — extend with shared port
    /// handles as real strategies are implemented.
    pub struct FetchContext {
        pub provider: ProviderId,
    }

    /// One credential path (CLI token, OAuth, cookie, API key, …). Providers compose an
    /// ordered chain of these; a pipeline runs them with fallback.
    #[async_trait]
    pub trait FetchStrategy: Send + Sync {
        fn kind(&self) -> FetchKind;
        async fn is_available(&self, ctx: &FetchContext) -> bool;
        async fn fetch(&self, ctx: &FetchContext) -> Result<UsageSnapshot, FetchError>;
        fn should_fallback(&self, err: &FetchError) -> bool;
    }

    /// Static description + the ordered fallback chain for one provider.
    pub struct ProviderDescriptor {
        pub id: ProviderId,
        pub display_name: String,
        pub strategies: Vec<Arc<dyn FetchStrategy>>,
    }
}

pub use domain::*;
pub use ports::*;
pub use providers::*;

#[cfg(test)]
mod tests {
    use super::domain::*;
    use super::ports::Clock;

    /// A fake `Clock` — proof that core logic is testable with no real clock and no IO.
    struct FakeClock {
        fixed: i64,
    }
    impl Clock for FakeClock {
        fn now(&self) -> Timestamp {
            Timestamp(self.fixed)
        }
    }

    #[test]
    fn fake_clock_is_deterministic() {
        let clock = FakeClock { fixed: 1_700_000_000_000 };
        assert_eq!(clock.now(), Timestamp(1_700_000_000_000));
    }

    #[test]
    fn remaining_percent_is_clamped() {
        let w = UsageWindow {
            kind: WindowKind::Session,
            used_percent: 73.0,
            window_minutes: Some(300),
            resets_at: Some(Timestamp(1_700_000_000_000)),
            reset_description: None,
        };
        assert_eq!(w.remaining_percent(), 27.0);

        let over = UsageWindow { used_percent: 140.0, ..w };
        assert_eq!(over.remaining_percent(), 0.0);
    }
}
