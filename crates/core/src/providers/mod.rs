//! The provider contract: a descriptor + an ordered chain of typed fetch strategies
//! tried until one succeeds (ADR 0005, refined from CodexBar's pipeline).
use crate::domain::*;
use crate::ports::*;
use async_trait::async_trait;
use std::sync::Arc;

pub mod claude;
pub mod codex;
pub mod oauth;
pub mod openrouter;

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

/// Everything a strategy needs to attempt a fetch. Stub — extend with shared request
/// context as more strategies are implemented.
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
