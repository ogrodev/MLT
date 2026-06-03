//! Codex (ChatGPT / OpenAI subscription) provider.
//!
//! Reuses the Codex CLI's own OAuth login (read by an adapter from `~/.codex/auth.json`) and
//! polls the private `chatgpt.com/backend-api/wham/usage` endpoint. Parsers are pure and
//! deliberately lossy (ADR 0015): the response carries `rate_limit.primary_window` /
//! `secondary_window` (mapped to session / weekly by their length) plus optional, additive
//! `additional_rate_limits[]` (model-specific limits) — a malformed or absent field is skipped,
//! never fatal. See docs/research/PROVIDERS.md.
use super::{FetchContext, FetchError, FetchKind, FetchStrategy};
use crate::domain::*;
use crate::ports::*;
use async_trait::async_trait;
use std::sync::Arc;

/// Default ChatGPT usage endpoint. The base is overridable per `~/.codex/config.toml`
/// (`chatgpt_base_url`) for enterprise proxies; the standard install uses chatgpt.com, which we
/// default to. The strategy holds the resolved full URL so that override is an adapter concern.
pub const DEFAULT_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
/// OpenAI's OAuth token endpoint + the public Codex CLI client id, used to refresh a stale
/// access token (PROVIDERS.md). Intentionally NOT live-fire tested, to avoid rotating the
/// user's real Codex refresh token.
pub const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
/// Scope the OpenAI token endpoint requires in the refresh body.
pub const REFRESH_SCOPE: &str = "openid profile email";
/// Build the keychain key under which we cache OUR refreshed copy of one account's token,
/// namespaced by ChatGPT account id so multiple Codex logins never collide. Never written back
/// to the vendor's own store (`~/.codex/auth.json` or Oh My Pi's DB), which MLT only reads
/// (AGENTS.md invariant). The connect catalog and the fetch strategy both derive the key from
/// here, so disconnect purges exactly what the strategy wrote.
pub fn account_cache_key(account_id: &str) -> String {
    format!("oauth.codex.{account_id}")
}

/// The session (5h) and weekly (7d) windows, in minutes. We classify each window by its
/// reported length rather than trusting its position, so a server that swaps primary/secondary
/// still maps each window to the right kind (ported from CodexBar's rate-window normalizer).
const SESSION_MINUTES: i64 = 300;
const WEEKLY_MINUTES: i64 = 10_080;

/// The raw, lossy-decoded fields of one usage window. Position-independent: the caller assigns
/// the [`WindowKind`] and label.
struct RawWindow {
    used_percent: f64,
    window_minutes: Option<i64>,
    resets_at: Option<Timestamp>,
}

/// Extract a `{ used_percent, reset_at, limit_window_seconds }` window. Lossy: a `null` /
/// non-object value, or one without a numeric `used_percent`, yields `None` (skipped), never an
/// error. `reset_at` is Unix **seconds** (→ ms); `limit_window_seconds` → window minutes.
fn raw_window(val: Option<&serde_json::Value>) -> Option<RawWindow> {
    let obj = val?.as_object()?;
    let used_percent = obj
        .get("used_percent")
        .and_then(serde_json::Value::as_f64)?;
    let window_minutes = obj
        .get("limit_window_seconds")
        .and_then(serde_json::Value::as_i64)
        .map(|secs| secs / 60);
    let resets_at = obj
        .get("reset_at")
        .and_then(serde_json::Value::as_i64)
        .map(|secs| Timestamp(secs * 1000));
    Some(RawWindow {
        used_percent,
        window_minutes,
        resets_at,
    })
}

/// Classify a primary/secondary window by its length, falling back to its position when the
/// length is unfamiliar (so an unknown-span window still surfaces as something sensible).
fn main_kind(window_minutes: Option<i64>, fallback: WindowKind) -> WindowKind {
    match window_minutes {
        Some(SESSION_MINUTES) => WindowKind::Session,
        Some(WEEKLY_MINUTES) => WindowKind::Weekly,
        _ => fallback,
    }
}

