//! OpenAI API-key validation (task 003) **and** usage reading (task 007).
//!
//! OpenAI is an API-cost provider (ADR 0014): the user pastes a normal `sk-…` key (stored by
//! task 003 in our keychain) and we try to read their billing with it. The catch (PROVIDERS.md):
//! API usage/cost is exposed **only to an organization Admin key** (`sk-admin…`) that individual
//! accounts usually cannot create — a normal key authenticates but is refused at the org-cost
//! endpoint. Tasks 007/008 exist to surface that **honestly** rather than invent a misleading 0%.
//!
//! - **Validation** ([`validate_key`]): prove a pasted key authenticates before we store it, with
//!   the cheapest call OpenAI offers every key — `GET /v1/models` — reading only the status. A
//!   200 (lists models) and a 403 (authenticates but this scope can't list them) both mean a
//!   *valid* key; only a 401 is a real rejection.
//! - **Usage** ([`OpenAiStrategy`]): poll `GET /v1/organization/costs` for the last ~30 days of
//!   spend. On 200 we report the **real USD total as an honest text note** — never a percentage
//!   bar, because these endpoints expose spend with no quota, so a percent would be invented
//!   (ADR 0015). On 403 (the common non-admin key) we report the limitation honestly, as a note,
//!   with no window — which is *not* an error.
//!
//! All HTTP IO is injected via [`HttpPort`] and the key is read via [`SecretStore`], so the
//! parsing/decision logic is pure and unit-tested against fakes — no live account is touched in
//! `cargo test` (a live check is the hand-run `openai_live` example).
use crate::domain::{Status, Timestamp, UsageSnapshot};
use crate::ports::{Clock, HttpPort, HttpRequest, SecretStore};
use crate::sources::api_key_secret_key;
use async_trait::async_trait;
use std::sync::Arc;

use super::{FetchContext, FetchError, FetchKind, FetchStrategy};

/// OpenAI's model-list endpoint. A 200 authenticated GET proves the key works; it is the cheapest
/// call every key can make (used by [`validate_key`]). It carries no usage — billing lives behind
/// the Admin-only org endpoints ([`costs_url`]).
pub const MODELS_URL: &str = "https://api.openai.com/v1/models";

/// The honest, user-facing note shown on the tile when a normal key cannot read org usage (HTTP
/// 403 at [`costs_url`]). Shown verbatim instead of a fabricated 0% bar (tasks 007/008, ADR 0015).
pub const LIMITATION_NOTE: &str = "This key can't read organization usage. OpenAI exposes API usage and cost only to an organization admin key (sk-admin…), which individual accounts usually can't create.";

/// Why a pasted API key could not be accepted. The `Display` text is **user-facing** — it is
/// what the popover shows — so each variant reads as a clear, actionable message (acceptance:
/// an invalid/rejected key shows a clear error and never silently appears connected).
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ApiKeyError {
    /// No key was entered (caught locally, before any network call).
    #[error("Enter an API key.")]
    Empty,
    /// The provider authenticated the request and refused the key (HTTP 401).
    #[error("That key was rejected by OpenAI. Check it and try again.")]
    Rejected,
    /// The verification request itself failed (offline, DNS, TLS …). Not a verdict on the key.
    #[error("Couldn't reach OpenAI to verify the key. Check your connection and try again.")]
    Unreachable,
    /// The endpoint answered with a status we don't treat as a clear accept/reject.
    #[error("OpenAI returned an unexpected response (HTTP {0}).")]
    Unexpected(u16),
}

