//! OpenRouter API-key validation (task 003) **and** usage reading (task 006).
//!
//! OpenRouter is an API-cost provider (ADR 0014): the user pastes a normal `sk-or-v1…` key
//! (stored by task 003 in our keychain) and we read their credit standing with it.
//!
//! - **Validation** ([`validate_key`]): prove a pasted key authenticates before we store it,
//!   with the cheapest authenticated call OpenRouter offers — `GET /api/v1/key` returns 200 for
//!   a usable key and 401/403 for a bad one — reading only the status.
//! - **Usage** ([`OpenRouterStrategy`]): poll `GET /api/v1/key` for the key's spend cap/usage
//!   and, best-effort, `GET /api/v1/credits` for the account's prepaid balance, mapping them to
//!   a single credit window. OpenRouter bills prepaid credit, not a resetting window, so that
//!   window honestly carries no reset countdown (task 006 AC).
//!
//! All HTTP IO is injected via [`HttpPort`] and the key is read via [`SecretStore`], so the
//! parsing/decision logic is pure and unit-tested against fakes — no live account is touched in
//! `cargo test` (a live check is the hand-run `openrouter_live` example).
use crate::domain::{Status, UsageSnapshot, UsageWindow, WindowKind};
use crate::ports::{Clock, HttpPort, HttpRequest, SecretStore};
use crate::sources::api_key_secret_key;
use async_trait::async_trait;
use std::sync::Arc;

use super::{FetchContext, FetchError, FetchKind, FetchStrategy};

/// OpenRouter's key-info endpoint. A 200 authenticated GET proves the key works (used by
/// [`validate_key`]), and its body carries the key's spend cap/usage (read by
/// [`OpenRouterStrategy`]).
pub const KEY_URL: &str = "https://openrouter.ai/api/v1/key";

/// Why a pasted API key could not be accepted. The `Display` text is **user-facing** — it is
/// what the popover shows — so each variant reads as a clear, actionable message (acceptance:
/// an invalid/rejected key shows a clear error and never silently appears connected).
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ApiKeyError {
    /// No key was entered (caught locally, before any network call).
    #[error("Enter an API key.")]
    Empty,
    /// The provider authenticated the request and refused the key (HTTP 401/403).
    #[error("That key was rejected by OpenRouter. Check it and try again.")]
    Rejected,
    /// The verification request itself failed (offline, DNS, TLS …). Not a verdict on the key.
    #[error("Couldn't reach OpenRouter to verify the key. Check your connection and try again.")]
    Unreachable,
    /// The endpoint answered with a status we don't treat as a clear accept/reject.
    #[error("OpenRouter returned an unexpected response (HTTP {0}).")]
    Unexpected(u16),
}

/// Validate an OpenRouter key by authenticating against [`KEY_URL`]. Returns `Ok(())` only when
/// the key is accepted (HTTP 200). It never returns or logs the key. A blank key is rejected
/// locally, without a network round-trip, and any transport failure is reported as
/// [`ApiKeyError::Unreachable`] (fail closed — an unverified key is never treated as good).
pub async fn validate_key(http: &dyn HttpPort, key: &str) -> Result<(), ApiKeyError> {
    let key = key.trim();
    if key.is_empty() {
        return Err(ApiKeyError::Empty);
    }
    match http.send(bearer_get(KEY_URL, key)).await {
        Ok(resp) => match resp.status {
            200 => Ok(()),
            401 | 403 => Err(ApiKeyError::Rejected),
            other => Err(ApiKeyError::Unexpected(other)),
        },
        Err(_) => Err(ApiKeyError::Unreachable),
    }
}

// ── Usage reading (task 006) ───────────────────────────────────────────────────

/// OpenRouter's account-wide credit balance endpoint. `total_credits` (everything granted or
/// purchased) minus `total_usage` (everything spent) is the remaining balance. Best-effort: the
/// docs mark it "management key required", so a normal key may be refused — we then fall back to
/// the key's own limit/usage (PROVIDERS.md).
pub const CREDITS_URL: &str = "https://openrouter.ai/api/v1/credits";