/// Pure parser for the `wham/usage` body. Lossy by design (ADR 0015).
pub fn parse_usage(body: &str) -> Result<Vec<UsageWindow>, FetchError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| FetchError::Upstream(format!("bad json: {e}")))?;
    if !value.is_object() {
        return Err(FetchError::Upstream("expected a JSON object".into()));
    }

    let mut windows = Vec::new();

    // Primary (session, 5h) + secondary (weekly, 7d) lanes.
    if let Some(rate_limit) = value.get("rate_limit") {
        for (key, fallback) in [
            ("primary_window", WindowKind::Session),
            ("secondary_window", WindowKind::Weekly),
        ] {
            if let Some(raw) = raw_window(rate_limit.get(key)) {
                windows.push(UsageWindow {
                    kind: main_kind(raw.window_minutes, fallback),
                    used_percent: raw.used_percent,
                    window_minutes: raw.window_minutes,
                    resets_at: raw.resets_at,
                    reset_description: None,
                });
            }
        }
    }

    // Model-specific extra limits (e.g. a Codex Spark window). Additive and lossy: a missing
    // field or a malformed entry leaves the primary/weekly windows untouched. We surface each
    // limit's primary window as a labelled Custom window so it reads as a distinct lane.
    if let Some(extra) = value
        .get("additional_rate_limits")
        .and_then(serde_json::Value::as_array)
    {
        for entry in extra {
            let label = entry
                .get("limit_name")
                .or_else(|| entry.get("metered_feature"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from);
            if let Some(raw) = entry
                .get("rate_limit")
                .and_then(|rl| raw_window(rl.get("primary_window")))
            {
                windows.push(UsageWindow {
                    kind: WindowKind::Custom,
                    used_percent: raw.used_percent,
                    window_minutes: raw.window_minutes,
                    resets_at: raw.resets_at,
                    reset_description: label,
                });
            }
        }
    }

    Ok(windows)
}

/// Minimal base64url decoder for JWT segments (unpadded base64url). Pure; any invalid
/// character yields `None`, so a malformed token degrades to "no identity" rather than panics.
fn b64url_decode(input: &str) -> Option<Vec<u8>> {
    fn sextet(b: u8) -> Option<u32> {
        Some(match b {
            b'A'..=b'Z' => u32::from(b - b'A'),
            b'a'..=b'z' => u32::from(b - b'a') + 26,
            b'0'..=b'9' => u32::from(b - b'0') + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        })
    }
    let mut out = Vec::with_capacity(input.len() / 4 * 3 + 3);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for &b in input.as_bytes() {
        if b == b'=' {
            break; // tolerate optional padding
        }
        acc = (acc << 6) | sextet(b)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1; // drop the consumed high bits so `acc` can't overflow
        }
    }
    Some(out)
}

/// Decode a JWT's payload (the middle segment) into JSON claims. Best-effort and pure.
fn jwt_claims(token: &str) -> Option<serde_json::Value> {
    let payload = token.split('.').nth(1)?;
    serde_json::from_slice(&b64url_decode(payload)?).ok()
}