/// Validate an OpenAI key by authenticating against [`MODELS_URL`]. Returns `Ok(())` when the key
/// is accepted: HTTP 200 (lists models) **or** 403 (the key authenticates but this org/scope
/// can't list them — still a usable key; the org-usage limitation is surfaced honestly later, not
/// at validation). A 401 is a genuine rejection. It never returns or logs the key. A blank key is
/// rejected locally, without a network round-trip, and any transport failure is reported as
/// [`ApiKeyError::Unreachable`] (fail closed — an unverified key is never treated as good).
pub async fn validate_key(http: &dyn HttpPort, key: &str) -> Result<(), ApiKeyError> {
    let key = key.trim();
    if key.is_empty() {
        return Err(ApiKeyError::Empty);
    }
    match http.send(bearer_get(MODELS_URL, key)).await {
        Ok(resp) => match resp.status {
            // 200: the key lists models. 403: it authenticates but this org/scope can't — still a
            // valid key, so we accept it rather than reject a working credential.
            200 | 403 => Ok(()),
            401 => Err(ApiKeyError::Rejected),
            other => Err(ApiKeyError::Unexpected(other)),
        },
        Err(_) => Err(ApiKeyError::Unreachable),
    }
}

// ── Usage reading (task 007) ───────────────────────────────────────────────────

/// OpenAI's organization cost endpoint, built for a rolling ~30-day window ending *now*.
/// `start_time` is in **Unix seconds** (OpenAI uses seconds, unlike Anthropic's RFC3339) — we
/// derive it from the injected clock's millis: `now_millis / 1000 - 30 days`. `bucket_width=1d`
/// with `limit=31` asks for the 31 daily buckets a single request is capped at (PROVIDERS.md).
/// Pure and shared by [`OpenAiStrategy`] and its tests so both target the byte-identical URL.
pub fn costs_url(now: Timestamp) -> String {
    let start_time = now.0 / 1000 - 30 * 86_400;
    format!(
        "https://api.openai.com/v1/organization/costs?start_time={start_time}&bucket_width=1d&limit=31"
    )
}

/// The cost report read from [`costs_url`]: the total USD spent across the window. Lossy
/// (ADR 0015): bad JSON aside, every shape imperfection degrades to `0.0` rather than failing.
#[derive(Debug, Clone, PartialEq)]
pub struct Costs {
    /// Total USD spend summed across every bucket/result in the report window.
    pub total_spend_usd: f64,
}

/// One USD amount, however `/v1/organization/costs` spells it: the real shape is an
/// `{ "value": <number>, "currency": "usd" }` object, but we also accept a bare JSON number or a
/// numeric string. Anything else — absent, null, non-numeric — reads as `0.0` (ADR 0015 lossy
/// decoding): a garbled amount is dropped from the sum, never fatal.
fn amount_usd(amount: &serde_json::Value) -> f64 {
    match amount {
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
        serde_json::Value::String(s) => s.trim().parse::<f64>().unwrap_or(0.0),
        serde_json::Value::Object(_) => amount.get("value").map_or(0.0, amount_usd),
        _ => 0.0,
    }
}

/// Parse `GET /v1/organization/costs`. Bad JSON is fatal (an upstream error); every other
/// imperfection is absorbed (ADR 0015): a missing `data`/`results` array, a result with no
/// amount, or a garbled amount all contribute `0.0` rather than failing the snapshot. The total
/// is the sum of every USD amount across all daily buckets in the window.
pub fn parse_costs(body: &str) -> Result<Costs, FetchError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| FetchError::Upstream(format!("bad json: {e}")))?;
    let total_spend_usd: f64 = value
        .get("data")
        .and_then(|data| data.as_array())
        .into_iter()
        .flatten()
        .filter_map(|bucket| bucket.get("results"))
        .filter_map(|results| results.as_array())
        .flatten()
        .filter_map(|result| result.get("amount"))
        .map(amount_usd)
        .sum();
    Ok(Costs { total_spend_usd })
}

/// The honest spend note for a 200 cost report: the real 30-day USD total as plain text. We show
/// it as a note rather than a percentage bar because these endpoints expose spend with **no
/// quota**, so any percentage would be invented — and we refuse to invent one (ADR 0015).
pub fn spend_note(total_spend_usd: f64) -> String {
    format!("API spend: ${total_spend_usd:.2} over the last 30 days.")
}