/// What we read from `GET /api/v1/key` (under its `data` envelope). Lossy (ADR 0015): an absent
/// or oddly-shaped field degrades to a sensible default rather than failing the whole snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyInfo {
    /// Per-key spend cap in credits, or `None` when the key draws on the account balance with no
    /// key-specific limit.
    pub limit: Option<f64>,
    /// Credits spent on this key (a lifetime aggregate).
    pub usage: f64,
}

/// The account credit balance from [`CREDITS_URL`]: `total_credits` granted/purchased vs
/// `total_usage` spent. Used to compute headroom when the key itself carries no spend cap.
#[derive(Debug, Clone, PartialEq)]
pub struct Credits {
    pub total_credits: f64,
    pub total_usage: f64,
}

/// A numeric field as `f64` (JSON ints and floats both), or `None` when absent/null/non-numeric.
fn number(obj: &serde_json::Value, key: &str) -> Option<f64> {
    obj.get(key).and_then(serde_json::Value::as_f64)
}

/// Parse `GET /api/v1/key`. Bad JSON is fatal (an upstream error); a missing `usage` reads as 0
/// and an absent/`null` `limit` as "no per-key cap" (ADR 0015 lossy decoding). OpenRouter nests
/// the payload under `data`; we tolerate a bare object too.
pub fn parse_key_info(body: &str) -> Result<KeyInfo, FetchError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| FetchError::Upstream(format!("bad json: {e}")))?;
    let data = value.get("data").unwrap_or(&value);
    Ok(KeyInfo {
        limit: number(data, "limit"),
        usage: number(data, "usage").unwrap_or(0.0),
    })
}

/// Parse `GET /api/v1/credits`, best-effort: any missing/garbled field yields `None` so a
/// refused or malformed credits call never breaks the snapshot — we fall back to the key's own
/// limit/usage instead.
pub fn parse_credits(body: &str) -> Option<Credits> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let data = value.get("data").unwrap_or(&value);
    Some(Credits {
        total_credits: number(data, "total_credits")?,
        total_usage: number(data, "total_usage").unwrap_or(0.0),
    })
}

/// Used fraction of a limit as a percent in `[0, 100]`. A non-finite or non-positive limit
/// yields 0 rather than NaN/∞ — defensive against odd upstream numbers.
fn percent(used: f64, limit: f64) -> f64 {
    if !limit.is_finite() || limit <= 0.0 {
        return 0.0;
    }
    ((used / limit) * 100.0).clamp(0.0, 100.0)
}

/// The key's usable per-key spend cap (finite and positive), if any. When present it alone
/// determines the credit window, so the best-effort account-wide credits call can be skipped.
fn key_cap(limit: Option<f64>) -> Option<f64> {
    limit.filter(|l| l.is_finite() && *l > 0.0)
}

/// The single credit window the popover shows for OpenRouter. OpenRouter bills **prepaid
/// credit**, not a resetting time window, so the window deliberately carries no `resets_at` (and
/// no `window_minutes`): the UI shows no countdown rather than inventing one (task 006 AC). We
/// report the most specific bound available — a per-key spend cap if the key has one, else the
/// account's prepaid balance, else an honest "no spending limit" when neither bound exists.
pub fn usage_windows(key: &KeyInfo, credits: Option<&Credits>) -> Vec<UsageWindow> {
    let (used_percent, description) = if let Some(limit) = key_cap(key.limit) {
        (percent(key.usage, limit), "Credit used")
    } else if let Some(c) = credits.filter(|c| c.total_credits.is_finite() && c.total_credits > 0.0)
    {
        (percent(c.total_usage, c.total_credits), "Credit used")
    } else {
        (0.0, "No spending limit")
    };
    vec![UsageWindow {
        kind: WindowKind::Custom,
        used_percent,
        window_minutes: None,
        resets_at: None,
        reset_description: Some(description.to_string()),
    }]
}