/// Best-effort, lossy account identity from a Codex OAuth token. Codex exposes no profile
/// endpoint, so the account email comes from the token's own JWT claims — the top-level `email`
/// or the `https://api.openai.com/profile` claim. Any shape we don't recognize yields an empty
/// identity rather than an error (ADR 0015): identity is a display nicety, never load-bearing.
pub fn parse_identity(access_token: &str) -> AccountIdentity {
    let Some(claims) = jwt_claims(access_token) else {
        return AccountIdentity::default();
    };
    let pick = |val: Option<&serde_json::Value>| {
        val.and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    let email = pick(claims.get("email")).or_else(|| {
        pick(
            claims
                .get("https://api.openai.com/profile")
                .and_then(|p| p.get("email")),
        )
    });
    AccountIdentity {
        email,
        organization: None,
    }
}

/// The access token's expiry, read from its JWT `exp` claim (Unix seconds → ms), or `None`
/// when there is no readable `exp`. The Codex CLI's `auth.json` carries no explicit expiry, so
/// the adapter uses this to populate [`OAuthTokens::expires_at`] — letting the shared
/// [`oauth::OAuthRefresher`](super::oauth) decide a refresh is due with the *same*
/// expiry-vs-clock logic it uses for Claude, instead of a bespoke staleness rule.
pub fn token_expiry(access_token: &str) -> Option<Timestamp> {
    let exp = jwt_claims(access_token)?.get("exp")?.as_i64()?;
    Some(Timestamp(exp * 1000))
}

/// The OAuth strategy for Codex: read the CLI's token, poll `wham/usage`.
pub struct CodexStrategy {
    pub creds: Arc<dyn OAuthCredentialSource>,
    pub http: Arc<dyn HttpPort>,
    pub clock: Arc<dyn Clock>,
    /// A Codex-CLI-style identifier, e.g. `"codex_cli_rs/0.20.0"`. The endpoint is not as
    /// UA-gated as Claude's, but we identify honestly rather than spoof a browser.
    pub user_agent: String,
    /// The full usage URL (default chatgpt.com; the adapter may override the base).
    pub usage_url: String,
    /// Caches the resolved account identity so the JWT is decoded at most once per account.
    pub identity: Arc<dyn IdentityStore>,
}

#[async_trait]
impl FetchStrategy for CodexStrategy {
    fn kind(&self) -> FetchKind {
        FetchKind::OAuth
    }

    async fn is_available(&self, _ctx: &FetchContext) -> bool {
        self.creds.load().await.is_ok()
    }

    async fn fetch(&self, ctx: &FetchContext) -> Result<UsageSnapshot, FetchError> {
        // The credential source returns a *valid* token (refreshing if needed).
        let tokens = self.creds.load().await?;
        let mut headers = vec![
            (
                "Authorization".into(),
                format!("Bearer {}", tokens.access_token),
            ),
            ("User-Agent".into(), self.user_agent.clone()),
            ("Accept".into(), "application/json".into()),
        ];
        // ChatGPT scopes usage to a workspace via this header; send it only when we have it.
        if let Some(account_id) = tokens.account_id.as_deref().filter(|s| !s.is_empty()) {
            headers.push(("ChatGPT-Account-Id".into(), account_id.into()));
        }
        let req = HttpRequest {
            method: "GET".into(),
            url: self.usage_url.clone(),
            headers,
            body: None,
        };
        let resp = self.http.send(req).await?;
        match resp.status {
            200 => {
                let body = String::from_utf8_lossy(&resp.body);
                let windows = parse_usage(&body)?;
                let account = self.resolve_identity(ctx, &tokens.access_token);
                Ok(UsageSnapshot {
                    provider: ctx.provider.clone(),
                    windows,
                    status: Status::Ok,
                    fetched_at: self.clock.now(),
                    account,
                })
            }
            429 => Err(FetchError::RateLimited),
            401 | 403 => Err(FetchError::Upstream(
                "Codex token expired or invalid — run `codex` to re-authenticate".into(),
            )),
            s => Err(FetchError::Upstream(format!("HTTP {s}"))),
        }
    }

    fn should_fallback(&self, err: &FetchError) -> bool {
        matches!(err, FetchError::Unavailable)
    }
}

impl CodexStrategy {
    /// The account identity for a snapshot: the cached value if present, otherwise decode it
    /// from the token's JWT claims (no network — unlike Claude, Codex has no profile endpoint)
    /// and cache it. Resolved at most once per account; an empty identity is never cached so a
    /// later token carrying claims can still resolve it.
    fn resolve_identity(&self, ctx: &FetchContext, access_token: &str) -> Option<AccountIdentity> {
        if let Ok(Some(cached)) = self.identity.identity(&ctx.provider) {
            return Some(cached);
        }
        let resolved = parse_identity(access_token);
        if resolved.is_empty() {
            return None;
        }
        let _ = self.identity.set_identity(&ctx.provider, &resolved);
        Some(resolved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use std::collections::HashMap;

    /// Captured shape of `wham/usage` (usage values are not secret).
    const FIXTURE: &str = include_str!("testdata/codex_usage.json");

    // JWTs built in tests (header.payload.sig; signature is irrelevant — we only read claims).
    const JWT_PROFILE_EMAIL: &str = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ1c2VyLWFiYyIsImh0dHBzOi8vYXBpLm9wZW5haS5jb20vYXV0aCI6eyJjaGF0Z3B0X2FjY291bnRfaWQiOiJhY2N0LTEyMyIsImNoYXRncHRfcGxhbl90eXBlIjoicHJvIn0sImh0dHBzOi8vYXBpLm9wZW5haS5jb20vcHJvZmlsZSI6eyJlbWFpbCI6ImNvZGV4dXNlckBleGFtcGxlLmNvbSIsImVtYWlsX3ZlcmlmaWVkIjp0cnVlfX0.sig";
    const JWT_TOPLEVEL_EMAIL: &str =
        "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJlbWFpbCI6InRvcEBleGFtcGxlLmNvbSJ9.sig";
    const JWT_NO_EMAIL: &str = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ4IiwiaHR0cHM6Ly9hcGkub3BlbmFpLmNvbS9hdXRoIjp7ImNoYXRncHRfcGxhbl90eXBlIjoicGx1cyJ9fQ.sig";

    #[test]
    fn parses_session_weekly_and_extra_windows() {
        let w = parse_usage(FIXTURE).expect("parse");
        assert_eq!(w.len(), 3, "got: {w:#?}");

        // primary_window (18000s = 5h) → Session.
        assert_eq!(w[0].kind, WindowKind::Session);
        assert_eq!(w[0].used_percent, 12.0);
        assert_eq!(w[0].window_minutes, Some(300));
        assert_eq!(w[0].resets_at, Some(Timestamp(1_780_000_000_000)));

        // secondary_window (604800s = 7d) → Weekly.
        assert_eq!(w[1].kind, WindowKind::Weekly);
        assert_eq!(w[1].used_percent, 47.0);
        assert_eq!(w[1].window_minutes, Some(10_080));

        // additional_rate_limits[0] → labelled Custom window.
        assert_eq!(w[2].kind, WindowKind::Custom);
        assert_eq!(
            w[2].reset_description.as_deref(),
            Some("GPT-5.3-Codex-Spark")
        );
        assert_eq!(w[2].used_percent, 3.0);
    }

    #[test]
    fn classifies_windows_by_length_not_position() {
        // A server that puts the weekly window in `primary` and the session in `secondary`
        // must still map each to the right kind (lengths, not slots, decide).
        let body = r#"{"rate_limit":{
            "primary_window":{"used_percent":50,"reset_at":1,"limit_window_seconds":604800},
            "secondary_window":{"used_percent":10,"reset_at":2,"limit_window_seconds":18000}}}"#;
        let w = parse_usage(body).expect("parse");
        assert_eq!(w[0].kind, WindowKind::Weekly);
        assert_eq!(w[1].kind, WindowKind::Session);
    }

    #[test]
    fn unknown_window_length_falls_back_to_position() {
        let body = r#"{"rate_limit":{
            "primary_window":{"used_percent":5,"reset_at":1,"limit_window_seconds":3600}}}"#;
        let w = parse_usage(body).expect("parse");
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].kind, WindowKind::Session); // unknown 60-min span → positional fallback
        assert_eq!(w[0].window_minutes, Some(60));
    }

    #[test]
    fn lossy_skips_null_and_unparseable_windows() {
        // null primary, a secondary missing used_percent → both skipped, never an error.
        let body = r#"{"rate_limit":{"primary_window":null,
            "secondary_window":{"reset_at":1,"limit_window_seconds":604800}}}"#;
        assert_eq!(parse_usage(body).unwrap().len(), 0);
    }

    #[test]
    fn malformed_input_is_an_error_not_a_panic() {
        assert!(parse_usage("not json").is_err());
        assert!(parse_usage("[]").is_err()); // not an object
        assert_eq!(parse_usage("{}").unwrap().len(), 0); // empty object → no windows
    }

    #[test]
    fn used_percent_accepts_int_or_float() {
        let body = r#"{"rate_limit":{
            "primary_window":{"used_percent":7.5,"reset_at":1,"limit_window_seconds":18000}}}"#;
        assert_eq!(parse_usage(body).unwrap()[0].used_percent, 7.5);
    }

    #[test]
    fn identity_reads_email_from_jwt_claims_lossily() {
        assert_eq!(
            parse_identity(JWT_PROFILE_EMAIL).email.as_deref(),
            Some("codexuser@example.com")
        );
        assert_eq!(
            parse_identity(JWT_TOPLEVEL_EMAIL).email.as_deref(),
            Some("top@example.com")
        );
        // No email claim anywhere → empty (we never invent one).
        assert!(parse_identity(JWT_NO_EMAIL).is_empty());
        // Malformed tokens degrade to empty, never panic.
        assert!(parse_identity("not.a.jwt").is_empty());
        assert!(parse_identity("garbage").is_empty());
        assert!(parse_identity("").is_empty());
        // Identity never carries an org for Codex.
        assert!(parse_identity(JWT_PROFILE_EMAIL).organization.is_none());
    }

    #[test]
    fn b64url_decode_round_trips_and_rejects_bad_input() {
        assert_eq!(b64url_decode("TWFu").unwrap(), b"Man");
        assert_eq!(b64url_decode("TWE").unwrap(), b"Ma"); // unpadded
        assert_eq!(b64url_decode("TWE=").unwrap(), b"Ma"); // padding tolerated
        assert!(b64url_decode("!!!!").is_none()); // invalid char
    }

    #[test]
    fn token_expiry_reads_exp_from_jwt() {
        const JWT_WITH_EXP: &str = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJleHAiOjE4OTM0NTYwMDAsImVtYWlsIjoiZXhwQGV4YW1wbGUuY29tIn0.sig";
        assert_eq!(
            token_expiry(JWT_WITH_EXP),
            Some(Timestamp(1_893_456_000_000))
        );
        // No `exp` claim, or an unparseable token → None (the refresher then trusts the token
        // and lets any real 401 surface, rather than refreshing blindly).
        assert_eq!(token_expiry(JWT_NO_EMAIL), None);
        assert_eq!(token_expiry("garbage"), None);
    }

    // ---- shared fakes: no network, no real clock ----
    struct FakeCreds(Option<OAuthTokens>);
    #[async_trait]
    impl OAuthCredentialSource for FakeCreds {
        async fn load(&self) -> Result<OAuthTokens, PortError> {
            self.0.clone().ok_or(PortError::NotFound)
        }
    }

    /// Records request headers so tests can assert what the strategy sent.
    struct FakeHttp {
        status: u16,
        body: String,
        last_headers: Mutex<Vec<(String, String)>>,
    }
    impl FakeHttp {
        fn new(status: u16, body: &str) -> Self {
            Self {
                status,
                body: body.into(),
                last_headers: Mutex::new(Vec::new()),
            }
        }
        fn header(&self, name: &str) -> Option<String> {
            self.last_headers
                .lock()
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        }
    }
    #[async_trait]
    impl HttpPort for FakeHttp {
        async fn send(&self, req: HttpRequest) -> Result<HttpResponse, PortError> {
            *self.last_headers.lock() = req.headers;
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
            Ok(self.0.lock().get(id.as_str()).cloned())
        }
        fn set_identity(
            &self,
            id: &ProviderId,
            identity: &AccountIdentity,
        ) -> Result<(), PortError> {
            self.0.lock().insert(id.as_str().into(), identity.clone());
            Ok(())
        }
        fn clear_identity(&self, id: &ProviderId) -> Result<(), PortError> {
            self.0.lock().remove(id.as_str());
            Ok(())
        }
    }

    fn tokens(access_token: &str, account_id: Option<&str>) -> OAuthTokens {
        OAuthTokens {
            access_token: access_token.into(),
            refresh_token: Some("rt".into()),
            expires_at: Some(Timestamp(9_999_999_999_999)),
            scopes: vec![],
            subscription_type: None,
            account_id: account_id.map(String::from),
        }
    }

    fn strategy(
        http: Arc<FakeHttp>,
        token: OAuthTokens,
        identity: Arc<FakeIdentity>,
    ) -> CodexStrategy {
        CodexStrategy {
            creds: Arc::new(FakeCreds(Some(token))),
            http,
            clock: Arc::new(FakeClock(1_700_000_000_000)),
            user_agent: "codex_cli_rs/test".into(),
            usage_url: DEFAULT_USAGE_URL.into(),
            identity,
        }
    }

    fn ctx() -> FetchContext {
        FetchContext {
            provider: ProviderId::new("codex"),
        }
    }

    #[tokio::test]
    async fn strategy_maps_200_into_a_snapshot_and_sends_account_header() {
        let http = Arc::new(FakeHttp::new(200, FIXTURE));
        let strat = strategy(
            http.clone(),
            tokens(JWT_PROFILE_EMAIL, Some("acct-123")),
            Arc::new(FakeIdentity::default()),
        );
        let snap = strat.fetch(&ctx()).await.expect("fetch");
        assert_eq!(snap.provider.as_str(), "codex");
        assert_eq!(snap.status, Status::Ok);
        assert_eq!(snap.fetched_at, Timestamp(1_700_000_000_000));
        assert_eq!(snap.windows.len(), 3);
        // Identity decoded from the token's JWT and attached to the snapshot.
        assert_eq!(
            snap.account.as_ref().and_then(|a| a.email.as_deref()),
            Some("codexuser@example.com")
        );
        // The workspace header is sent with the bearer token.
        assert_eq!(
            http.header("ChatGPT-Account-Id").as_deref(),
            Some("acct-123")
        );
        assert_eq!(
            http.header("Authorization").as_deref(),
            Some(format!("Bearer {JWT_PROFILE_EMAIL}").as_str())
        );
    }

    #[tokio::test]
    async fn strategy_omits_account_header_when_absent() {
        let http = Arc::new(FakeHttp::new(200, FIXTURE));
        let strat = strategy(
            http.clone(),
            tokens(JWT_TOPLEVEL_EMAIL, None),
            Arc::new(FakeIdentity::default()),
        );
        strat.fetch(&ctx()).await.expect("fetch");
        assert!(http.header("ChatGPT-Account-Id").is_none());
    }

    #[tokio::test]
    async fn strategy_surfaces_429_as_rate_limited() {
        let http = Arc::new(FakeHttp::new(429, ""));
        let strat = strategy(
            http,
            tokens(JWT_NO_EMAIL, None),
            Arc::new(FakeIdentity::default()),
        );
        assert!(matches!(
            strat.fetch(&ctx()).await,
            Err(FetchError::RateLimited)
        ));
    }

    #[tokio::test]
    async fn strategy_maps_401_to_an_upstream_error() {
        let http = Arc::new(FakeHttp::new(401, ""));
        let strat = strategy(
            http,
            tokens(JWT_NO_EMAIL, None),
            Arc::new(FakeIdentity::default()),
        );
        assert!(matches!(
            strat.fetch(&ctx()).await,
            Err(FetchError::Upstream(_))
        ));
    }

    #[tokio::test]
    async fn strategy_caches_identity_so_it_is_decoded_once() {
        let identity = Arc::new(FakeIdentity::default());
        let strat = strategy(
            Arc::new(FakeHttp::new(200, FIXTURE)),
            tokens(JWT_PROFILE_EMAIL, Some("acct-123")),
            identity.clone(),
        );
        strat.fetch(&ctx()).await.expect("fetch");
        assert_eq!(
            identity
                .identity(&ProviderId::new("codex"))
                .unwrap()
                .and_then(|a| a.email)
                .as_deref(),
            Some("codexuser@example.com")
        );
    }

    #[tokio::test]
    async fn strategy_leaves_account_none_when_token_has_no_email() {
        let strat = strategy(
            Arc::new(FakeHttp::new(200, FIXTURE)),
            tokens(JWT_NO_EMAIL, None),
            Arc::new(FakeIdentity::default()),
        );
        let snap = strat.fetch(&ctx()).await.expect("fetch");
        assert!(snap.account.is_none());
    }
}