/// A bearer-authenticated `GET` of `url` asking for JSON. Shared by the models and cost calls.
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

/// The API-key fetch strategy for OpenAI: read the user-entered key from our keychain (via the
/// [`SecretStore`] port — never the network) and poll the org cost endpoint. There is no OAuth to
/// refresh, so this strategy only ever *reads* the stored key and writes nothing back. The
/// snapshot reports no account identity: OpenAI's cost endpoint exposes none, and we never invent
/// one (ADR 0015 — an all-`None` identity is simply not shown).
pub struct OpenAiStrategy {
    /// Keychain access for the stored API key (read-only here).
    pub secrets: Arc<dyn SecretStore>,
    /// HTTP transport (injected so the core stays IO-free and testable).
    pub http: Arc<dyn HttpPort>,
    /// Clock for the cost window's `start_time` and the snapshot's `fetched_at`.
    pub clock: Arc<dyn Clock>,
}

#[async_trait]
impl FetchStrategy for OpenAiStrategy {
    fn kind(&self) -> FetchKind {
        FetchKind::ApiToken
    }

    async fn is_available(&self, ctx: &FetchContext) -> bool {
        self.api_key(ctx).is_some()
    }

    async fn fetch(&self, ctx: &FetchContext) -> Result<UsageSnapshot, FetchError> {
        let key = self.api_key(ctx).ok_or(FetchError::Unavailable)?;
        let now = self.clock.now();
        let resp = self.http.send(bearer_get(&costs_url(now), &key)).await?;
        let note = match resp.status {
            // 200: a real spend total — shown as an honest note, never a fabricated 0% bar.
            200 => spend_note(parse_costs(&String::from_utf8_lossy(&resp.body))?.total_spend_usd),
            // 403 ≠ 401 here, deliberately: `validate_key` already proved this key authenticates
            // against /v1/models, so a 403 at the org-cost endpoint means "valid key, lacks the
            // org-usage scope" — the common non-admin key. That is an honest limitation we state
            // plainly (Status::Ok, no window, a note), not an error and not a fake 0% window.
            // A 401 (below) means the key was since revoked — a real error.
            403 => LIMITATION_NOTE.to_string(),
            401 => {
                return Err(FetchError::Upstream(
                    "OpenAI rejected the key — reconnect it".into(),
                ))
            }
            429 => return Err(FetchError::RateLimited),
            s => return Err(FetchError::Upstream(format!("HTTP {s}"))),
        };
        Ok(UsageSnapshot {
            provider: ctx.provider.clone(),
            windows: vec![],
            status: Status::Ok,
            fetched_at: now,
            account: None,
            note: Some(note),
        })
    }

    fn should_fallback(&self, err: &FetchError) -> bool {
        matches!(err, FetchError::Unavailable)
    }
}

