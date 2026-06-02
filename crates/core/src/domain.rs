//! Provider-agnostic domain types. No behavior that needs IO.
use serde::{Deserialize, Serialize};

/// Unix epoch milliseconds. Time only enters core via the [`crate::ports::Clock`] port —
/// core code never calls `SystemTime::now()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Timestamp(pub i64);

/// Stable provider slug, e.g. `"claude-code"`, `"openrouter"`.
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
    /// Percent of the window consumed, `0.0..=100.0`.
    pub used_percent: f64,
    pub window_minutes: Option<i64>,
    pub resets_at: Option<Timestamp>,
    /// Human label for `Custom`/model-specific windows (e.g. "Sonnet · 7-day").
    pub reset_description: Option<String>,
}

impl UsageWindow {
    /// Remaining headroom in the window, clamped to `[0, 100]`. Pure logic — the kind of
    /// thing unit-tested without any IO.
    pub fn remaining_percent(&self) -> f64 {
        (100.0 - self.used_percent).clamp(0.0, 100.0)
    }
}

/// A provider account's identity, fetched **from the provider** (never user-entered) so the
/// user can tell *which* account a panel reports. Display-only — it plays no part in auth or
/// consent — and siloed per provider (never rendered under another; see AGENTS.md). Lossy
/// (ADR 0015): any field the provider omits stays `None`, and an all-`None` identity is simply
/// not shown.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AccountIdentity {
    /// The account's email address (e.g. Anthropic's OAuth profile). The primary identifier.
    pub email: Option<String>,
    /// The account's organization/team name when the provider exposes one — shown as a
    /// fallback identifier when there is no email.
    pub organization: Option<String>,
}

impl AccountIdentity {
    /// Nothing worth showing — neither an email nor an organization was resolved.
    pub fn is_empty(&self) -> bool {
        self.email.is_none() && self.organization.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageSnapshot {
    pub provider: ProviderId,
    pub windows: Vec<UsageWindow>,
    pub status: Status,
    pub fetched_at: Timestamp,
    /// Which account this snapshot reports, for display (email/org), or `None` when unknown.
    /// Provider-fetched, never user-entered; siloed per provider.
    pub account: Option<AccountIdentity>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalendarEvent {
    pub title: String,
    pub start: Timestamp,
    pub end: Timestamp,
}

/// OAuth tokens read from a vendor CLI's credential store (e.g. Claude Code, Codex).
/// We reuse the existing login rather than running our own (see docs/research/PROVIDERS.md).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OAuthTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<Timestamp>,
    pub scopes: Vec<String>,
    pub subscription_type: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let over = UsageWindow {
            used_percent: 140.0,
            ..w
        };
        assert_eq!(over.remaining_percent(), 0.0);
    }
}
