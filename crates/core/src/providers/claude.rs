//! Claude Code (Anthropic subscription) provider.
//!
//! Reuses the Claude Code CLI's own OAuth token (read by an adapter from the file or the
//! macOS Keychain) and polls the private `api/oauth/usage` endpoint. The parser is pure and
//! deliberately lossy (ADR 0015): the endpoint returns a map of window → {utilization,
//! resets_at} with many null / experimental keys, so unknown or null windows are skipped,
//! never fatal. See docs/research/PROVIDERS.md.
use crate::domain::*;
use crate::ports::*;
use super::{FetchContext, FetchError, FetchKind, FetchStrategy};
use async_trait::async_trait;
use std::sync::Arc;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

pub const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const OAUTH_BETA: &str = "oauth-2025-04-20";
const REQUIRED_SCOPE: &str = "user:profile";

fn parse_rfc3339_ms(s: &str) -> Option<Timestamp> {
    OffsetDateTime::parse(s, &Rfc3339)
        .ok()
        .map(|dt| Timestamp((dt.unix_timestamp_nanos() / 1_000_000) as i64))
}

fn kind_rank(k: WindowKind) -> u8 {
    match k {
        WindowKind::Session => 0,
        WindowKind::Weekly => 1,
        WindowKind::Monthly => 2,
        WindowKind::Custom => 3,
    }
}

/// Map a window key to (kind, window_minutes, label). Unknown keys still parse as `Custom`
/// windows if they carry a `utilization`, so new server-side windows never break us.
fn classify(key: &str) -> (WindowKind, Option<i64>, Option<String>) {
    match key {
        "five_hour" => (WindowKind::Session, Some(300), None),
        "seven_day" => (WindowKind::Weekly, Some(10_080), None),
        "seven_day_opus" => (WindowKind::Custom, Some(10_080), Some("Opus · 7-day".into())),
        "seven_day_sonnet" => (WindowKind::Custom, Some(10_080), Some("Sonnet · 7-day".into())),
        "seven_day_oauth_apps" => {
            (WindowKind::Custom, Some(10_080), Some("OAuth apps · 7-day".into()))
        }
        "seven_day_cowork" => (WindowKind::Custom, Some(10_080), Some("Cowork · 7-day".into())),
        other => (WindowKind::Custom, None, Some(other.replace('_', " "))),
    }
}

/// Pure parser for the `api/oauth/usage` body. Lossy by design (ADR 0015).
pub fn parse_usage(body: &str) -> Result<Vec<UsageWindow>, FetchError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| FetchError::Upstream(format!("bad json: {e}")))?;
    let obj = value
        .as_object()
        .ok_or_else(|| FetchError::Upstream("expected a JSON object".into()))?;

    let mut windows = Vec::new();
    for (key, val) in obj {
        if key == "extra_usage" {
            if let Some(w) = parse_extra_usage(val) {
                windows.push(w);
            }
            continue;
        }
        // A window is any object carrying a numeric `utilization`. null / other → skip.
        let Some(util) = val
            .as_object()
            .and_then(|o| o.get("utilization"))
            .and_then(|u| u.as_f64())
        else {
            continue;
        };
        let (kind, window_minutes, reset_description) = classify(key);
        let resets_at = val
            .get("resets_at")
            .and_then(|r| r.as_str())
            .and_then(parse_rfc3339_ms);
        windows.push(UsageWindow {
            kind,
            used_percent: util,
            window_minutes,
            resets_at,
            reset_description,
        });
    }

    // Stable order regardless of JSON key order: Session, Weekly, Monthly, then Custom by label.
    windows.sort_by(|a, b| {
        kind_rank(a.kind)
            .cmp(&kind_rank(b.kind))
            .then_with(|| a.reset_description.cmp(&b.reset_description))
    });
    Ok(windows)
}

/// `extra_usage` is credit-shaped, not a normal window. Surface it as a Monthly window only
/// when it carries a numeric utilization; otherwise there's nothing meaningful to show.
fn parse_extra_usage(val: &serde_json::Value) -> Option<UsageWindow> {
    let o = val.as_object()?;
    if o.get("is_enabled").and_then(|b| b.as_bool()) != Some(true) {
        return None;
    }
    let util = o.get("utilization").and_then(|u| u.as_f64())?;
    let currency = o.get("currency").and_then(|c| c.as_str()).unwrap_or("USD");
    Some(UsageWindow {
        kind: WindowKind::Monthly,
        used_percent: util,
        window_minutes: None,
        resets_at: None,
        reset_description: Some(format!("Extra usage ({currency})")),
    })
}

/// The OAuth strategy for Claude Code: read the CLI's token, poll `api/oauth/usage`.
pub struct ClaudeCodeStrategy {
    pub creds: Arc<dyn OAuthCredentialSource>,
    pub http: Arc<dyn HttpPort>,
    pub clock: Arc<dyn Clock>,
    /// e.g. `"claude-code/2.1.158"`. REQUIRED — without the claude-code UA the endpoint
    /// rate-limits hard (persistent 429). See docs/research/PROVIDERS.md.
    pub user_agent: String,
}

#[async_trait]
impl FetchStrategy for ClaudeCodeStrategy {
    fn kind(&self) -> FetchKind {
        FetchKind::OAuth
    }

    async fn is_available(&self, _ctx: &FetchContext) -> bool {
        matches!(self.creds.load().await, Ok(t) if t.scopes.iter().any(|s| s == REQUIRED_SCOPE))
    }