/// A bearer-authenticated `GET` of `url` asking for JSON. Shared by the key and credits calls.
fn bearer_get(url: &str, key: &str) -> HttpRequest {
    HttpRequest {
        method: "GET".into(),
        url: url.into(),
        headers: vec![
            ("Authorization".into(), format!("Bearer {key}")),
            ("Accept".into(), "application/json".into()),
        ],
        body: None,
    }
}

/// The API-key fetch strategy for OpenRouter: read the user-entered key from our keychain (via
/// the [`SecretStore`] port — never the network) and poll the key + credits endpoints. There is
/// no OAuth to refresh, so this strategy only ever *reads* the stored key and writes nothing
/// back. The snapshot reports no account identity: OpenRouter's key endpoint exposes none, and
/// we never invent one (ADR 0015 — an all-`None` identity is simply not shown).
pub struct OpenRouterStrategy {
    pub secrets: Arc<dyn SecretStore>,
    pub http: Arc<dyn HttpPort>,
    pub clock: Arc<dyn Clock>,
}

#[async_trait]
impl FetchStrategy for OpenRouterStrategy {
    fn kind(&self) -> FetchKind {
        FetchKind::ApiToken
    }

    async fn is_available(&self, ctx: &FetchContext) -> bool {
        self.api_key(ctx).is_some()
    }

    async fn fetch(&self, ctx: &FetchContext) -> Result<UsageSnapshot, FetchError> {
        let key = self.api_key(ctx).ok_or(FetchError::Unavailable)?;
        let resp = self.http.send(bearer_get(KEY_URL, &key)).await?;
        let key_info = match resp.status {
            200 => parse_key_info(&String::from_utf8_lossy(&resp.body))?,
            401 | 403 => {
                return Err(FetchError::Upstream(
                    "OpenRouter rejected the key — reconnect it".into(),
                ))
            }
            429 => return Err(FetchError::RateLimited),
            s => return Err(FetchError::Upstream(format!("HTTP {s}"))),
        };
        // The account balance is only the *fallback* bound, so fetch it only when the key has no
        // usable spend cap of its own. This skips a recurring round-trip on the common capped key
        // (OpenRouter refuses /credits for a normal inference key anyway). Best-effort: any
        // non-200 or transport failure degrades to None — never a failed snapshot.
        let credits = if key_cap(key_info.limit).is_some() {
            None
        } else {
            match self.http.send(bearer_get(CREDITS_URL, &key)).await {
                Ok(resp) if resp.status == 200 => {
                    parse_credits(&String::from_utf8_lossy(&resp.body))
                }
                _ => None,
            }
        };
        Ok(UsageSnapshot {
            provider: ctx.provider.clone(),
            windows: usage_windows(&key_info, credits.as_ref()),
            status: Status::Ok,
            fetched_at: self.clock.now(),
            account: None,
        })
    }

    fn should_fallback(&self, err: &FetchError) -> bool {
        matches!(err, FetchError::Unavailable)
    }
}

impl OpenRouterStrategy {
    /// The user-entered key from our keychain for this source — trimmed and non-empty — or
    /// `None` when the source isn't connected (no key stored), which reads as "unavailable".
    fn api_key(&self, ctx: &FetchContext) -> Option<String> {
        self.secrets
            .get(&api_key_secret_key(&ctx.provider))
            .ok()
            .flatten()
            .map(|key| key.trim().to_string())
            .filter(|key| !key.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ProviderId, Timestamp};
    use crate::ports::{HttpResponse, PortError};
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::sync::Mutex;

    /// Scripts one HTTP outcome, records how many requests were sent, and captures the last
    /// request so a test can assert exactly what hit the wire.
    struct FakeHttp {
        outcome: Result<u16, ()>,
        calls: AtomicUsize,
        last: Mutex<Option<HttpRequest>>,
    }
    impl FakeHttp {
        fn status(status: u16) -> Self {
            Self {
                outcome: Ok(status),
                calls: AtomicUsize::new(0),
                last: Mutex::new(None),
            }
        }
        fn transport_error() -> Self {
            Self {
                outcome: Err(()),
                calls: AtomicUsize::new(0),
                last: Mutex::new(None),
            }
        }
    }
    #[async_trait]
    impl HttpPort for FakeHttp {
        async fn send(&self, req: HttpRequest) -> Result<HttpResponse, PortError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.last.lock().unwrap() = Some(req);
            match self.outcome {
                Ok(status) => Ok(HttpResponse {
                    status,
                    body: Vec::new(),
                }),
                Err(()) => Err(PortError::Io("offline".into())),
            }
        }
    }