impl OpenAiStrategy {
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
        assert_eq!(validate_key(&http, "sk-good").await, Ok(()));
        // One authenticated GET to the models endpoint, carrying the bearer token.
        let req = http
            .last
            .lock()
            .unwrap()
            .take()
            .expect("a request was sent");
        assert_eq!(req.method, "GET");
        assert_eq!(req.url, MODELS_URL);
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "Authorization" && v == "Bearer sk-good"));
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "Accept" && v == "application/json"));
    }

    #[tokio::test]
    async fn accepts_a_scoped_key_that_authenticates_but_is_forbidden() {
        // A 403 here means the key authenticates but this org/scope can't list models — still a
        // *valid* key. We accept it (the org-usage limitation is surfaced honestly later, not now)
        // rather than reject a working credential. This is the key difference from a 401.
        let http = FakeHttp::status(403);
        assert_eq!(validate_key(&http, "sk-scoped").await, Ok(()));
    }

    #[tokio::test]
    async fn rejects_a_key_the_endpoint_refuses() {
        let http = FakeHttp::status(401);
        assert_eq!(
            validate_key(&http, "sk-bad").await,
            Err(ApiKeyError::Rejected)
        );
    }

    #[tokio::test]
    async fn surfaces_an_unexpected_status_distinctly() {
        // A rate limit or a server error at validation isn't a verdict on the key — it's reported
        // as its own distinct status, not silently treated as accept or reject.
        for status in [429u16, 500] {
            let http = FakeHttp::status(status);
            assert_eq!(
                validate_key(&http, "k").await,
                Err(ApiKeyError::Unexpected(status))
            );
        }
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
        assert_eq!(validate_key(&http, "  sk-good\n").await, Ok(()));
        let req = http
            .last
            .lock()
            .unwrap()
            .take()
            .expect("a request was sent");
        assert!(req.headers.iter().any(|(_, v)| v == "Bearer sk-good"));
    }

    // ── Usage reading (task 007) ───────────────────────────────────────────────

    /// Maps a request URL to a scripted `(status, body)` and records every request. An
    /// unscripted URL fails as a transport error, so a test exercises the transport-error path
    /// simply by scripting nothing.
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

    /// A cost report mixing every amount shape we tolerate: an `{ "value": … }` object, a bare
    /// JSON number, and a numeric string — across multiple daily buckets. Sums to $12.50.
    const COST_FIXTURE: &str = r#"{
      "object": "page",
      "data": [
        {"object":"bucket","results":[
          {"amount":{"value":1.50,"currency":"usd"}},
          {"amount":{"value":2.50,"currency":"usd"}}
        ]},
        {"object":"bucket","results":[{"amount":3.00}]},
        {"object":"bucket","results":[{"amount":"5.50"}]}
      ],
      "has_more": false
    }"#;

    fn strategy(key: Option<&str>, http: ScriptedHttp) -> OpenAiStrategy {
        OpenAiStrategy {
            secrets: Arc::new(KeySecrets(key.map(String::from))),
            http: Arc::new(http),
            clock: Arc::new(FakeClock(FETCHED_AT)),
        }
    }

    fn ctx() -> FetchContext {
        FetchContext {
            provider: ProviderId::new("openai"),
        }
    }

    #[test]
    fn costs_url_targets_a_30_day_window_in_unix_seconds() {
        // start_time = 1_700_000_000_000 ms / 1000 − 30·86400 s = 1_700_000_000 − 2_592_000.
        assert_eq!(
            costs_url(Timestamp(FETCHED_AT)),
            "https://api.openai.com/v1/organization/costs?start_time=1697408000&bucket_width=1d&limit=31"
        );
    }

    #[test]
    fn parse_costs_sums_every_amount_across_buckets() {
        // 1.50 + 2.50 (object amounts) + 3.00 (bare number) + 5.50 (numeric string) = 12.50.
        let costs = parse_costs(COST_FIXTURE).unwrap();
        assert_eq!(costs.total_spend_usd, 12.50);
    }

    #[test]
    fn parse_costs_is_lossy_for_garbled_or_missing_amounts() {
        // A non-numeric value, a null amount, and a result with no amount at all each contribute
        // 0.0 rather than failing — only the one well-formed amount (7.25) is counted (ADR 0015).
        let lossy = r#"{"data":[{"results":[
            {"amount":{"value":"not-a-number"}},
            {"amount":null},
            {"currency":"usd"},
            {"amount":{"value":7.25,"currency":"usd"}}
        ]}]}"#;
        assert_eq!(parse_costs(lossy).unwrap().total_spend_usd, 7.25);
        // A report with no `data` array is empty spend, not an error.
        assert_eq!(
            parse_costs(r#"{"object":"page"}"#).unwrap().total_spend_usd,
            0.0
        );
    }

    #[test]
    fn parse_costs_rejects_non_json() {
        assert!(parse_costs("not json").is_err());
    }

    #[test]
    fn spend_note_states_the_real_total_to_the_cent() {
        assert_eq!(spend_note(12.5), "API spend: $12.50 over the last 30 days.");
        assert_eq!(spend_note(0.0), "API spend: $0.00 over the last 30 days.");
    }

    #[tokio::test]
    async fn fetch_reports_real_spend_as_an_honest_note_and_sends_the_bearer_token() {
        let url = costs_url(Timestamp(FETCHED_AT));
        let http = Arc::new(ScriptedHttp::new(&[(url.as_str(), 200, COST_FIXTURE)]));
        let strat = OpenAiStrategy {
            secrets: Arc::new(KeySecrets(Some("sk-good".into()))),
            http: http.clone(),
            clock: Arc::new(FakeClock(FETCHED_AT)),
        };
        let snap = strat
            .fetch(&ctx())
            .await
            .expect("a connected key fetches usage");
        assert_eq!(snap.provider, ProviderId::new("openai"));
        assert_eq!(snap.status, Status::Ok);
        assert_eq!(
            snap.account, None,
            "OpenAI's cost endpoint exposes no identity"
        );
        assert!(
            snap.windows.is_empty(),
            "spend is shown as a note, never a fabricated percentage bar"
        );
        assert_eq!(
            snap.note.as_deref(),
            Some("API spend: $12.50 over the last 30 days.")
        );
        assert_eq!(snap.fetched_at, Timestamp(FETCHED_AT));
        // A single bearer GET to the cost endpoint.
        let sent = http.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].method, "GET");
        assert_eq!(sent[0].url, url);
        assert!(sent[0]
            .headers
            .iter()
            .any(|(k, v)| k == "Authorization" && v == "Bearer sk-good"));
    }

    #[tokio::test]
    async fn fetch_reports_the_org_usage_limitation_honestly_on_403() {
        // The common non-admin key: it authenticates (validate_key passed) but can't read org
        // usage. That is NOT an error and NOT a zero window — it's an honest note, Status::Ok.
        let url = costs_url(Timestamp(FETCHED_AT));
        let strat = strategy(
            Some("sk-scoped"),
            ScriptedHttp::new(&[(url.as_str(), 403, "")]),
        );
        let snap = strat
            .fetch(&ctx())
            .await
            .expect("403 is an honest note, not an error");
        assert_eq!(snap.status, Status::Ok);
        assert!(snap.windows.is_empty());
        assert_eq!(snap.note.as_deref(), Some(LIMITATION_NOTE));
    }

    #[tokio::test]
    async fn fetch_maps_a_revoked_key_to_an_error() {
        // A 401 (unlike 403) means the key was since revoked — a real error, not a limitation.
        let url = costs_url(Timestamp(FETCHED_AT));
        let strat = strategy(
            Some("sk-bad"),
            ScriptedHttp::new(&[(url.as_str(), 401, "")]),
        );
        assert!(matches!(
            strat.fetch(&ctx()).await,
            Err(FetchError::Upstream(_))
        ));
    }

    #[tokio::test]
    async fn fetch_maps_a_rate_limit() {
        let url = costs_url(Timestamp(FETCHED_AT));
        let strat = strategy(Some("k"), ScriptedHttp::new(&[(url.as_str(), 429, "")]));
        assert!(matches!(
            strat.fetch(&ctx()).await,
            Err(FetchError::RateLimited)
        ));
    }

    #[tokio::test]
    async fn fetch_maps_an_unexpected_status_to_upstream() {
        // A 5xx surfaces distinctly (not silently OK), per ADR 0015.
        let url = costs_url(Timestamp(FETCHED_AT));
        let strat = strategy(Some("k"), ScriptedHttp::new(&[(url.as_str(), 500, "")]));
        assert!(matches!(
            strat.fetch(&ctx()).await,
            Err(FetchError::Upstream(_))
        ));
    }

    #[tokio::test]
    async fn fetch_surfaces_a_transport_error() {
        // An unscripted URL fails as a transport error: it must surface as Err, never panic.
        let strat = strategy(Some("k"), ScriptedHttp::new(&[]));
        assert!(strat.fetch(&ctx()).await.is_err());
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
}