    async fn fetch(&self, ctx: &FetchContext) -> Result<UsageSnapshot, FetchError> {
        let tokens = self.creds.load().await?;
        if !tokens.scopes.iter().any(|s| s == REQUIRED_SCOPE) {
            return Err(FetchError::Upstream(
                "Claude token lacks the user:profile scope required for usage".into(),
            ));
        }
        if let Some(exp) = tokens.expires_at {
            if exp <= self.clock.now() {
                return Err(FetchError::Upstream(
                    "Claude token expired — open Claude Code to refresh".into(),
                ));
            }
        }
        let req = HttpRequest {
            method: "GET".into(),
            url: USAGE_URL.into(),
            headers: vec![
                ("Authorization".into(), format!("Bearer {}", tokens.access_token)),
                ("anthropic-beta".into(), OAUTH_BETA.into()),
                ("User-Agent".into(), self.user_agent.clone()),
            ],
            body: None,
        };
        let resp = self.http.send(req).await?;
        match resp.status {
            200 => {
                let body = String::from_utf8_lossy(&resp.body);
                let windows = parse_usage(&body)?;
                Ok(UsageSnapshot {
                    provider: ctx.provider.clone(),
                    windows,
                    status: Status::Ok,
                    fetched_at: self.clock.now(),
                })
            }
            429 => Err(FetchError::RateLimited),
            s => Err(FetchError::Upstream(format!("HTTP {s}"))),
        }
    }

    fn should_fallback(&self, err: &FetchError) -> bool {
        matches!(err, FetchError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real captured shape of `api/oauth/usage` (usage values are not secret).
    const FIXTURE: &str = include_str!("testdata/claude_usage.json");

    #[test]
    fn parses_known_windows_and_skips_nulls() {
        let w = parse_usage(FIXTURE).expect("parse");
        // session + weekly + sonnet(custom); opus/oauth_apps/codenames are null → skipped;
        // extra_usage has null utilization → skipped.
        assert_eq!(w.len(), 3, "got: {w:#?}");

        assert_eq!(w[0].kind, WindowKind::Session);
        assert_eq!(w[0].used_percent, 4.0);
        assert_eq!(w[0].window_minutes, Some(300));
        assert!(w[0].resets_at.is_some());

        assert_eq!(w[1].kind, WindowKind::Weekly);
        assert_eq!(w[1].used_percent, 25.0);
        assert!(w[1].resets_at.is_some());

        assert_eq!(w[2].kind, WindowKind::Custom);
        assert_eq!(w[2].reset_description.as_deref(), Some("Sonnet · 7-day"));
        assert_eq!(w[2].used_percent, 0.0);
        assert!(w[2].resets_at.is_none());
    }

    #[test]
    fn malformed_input_is_an_error_not_a_panic() {
        assert!(parse_usage("not json").is_err());
        assert!(parse_usage("[]").is_err()); // not an object
        // An empty object is valid and simply yields no windows.
        assert_eq!(parse_usage("{}").unwrap().len(), 0);
    }

    // ---- Full strategy, exercised with fakes: no network, no Keychain, no real clock. ----
    struct FakeCreds(OAuthTokens);
    #[async_trait]
    impl OAuthCredentialSource for FakeCreds {
        async fn load(&self) -> Result<OAuthTokens, PortError> {
            Ok(self.0.clone())
        }
    }
    struct FakeHttp {
        status: u16,
        body: String,
    }
    #[async_trait]
    impl HttpPort for FakeHttp {
        async fn send(&self, _req: HttpRequest) -> Result<HttpResponse, PortError> {
            Ok(HttpResponse { status: self.status, body: self.body.clone().into_bytes() })
        }
    }
    struct FakeClock(i64);
    impl Clock for FakeClock {
        fn now(&self) -> Timestamp {
            Timestamp(self.0)
        }
    }

    fn tokens() -> OAuthTokens {
        OAuthTokens {
            access_token: "sk-ant-oat-test".into(),
            refresh_token: None,
            expires_at: Some(Timestamp(9_999_999_999_999)),
            scopes: vec!["user:profile".into(), "user:inference".into()],
            subscription_type: Some("team".into()),
        }
    }

    fn strategy(status: u16, body: &str, now: i64) -> ClaudeCodeStrategy {
        ClaudeCodeStrategy {
            creds: Arc::new(FakeCreds(tokens())),
            http: Arc::new(FakeHttp { status, body: body.into() }),
            clock: Arc::new(FakeClock(now)),
            user_agent: "claude-code/test".into(),
        }
    }

    #[tokio::test]
    async fn strategy_maps_200_into_a_snapshot() {
        let strat = strategy(200, FIXTURE, 1_700_000_000_000);
        let ctx = FetchContext { provider: ProviderId::new("claude-code") };
        let snap = strat.fetch(&ctx).await.expect("fetch");
        assert_eq!(snap.provider.as_str(), "claude-code");
        assert_eq!(snap.status, Status::Ok);
        assert_eq!(snap.fetched_at, Timestamp(1_700_000_000_000));
        assert_eq!(snap.windows.len(), 3);
    }

    #[tokio::test]
    async fn strategy_surfaces_429_as_rate_limited() {
        let strat = strategy(429, "", 1);
        let ctx = FetchContext { provider: ProviderId::new("claude-code") };
        assert!(matches!(strat.fetch(&ctx).await, Err(FetchError::RateLimited)));
    }
}
