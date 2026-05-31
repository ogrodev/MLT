//! The IO contracts. Adapters implement these; core only ever sees the traits.
use crate::domain::*;
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

/// Secrets we own live in the OS keychain only — never in the DB or logs (ADR 0012).
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>, PortError>;
    fn set(&self, key: &str, value: &str) -> Result<(), PortError>;
    fn delete(&self, key: &str) -> Result<(), PortError>;
}

/// Reads OAuth tokens from a vendor CLI's credential store (file and/or OS keychain).
/// `NotFound` means the provider isn't logged in on this machine.
#[async_trait]
pub trait OAuthCredentialSource: Send + Sync {
    async fn load(&self) -> Result<OAuthTokens, PortError>;
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let clock = FakeClock {
            fixed: 1_700_000_000_000,
        };
        assert_eq!(clock.now(), Timestamp(1_700_000_000_000));
    }
}
