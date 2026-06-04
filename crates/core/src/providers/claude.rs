//! Claude Code (Anthropic subscription) provider.
//!
//! Reuses the Claude Code CLI's own OAuth token (read by an adapter from the file or the
//! macOS Keychain) and polls the private `api/oauth/usage` endpoint. The parser is pure and
//! deliberately lossy (ADR 0015): the endpoint returns a map of window → {utilization,
//! resets_at} with many null / experimental keys, so unknown or null windows are skipped,
//! never fatal. See docs/research/PROVIDERS.md.
use super::{FetchContext, FetchError, FetchKind, FetchStrategy};
use crate::domain::*;
use crate::ports::*;
use async_trait::async_trait;
use std::sync::Arc;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

pub const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
/// Account profile (email/org) for the OAuth login — read with the same token, used only to
/// show *which* account the panel reports. Verified to exist (401 without a token, not 404).
pub const PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";
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
        "seven_day_opus" => (
            WindowKind::Custom,
            Some(10_080),
            Some("Opus · 7-day".into()),
        ),
        "seven_day_sonnet" => (
            WindowKind::Custom,
            Some(10_080),
            Some("Sonnet · 7-day".into()),
        ),
        "seven_day_oauth_apps" => (
            WindowKind::Custom,
            Some(10_080),
            Some("OAuth apps · 7-day".into()),
        ),
        "seven_day_cowork" => (
            WindowKind::Custom,
            Some(10_080),
            Some("Cowork · 7-day".into()),
        ),
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

/// Pure, lossy parser for the OAuth profile body. Identity is account-identifying display
/// metadata, not a secret: we read the account email and organization name. Any shape we
/// don't recognize yields an empty identity rather than an error (ADR 0015) — identity is
/// best-effort, never load-bearing.
fn parse_profile(body: &[u8]) -> AccountIdentity {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) else {
        return AccountIdentity::default();
    };
    let pick = |val: Option<&serde_json::Value>| {
        val.and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    let email = pick(
        v.pointer("/account/email_address")
            .or_else(|| v.pointer("/account/email"))
            .or_else(|| v.get("email_address"))
            .or_else(|| v.get("email")),
    );
    let organization = pick(
        v.pointer("/organization/name")
            .or_else(|| v.get("organization_name")),
    );
    AccountIdentity {
        email,
        organization,
    }
}

/// Best-effort fetch of the Claude account profile (email/org) for display, using the same
/// OAuth token as the usage call. Never fatal: any failure — transport, non-200, unparseable,
/// or an empty profile — yields `None`, so identity never blocks or breaks a usage fetch.
async fn fetch_identity(
    http: &dyn HttpPort,
    access_token: &str,
    user_agent: &str,
) -> Option<AccountIdentity> {
    let req = HttpRequest {
        method: "GET".into(),
        url: PROFILE_URL.into(),
        headers: vec![
            ("Authorization".into(), format!("Bearer {access_token}")),
            ("anthropic-beta".into(), OAUTH_BETA.into()),
            ("User-Agent".into(), user_agent.into()),
        ],
        body: None,
    };
    let resp = http.send(req).await.ok()?;
    if resp.status != 200 {
        return None;
    }
    let identity = parse_profile(&resp.body);
    (!identity.is_empty()).then_some(identity)
}

/// The OAuth strategy for Claude Code: read the CLI's token, poll `api/oauth/usage`.
pub struct ClaudeCodeStrategy {
    pub creds: Arc<dyn OAuthCredentialSource>,
    pub http: Arc<dyn HttpPort>,
    pub clock: Arc<dyn Clock>,
    /// e.g. `"claude-code/2.1.158"`. REQUIRED — without the claude-code UA the endpoint
    /// rate-limits hard (persistent 429). See docs/research/PROVIDERS.md.
    pub user_agent: String,
    /// Caches the resolved account identity so the profile is fetched at most once, not on
    /// every poll — keeping the extra request off the rate-limited usage endpoint's back.
    pub identity: Arc<dyn IdentityStore>,
}

#[async_trait]
impl FetchStrategy for ClaudeCodeStrategy {
    fn kind(&self) -> FetchKind {
        FetchKind::OAuth
    }

