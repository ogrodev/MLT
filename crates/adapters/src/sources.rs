//! [`SourceProbe`] implementation: metadata-only discovery of local sources (ADR 0012).
//!
//! Dispatches a source id to its presence check. Every branch decides presence from existence
//! alone (a credentials file, a Keychain item) and never reads a secret — the per-source
//! checks live with their credential adapter (e.g. [`crate::claude::ClaudeCredentials::is_present`]).
use async_trait::async_trait;

use mlt_core::domain::ProviderId;
use mlt_core::ports::SourceProbe;

use crate::claude::ClaudeCredentials;

/// Probes the real machine for each known source. Unknown ids report absent rather than
/// erroring, so the catalog can list a source before its probe exists.
#[derive(Debug, Default, Clone, Copy)]
pub struct LocalSourceProbe;

#[async_trait]
impl SourceProbe for LocalSourceProbe {
    async fn is_present(&self, id: &ProviderId) -> bool {
        match id.as_str() {
            "claude-code" => ClaudeCredentials::is_present(),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unknown_source_is_absent() {
        assert!(!LocalSourceProbe.is_present(&ProviderId::new("nope")).await);
    }
}