    #[tokio::test]
    async fn accepts_a_key_the_endpoint_authenticates() {
        let http = FakeHttp::status(200);
        assert_eq!(validate_key(&http, "sk-or-v1-good").await, Ok(()));
        // One authenticated GET to the key endpoint, carrying the bearer token.
        let req = http
            .last
            .lock()
            .unwrap()
            .take()
            .expect("a request was sent");
        assert_eq!(req.method, "GET");
        assert_eq!(req.url, KEY_URL);
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "Authorization" && v == "Bearer sk-or-v1-good"));
    }

    #[tokio::test]
    async fn rejects_a_key_the_endpoint_refuses() {
        for status in [401u16, 403] {
            let http = FakeHttp::status(status);
            assert_eq!(
                validate_key(&http, "sk-or-v1-bad").await,
                Err(ApiKeyError::Rejected)
            );
        }
    }

    #[tokio::test]
    async fn surfaces_an_unexpected_status_distinctly() {
        let http = FakeHttp::status(500);
        assert_eq!(
            validate_key(&http, "k").await,
            Err(ApiKeyError::Unexpected(500))
        );
    }

    #[tokio::test]
    async fn a_transport_failure_is_unverified_not_rejected() {
        // Fail closed: we couldn't verify, so the key must not be treated as good — but it's a
        // distinct, recoverable message, not an outright rejection.
        let http = FakeHttp::transport_error();
        assert_eq!(
            validate_key(&http, "k").await,
            Err(ApiKeyError::Unreachable)
        );
    }

    #[tokio::test]
    async fn a_blank_key_is_rejected_without_a_network_call() {
        let http = FakeHttp::status(200);
        assert_eq!(validate_key(&http, "   ").await, Err(ApiKeyError::Empty));
        assert_eq!(
            http.calls.load(Ordering::SeqCst),
            0,
            "a blank key must never reach the network"
        );
    }

    #[tokio::test]
    async fn trims_surrounding_whitespace_from_a_pasted_key() {
        let http = FakeHttp::status(200);
        assert_eq!(validate_key(&http, "  sk-or-v1-good\n").await, Ok(()));
        let req = http
            .last
            .lock()
            .unwrap()
            .take()
            .expect("a request was sent");
        assert!(req.headers.iter().any(|(_, v)| v == "Bearer sk-or-v1-good"));
    }

    // ── Usage reading (task 006) ───────────────────────────────────────────────

    /// Maps a request URL to a scripted `(status, body)` and records every request. An
    /// unscripted URL fails as a transport error, so a test that scripts only `/key` exercises
    /// the best-effort `/credits` fallback.
    struct ScriptedHttp {
        routes: HashMap<String, (u16, String)>,
        sent: Mutex<Vec<HttpRequest>>,
    }
    impl ScriptedHttp {
        fn new(routes: &[(&str, u16, &str)]) -> Self {
            Self {
                routes: routes
                    .iter()
                    .map(|(url, status, body)| ((*url).to_string(), (*status, (*body).to_string())))
                    .collect(),
                sent: Mutex::new(Vec::new()),
            }
        }
    }
    #[async_trait]
    impl HttpPort for ScriptedHttp {
        async fn send(&self, req: HttpRequest) -> Result<HttpResponse, PortError> {
            let routed = self.routes.get(&req.url).cloned();
            self.sent.lock().unwrap().push(req);
            match routed {
                Some((status, body)) => Ok(HttpResponse {
                    status,
                    body: body.into_bytes(),
                }),
                None => Err(PortError::Io("unscripted url".into())),
            }
        }
    }

    /// A keychain stub returning a fixed (or absent) key for any entry name.
    struct KeySecrets(Option<String>);
    impl SecretStore for KeySecrets {
        fn get(&self, _key: &str) -> Result<Option<String>, PortError> {
            Ok(self.0.clone())
        }
        fn set(&self, _key: &str, _value: &str) -> Result<(), PortError> {
            Ok(())
        }
        fn delete(&self, _key: &str) -> Result<(), PortError> {
            Ok(())
        }
    }

    struct FakeClock(i64);
    impl Clock for FakeClock {
        fn now(&self) -> Timestamp {
            Timestamp(self.0)
        }
    }

    const FETCHED_AT: i64 = 1_700_000_000_000;

    fn strategy(key: Option<&str>, http: ScriptedHttp) -> OpenRouterStrategy {
        OpenRouterStrategy {
            secrets: Arc::new(KeySecrets(key.map(String::from))),
            http: Arc::new(http),
            clock: Arc::new(FakeClock(FETCHED_AT)),
        }
    }

    fn ctx() -> FetchContext {
        FetchContext {
            provider: ProviderId::new("openrouter"),
        }
    }

    #[test]
    fn parse_key_info_reads_limit_and_usage() {
        let info = parse_key_info(r#"{"data":{"limit":10.0,"usage":2.5}}"#).unwrap();
        assert_eq!(info.limit, Some(10.0));
        assert_eq!(info.usage, 2.5);
    }

    #[test]
    fn parse_key_info_is_lossy_for_missing_or_null_fields() {
        // A missing `usage` reads as 0 and a `null` limit as "no cap" — never an error.
        let info = parse_key_info(r#"{"data":{"limit":null}}"#).unwrap();
        assert_eq!(info.limit, None);
        assert_eq!(info.usage, 0.0);
        // A bare object (no `data` envelope) is tolerated, and integers parse as f64.
        let bare = parse_key_info(r#"{"limit":7,"usage":3}"#).unwrap();
        assert_eq!(bare.limit, Some(7.0));
        assert_eq!(bare.usage, 3.0);
    }

    #[test]
    fn parse_key_info_rejects_non_json() {
        assert!(parse_key_info("not json").is_err());
    }

    #[test]
    fn parse_credits_reads_totals_and_is_best_effort() {
        let c = parse_credits(r#"{"data":{"total_credits":20,"total_usage":8}}"#).unwrap();
        assert_eq!(c.total_credits, 20.0);
        assert_eq!(c.total_usage, 8.0);
        // A missing total or non-JSON yields None, so a refused/garbled call is simply skipped.
        assert!(parse_credits(r#"{"data":{"total_usage":8}}"#).is_none());
        assert!(parse_credits("nope").is_none());
    }

    #[test]
    fn usage_window_prefers_a_key_spend_cap() {
        // The per-key cap (10/40) wins over the account balance when the key has one.
        let windows = usage_windows(
            &KeyInfo {
                limit: Some(40.0),
                usage: 10.0,
            },
            Some(&Credits {
                total_credits: 100.0,
                total_usage: 90.0,
            }),
        );
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].used_percent, 25.0);
        assert_eq!(windows[0].reset_description.as_deref(), Some("Credit used"));
    }

    #[test]
    fn usage_window_falls_back_to_the_account_balance_without_a_key_cap() {
        let windows = usage_windows(
            &KeyInfo {
                limit: None,
                usage: 999.0,
            },
            Some(&Credits {
                total_credits: 50.0,
                total_usage: 20.0,
            }),
        );
        assert_eq!(windows[0].used_percent, 40.0);
        assert_eq!(windows[0].reset_description.as_deref(), Some("Credit used"));
    }

    #[test]
    fn usage_window_states_no_limit_honestly() {
        // No key cap and no prepaid balance: don't invent a number, say so plainly.
        let windows = usage_windows(
            &KeyInfo {
                limit: None,
                usage: 5.0,
            },
            None,
        );
        assert_eq!(windows[0].used_percent, 0.0);
        assert_eq!(
            windows[0].reset_description.as_deref(),
            Some("No spending limit")
        );
    }

    #[test]
    fn usage_window_never_invents_a_reset_countdown() {
        // AC: where OpenRouter exposes no reset window, the UI gets no countdown to render.
        let cases = [
            usage_windows(
                &KeyInfo {
                    limit: Some(10.0),
                    usage: 1.0,
                },
                None,
            ),
            usage_windows(
                &KeyInfo {
                    limit: None,
                    usage: 0.0,
                },
                Some(&Credits {
                    total_credits: 10.0,
                    total_usage: 1.0,
                }),
            ),
            usage_windows(
                &KeyInfo {
                    limit: None,
                    usage: 0.0,
                },
                None,
            ),
        ];
        for windows in cases {
            assert_eq!(windows[0].resets_at, None);
            assert_eq!(windows[0].window_minutes, None);
            assert_eq!(windows[0].kind, WindowKind::Custom);
        }
    }

    #[test]
    fn usage_window_clamps_overage_to_100() {
        let windows = usage_windows(
            &KeyInfo {
                limit: Some(10.0),
                usage: 25.0,
            },
            None,
        );
        assert_eq!(windows[0].used_percent, 100.0);
    }

    #[test]
    fn usage_window_treats_a_non_positive_cap_as_no_cap() {
        // A zero/negative "cap" is not a real limit: fall through to credits, else "No spending limit".
        let from_credits = usage_windows(
            &KeyInfo {
                limit: Some(0.0),
                usage: 5.0,
            },
            Some(&Credits {
                total_credits: 20.0,
                total_usage: 5.0,
            }),
        );
        assert_eq!(from_credits[0].used_percent, 25.0); // 5/20 from credits, not the 0 cap
        assert_eq!(
            from_credits[0].reset_description.as_deref(),
            Some("Credit used")
        );

        let no_limit = usage_windows(
            &KeyInfo {
                limit: Some(-1.0),
                usage: 5.0,
            },
            None,
        );
        assert_eq!(no_limit[0].used_percent, 0.0);
        assert_eq!(
            no_limit[0].reset_description.as_deref(),
            Some("No spending limit")
        );
    }

    #[test]
    fn usage_window_ignores_zero_account_credits() {
        // total_credits = 0 (no purchased balance) is not a usable bound → honest "No spending limit".
        let windows = usage_windows(
            &KeyInfo {
                limit: None,
                usage: 2.0,
            },
            Some(&Credits {
                total_credits: 0.0,
                total_usage: 0.0,
            }),
        );
        assert_eq!(windows[0].used_percent, 0.0);
        assert_eq!(
            windows[0].reset_description.as_deref(),
            Some("No spending limit")
        );
    }

    #[tokio::test]
    async fn fetch_builds_a_credit_snapshot_and_sends_the_bearer_token() {
        let http = Arc::new(ScriptedHttp::new(&[(
            KEY_URL,
            200,
            r#"{"data":{"limit":50.0,"usage":5.0}}"#,
        )]));
        let strat = OpenRouterStrategy {
            secrets: Arc::new(KeySecrets(Some("sk-or-v1-good".into()))),
            http: http.clone(),
            clock: Arc::new(FakeClock(FETCHED_AT)),
        };
        let snap = strat
            .fetch(&ctx())
            .await
            .expect("a connected key fetches usage");
        assert_eq!(snap.provider, ProviderId::new("openrouter"));
        assert_eq!(snap.status, Status::Ok);
        assert_eq!(snap.account, None, "OpenRouter exposes no account identity");
        assert_eq!(snap.windows.len(), 1);
        assert_eq!(snap.windows[0].used_percent, 10.0); // 5/50 from the key cap
        assert_eq!(snap.fetched_at, Timestamp(FETCHED_AT));
        // The first call is a bearer GET to the key endpoint.
        let sent = http.sent.lock().unwrap();
        assert_eq!(sent[0].method, "GET");
        assert_eq!(sent[0].url, KEY_URL);
        assert!(sent[0]
            .headers
            .iter()
            .any(|(k, v)| k == "Authorization" && v == "Bearer sk-or-v1-good"));
        // A key with its own cap settles the window, so /credits is never consulted (no second call).
        assert_eq!(sent.len(), 1, "a capped key must not also call /credits");
    }

    #[tokio::test]
    async fn fetch_uses_account_credits_when_the_key_has_no_cap() {
        let strat = strategy(
            Some("k"),
            ScriptedHttp::new(&[
                (KEY_URL, 200, r#"{"data":{"limit":null,"usage":3.0}}"#),
                (
                    CREDITS_URL,
                    200,
                    r#"{"data":{"total_credits":40.0,"total_usage":10.0}}"#,
                ),
            ]),
        );
        let snap = strat.fetch(&ctx()).await.unwrap();
        assert_eq!(snap.windows[0].used_percent, 25.0); // 10/40 from /credits
        assert_eq!(snap.windows[0].resets_at, None);
    }

    #[tokio::test]
    async fn fetch_tolerates_a_refused_credits_call() {
        // No key cap, so /credits IS consulted — but it answers 403 (management-key-only). The
        // snapshot still succeeds, honestly reporting no spending limit instead of failing.
        let strat = strategy(
            Some("k"),
            ScriptedHttp::new(&[
                (KEY_URL, 200, r#"{"data":{"limit":null,"usage":5.0}}"#),
                (CREDITS_URL, 403, ""),
            ]),
        );
        let snap = strat.fetch(&ctx()).await.unwrap();
        assert_eq!(snap.status, Status::Ok);
        assert_eq!(snap.windows[0].used_percent, 0.0);
        assert_eq!(
            snap.windows[0].reset_description.as_deref(),
            Some("No spending limit")
        );
    }

    #[tokio::test]
    async fn fetch_without_a_stored_key_is_unavailable() {
        let strat = strategy(None, ScriptedHttp::new(&[]));
        assert!(!strat.is_available(&ctx()).await);
        assert!(matches!(
            strat.fetch(&ctx()).await,
            Err(FetchError::Unavailable)
        ));
    }

    #[tokio::test]
    async fn fetch_maps_a_rejected_key_to_an_error() {
        for status in [401u16, 403] {
            let strat = strategy(
                Some("sk-or-v1-bad"),
                ScriptedHttp::new(&[(KEY_URL, status, "")]),
            );
            assert!(matches!(
                strat.fetch(&ctx()).await,
                Err(FetchError::Upstream(_))
            ));
        }
    }

    #[tokio::test]
    async fn fetch_maps_a_rate_limit() {
        let strat = strategy(Some("k"), ScriptedHttp::new(&[(KEY_URL, 429, "")]));
        assert!(matches!(
            strat.fetch(&ctx()).await,
            Err(FetchError::RateLimited)
        ));
    }

    #[tokio::test]
    async fn fetch_maps_an_unexpected_status_to_upstream() {
        // A 5xx on the key call surfaces distinctly (not silently OK), per ADR 0015.
        let strat = strategy(Some("k"), ScriptedHttp::new(&[(KEY_URL, 500, "")]));
        assert!(matches!(
            strat.fetch(&ctx()).await,
            Err(FetchError::Upstream(_))
        ));
    }

    #[tokio::test]
    async fn fetch_surfaces_a_transport_error_on_the_key_call() {
        // An unscripted /key URL fails as a transport error: it must surface as Err, never panic.
        let strat = strategy(Some("k"), ScriptedHttp::new(&[]));
        assert!(strat.fetch(&ctx()).await.is_err());
    }
}