    async fn is_available(&self, _ctx: &FetchContext) -> bool {
        matches!(self.creds.load().await, Ok(t) if t.scopes.is_empty() || t.scopes.iter().any(|s| s == REQUIRED_SCOPE))
    }

    async fn fetch(&self, ctx: &FetchContext) -> Result<UsageSnapshot, FetchError> {
        // The credential source is responsible for returning a *valid* token (refreshing if
        // needed). Fail fast only when the scopes are *known* and lack `user:profile`; a token
        // whose scopes we didn't record (e.g. an Oh My Pi account, whose stored blob omits them)
        // is trusted and the endpoint decides — it 401s if the scope is truly absent.
        let tokens = self.creds.load().await?;
        if !tokens.scopes.is_empty() && !tokens.scopes.iter().any(|s| s == REQUIRED_SCOPE) {
            return Err(FetchError::Upstream(
                "Claude token lacks the user:profile scope required for usage".into(),
            ));
        }
        let req = HttpRequest {
            method: "GET".into(),
            url: USAGE_URL.into(),
            headers: vec![
                (
                    "Authorization".into(),
                    format!("Bearer {}", tokens.access_token),
                ),
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
                let account = self.resolve_identity(ctx, &tokens.access_token).await;
                Ok(UsageSnapshot {
                    provider: ctx.provider.clone(),
                    windows,
                    status: Status::Ok,
                    fetched_at: self.clock.now(),
                    account,
                    note: None,
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

impl ClaudeCodeStrategy {
    /// The account identity for a snapshot: the cached value if present, otherwise a one-shot
    /// best-effort profile fetch that is then cached. Fetched at most once per account — later
    /// polls hit the cache — so identity adds no recurring load to the usage endpoint. Any
    /// failure leaves identity `None` and never affects the usage result.
    async fn resolve_identity(
        &self,
        ctx: &FetchContext,
        access_token: &str,
    ) -> Option<AccountIdentity> {
        if let Ok(Some(cached)) = self.identity.identity(&ctx.provider) {
            return Some(cached);
        }
        let fetched = fetch_identity(self.http.as_ref(), access_token, &self.user_agent).await?;
        let _ = self.identity.set_identity(&ctx.provider, &fetched);
        Some(fetched)
    }
}

// ---- Token refresh config -----------------------------------------------------------------
//
// The refresh *logic* is the shared [`crate::providers::oauth::OAuthRefresher`]; these consts
// are just Claude Code's endpoint / client / cache wiring for it.

/// Anthropic's OAuth token endpoint + the public Claude Code PKCE client id (overridable).
/// Sourced from research (docs/research/PROVIDERS.md); intentionally NOT live-fire tested,
/// to avoid rotating the user's real Claude Code refresh token.
pub const DEFAULT_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
pub const DEFAULT_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
/// Key under which we cache OUR refreshed copy — never written back to Claude Code's store.
pub const CACHE_KEY: &str = "oauth.claude";

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

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
        assert_eq!(parse_usage("{}").unwrap().len(), 0); // empty object → no windows
    }

    // ---- shared fakes: no network, no Keychain, no real clock ----
    struct FakeCreds(Option<OAuthTokens>);
    #[async_trait]
    impl OAuthCredentialSource for FakeCreds {
        async fn load(&self) -> Result<OAuthTokens, PortError> {
            self.0.clone().ok_or(PortError::NotFound)
        }
    }
    struct FakeHttp {
        status: u16,
        body: String,
        calls: AtomicUsize,
    }
    impl FakeHttp {
        fn new(status: u16, body: &str) -> Self {
            Self {
                status,
                body: body.into(),
                calls: AtomicUsize::new(0),
            }
        }
    }
    #[async_trait]
    impl HttpPort for FakeHttp {
        async fn send(&self, _req: HttpRequest) -> Result<HttpResponse, PortError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(HttpResponse {
                status: self.status,
                body: self.body.clone().into_bytes(),
            })
        }
    }
    struct FakeClock(i64);
    impl Clock for FakeClock {
        fn now(&self) -> Timestamp {
            Timestamp(self.0)
        }
    }

    #[derive(Default)]
    struct FakeIdentity(Mutex<HashMap<String, AccountIdentity>>);
    impl IdentityStore for FakeIdentity {
        fn identity(&self, id: &ProviderId) -> Result<Option<AccountIdentity>, PortError> {
            Ok(self.0.lock().unwrap().get(id.as_str()).cloned())
        }
        fn set_identity(
            &self,
            id: &ProviderId,
            identity: &AccountIdentity,
        ) -> Result<(), PortError> {
            self.0
                .lock()
                .unwrap()
                .insert(id.as_str().into(), identity.clone());
            Ok(())
        }
        fn clear_identity(&self, id: &ProviderId) -> Result<(), PortError> {
            self.0.lock().unwrap().remove(id.as_str());
            Ok(())
        }
    }

    const PROFILE_FIXTURE: &str = r#"{"account":{"uuid":"u1","email_address":"dev@example.com"},
        "organization":{"uuid":"o1","name":"Acme"}}"#;

    fn tokens_expiring_at(exp: i64) -> OAuthTokens {
        OAuthTokens {
            access_token: "old-access".into(),
            refresh_token: Some("rt-old".into()),
            expires_at: Some(Timestamp(exp)),
            scopes: vec!["user:profile".into(), "user:inference".into()],
            subscription_type: Some("team".into()),
            account_id: None,
        }
    }

    // ---- strategy ----
    #[tokio::test]
    async fn strategy_maps_200_into_a_snapshot() {
        let strat = ClaudeCodeStrategy {
            creds: Arc::new(FakeCreds(Some(tokens_expiring_at(9_999_999_999_999)))),
            http: Arc::new(FakeHttp::new(200, FIXTURE)),
            clock: Arc::new(FakeClock(1_700_000_000_000)),
            user_agent: "claude-code/test".into(),
            identity: Arc::new(FakeIdentity::default()),
        };
        let ctx = FetchContext {
            provider: ProviderId::new("claude-code"),
        };
        let snap = strat.fetch(&ctx).await.expect("fetch");
        assert_eq!(snap.provider.as_str(), "claude-code");
        assert_eq!(snap.status, Status::Ok);
        assert_eq!(snap.fetched_at, Timestamp(1_700_000_000_000));
        assert_eq!(snap.windows.len(), 3);
        // The usage body carries no identity, so a profile parsed from it stays empty → None.
        assert!(snap.account.is_none());
    }

    #[tokio::test]
    async fn strategy_surfaces_429_as_rate_limited() {
        let strat = ClaudeCodeStrategy {
            creds: Arc::new(FakeCreds(Some(tokens_expiring_at(9_999_999_999_999)))),
            http: Arc::new(FakeHttp::new(429, "")),
            clock: Arc::new(FakeClock(1)),
            user_agent: "claude-code/test".into(),
            identity: Arc::new(FakeIdentity::default()),
        };
        let ctx = FetchContext {
            provider: ProviderId::new("claude-code"),
        };
        assert!(matches!(
            strat.fetch(&ctx).await,
            Err(FetchError::RateLimited)
        ));
    }

    #[test]
    fn parse_profile_reads_email_and_org_lossily() {
        let id = parse_profile(PROFILE_FIXTURE.as_bytes());
        assert_eq!(id.email.as_deref(), Some("dev@example.com"));
        assert_eq!(id.organization.as_deref(), Some("Acme"));
        // Lossy (ADR 0015): malformed / unknown shapes degrade to empty, never an error.
        assert!(parse_profile(b"not json").is_empty());
        assert!(parse_profile(b"{}").is_empty());
        // A partial profile keeps what it can.
        let only_email = parse_profile(br#"{"account":{"email_address":"x@y.z"}}"#);
        assert_eq!(only_email.email.as_deref(), Some("x@y.z"));
        assert_eq!(only_email.organization, None);
    }

    #[tokio::test]
    async fn fetch_identity_is_some_on_200_and_none_otherwise() {
        let got = fetch_identity(&FakeHttp::new(200, PROFILE_FIXTURE), "tok", "ua").await;
        assert_eq!(got.unwrap().email.as_deref(), Some("dev@example.com"));
        // A non-200 (e.g. rejected) yields no identity — we never invent one.
        assert!(fetch_identity(&FakeHttp::new(401, ""), "tok", "ua")
            .await
            .is_none());
        // 200 with nothing recognizable → None (nothing worth showing).
        assert!(fetch_identity(&FakeHttp::new(200, "{}"), "tok", "ua")
            .await
            .is_none());
    }

    #[tokio::test]
    async fn fetch_identity_fails_closed_when_the_profile_call_errors() {
        struct OfflineHttp;
        #[async_trait]
        impl HttpPort for OfflineHttp {
            async fn send(&self, _req: HttpRequest) -> Result<HttpResponse, PortError> {
                Err(PortError::Io("offline".into()))
            }
        }
        assert!(fetch_identity(&OfflineHttp, "tok", "ua").await.is_none());
    }

    /// Routes by URL: the usage fixture for the usage endpoint, a profile body for the
    /// profile endpoint, counting profile hits to prove it is fetched at most once.
    struct RoutingHttp {
        profile_body: String,
        profile_calls: AtomicUsize,
    }
    #[async_trait]
    impl HttpPort for RoutingHttp {
        async fn send(&self, req: HttpRequest) -> Result<HttpResponse, PortError> {
            let body = if req.url.contains("/profile") {
                self.profile_calls.fetch_add(1, Ordering::SeqCst);
                self.profile_body.clone()
            } else {
                FIXTURE.to_string()
            };
            Ok(HttpResponse {
                status: 200,
                body: body.into_bytes(),
            })
        }
    }

    #[tokio::test]
    async fn strategy_attaches_identity_and_caches_it() {
        let http = Arc::new(RoutingHttp {
            profile_body: PROFILE_FIXTURE.into(),
            profile_calls: AtomicUsize::new(0),
        });
        let identity = Arc::new(FakeIdentity::default());
        let strat = ClaudeCodeStrategy {
            creds: Arc::new(FakeCreds(Some(tokens_expiring_at(9_999_999_999_999)))),
            http: http.clone(),
            clock: Arc::new(FakeClock(1)),
            user_agent: "claude-code/test".into(),
            identity: identity.clone(),
        };
        let ctx = FetchContext {
            provider: ProviderId::new("claude-code"),
        };
        let snap = strat.fetch(&ctx).await.expect("fetch");
        assert_eq!(
            snap.account.as_ref().and_then(|a| a.email.as_deref()),
            Some("dev@example.com")
        );
        // Cached for the source, so the panel can show it without re-fetching.
        assert_eq!(
            identity
                .identity(&ProviderId::new("claude-code"))
                .unwrap()
                .and_then(|a| a.email)
                .as_deref(),
            Some("dev@example.com")
        );
        // A second poll resolves identity from cache — no extra request to the rate-limited host.
        strat.fetch(&ctx).await.expect("fetch");
        assert_eq!(
            http.profile_calls.load(Ordering::SeqCst),
            1,
            "profile fetched at most once"
        );
    }

    #[tokio::test]
    async fn strategy_accepts_a_token_whose_scopes_are_unknown() {
        // An Oh My Pi account's stored blob omits scopes; the guard must not reject such a token
        // (the endpoint is the authority). A token with KNOWN scopes lacking user:profile is
        // still rejected — that path is unchanged.
        let mut token = tokens_expiring_at(9_999_999_999_999);
        token.scopes = Vec::new();
        let strat = ClaudeCodeStrategy {
            creds: Arc::new(FakeCreds(Some(token))),
            http: Arc::new(RoutingHttp {
                profile_body: PROFILE_FIXTURE.into(),
                profile_calls: AtomicUsize::new(0),
            }),
            clock: Arc::new(FakeClock(1)),
            user_agent: "claude-code/test".into(),
            identity: Arc::new(FakeIdentity::default()),
        };
        let ctx = FetchContext {
            provider: ProviderId::new("claude-code:acct-1"),
        };
        let snap = strat
            .fetch(&ctx)
            .await
            .expect("a token with unknown (empty) scopes is accepted, not rejected");
        assert_eq!(snap.provider.as_str(), "claude-code:acct-1");
        assert!(!snap.windows.is_empty(), "usage parsed from the fixture");
    }
}
