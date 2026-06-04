//! Anthropic **API** key validation (task 003) **and** usage reading (task 008).
//!
//! This is the Anthropic *API* provider (ADR 0014) — **NOT** Claude Code's subscription. The
//! two are separate, siloed providers: `"claude-code"` tracks a Claude *subscription* via a
//! reused OAuth login, while `"anthropic"` (here) tracks *API* billing via a normal `sk-ant-api…`
//! key the user pastes (stored by task 003 in our keychain). Different id, different display
//! name, different tile, different data — never conflated (PROVIDERS.md §"Two product
//! categories", AGENTS.md siloing).
//!
//! The honesty contract (task 008): Anthropic exposes API usage/cost only to an **org admin key**
//! (`sk-ant-admin…`) that individual accounts usually cannot create, so a normal key returns no
//! usage. We therefore never invent a number — we tell the truth:
//!
//! - **Validation** ([`validate_key`]): prove a pasted key authenticates before we store it, with
//!   the cheapest call Anthropic offers — `GET /v1/models`. 200 accepts it, and so does 403 (a
//!   valid key that merely lacks some scope — validation is about *authentication*, not org
//!   access); only 401 is a real rejection. Status-only — the body is never parsed.
//! - **Usage** ([`AnthropicStrategy`]): poll the org `cost_report` endpoint. A 200 reports the
//!   **real USD spend honestly as a note** — never a fake 0% bar, since these endpoints expose
//!   spend with no quota and a percentage would be invented. A 403 is the **honest limitation**
//!   (a non-admin key), surfaced as a plain note — not an error and not a zero window.
//!
//! All HTTP IO is injected via [`HttpPort`] and the key is read via [`SecretStore`], so the
//! parsing/decision logic is pure and unit-tested against fakes — no live account is touched in
//! `cargo test` (a live check is the hand-run `anthropic_live` example).
use crate::domain::{Status, UsageSnapshot};
use crate::ports::{Clock, HttpPort, HttpRequest, SecretStore};
use crate::sources::api_key_secret_key;
use async_trait::async_trait;
use std::sync::Arc;

use super::{FetchContext, FetchError, FetchKind, FetchStrategy};

/// Anthropic's model-list endpoint. An authenticated GET that answers 200 (or 403) proves the
/// key works without needing org scope, so it is the cheapest call for [`validate_key`].
pub const MODELS_URL: &str = "https://api.anthropic.com/v1/models";

/// Anthropic's org **cost** endpoint, read by [`AnthropicStrategy`]. The URL is static — a `1d`
/// bucket is capped at 31 buckets, so `limit=31` fixes a rolling ~30-day window with no dynamic
/// `start` parameter (PROVIDERS.md §"Anthropic API"). Reports USD spend with no quota attached.
pub const COST_REPORT_URL: &str =
    "https://api.anthropic.com/v1/organizations/cost_report?bucket_width=1d&limit=31";

/// The honest note shown when the key authenticates but cannot read org usage (a 403 at the cost
/// endpoint). Shown verbatim on the tile instead of a misleading number (task 008 AC).
pub const LIMITATION_NOTE: &str = "This key can't read organization usage. Anthropic exposes API usage and cost only to an organization admin key (sk-ant-admin…), which individual accounts usually can't create.";

/// The dated API contract every Anthropic request must declare via the `anthropic-version` header.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Why a pasted API key could not be accepted. The `Display` text is **user-facing** — it is
/// what the popover shows — so each variant reads as a clear, actionable message (acceptance:
/// an invalid/rejected key shows a clear error and never silently appears connected).
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ApiKeyError {
    /// No key was entered (caught locally, before any network call).
    #[error("Enter an API key.")]
    Empty,
    /// The provider authenticated the request and refused the key (HTTP 401).
    #[error("That key was rejected by Anthropic. Check it and try again.")]
    Rejected,
    /// The verification request itself failed (offline, DNS, TLS …). Not a verdict on the key.
    #[error("Couldn't reach Anthropic to verify the key. Check your connection and try again.")]
    Unreachable,
    /// The endpoint answered with a status we don't treat as a clear accept/reject.
    #[error("Anthropic returned an unexpected response (HTTP {0}).")]
    Unexpected(u16),
}

/// Validate an Anthropic API key by authenticating against [`MODELS_URL`]. Returns `Ok(())` when
/// the key is accepted — **200**, and also **403** (a valid key that simply lacks some scope;
/// `/v1/models` needs none, so a 403 still means the key authenticates). **401** is a real
/// rejection; **429** and any other status are reported distinctly via
/// [`ApiKeyError::Unexpected`]. It never returns or logs the key. A blank key is rejected
/// locally, without a network round-trip, and any transport failure is reported as
/// [`ApiKeyError::Unreachable`] (fail closed — an unverified key is never treated as good).
pub async fn validate_key(http: &dyn HttpPort, key: &str) -> Result<(), ApiKeyError> {
    let key = key.trim();
    if key.is_empty() {
        return Err(ApiKeyError::Empty);
    }
    match http.send(anthropic_get(MODELS_URL, key)).await {
        Ok(resp) => match resp.status {
            200 | 403 => Ok(()),
            401 => Err(ApiKeyError::Rejected),
            other => Err(ApiKeyError::Unexpected(other)),
        },
        Err(_) => Err(ApiKeyError::Unreachable),
    }
}

// ── Usage reading (task 008) ───────────────────────────────────────────────────

/// The org spend read from [`COST_REPORT_URL`], summed across every daily bucket. Lossy
/// (ADR 0015): a missing or garbled amount reads as 0.0 rather than failing the whole snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct CostReport {
    /// Total USD spend across the report's buckets (the rolling ~30-day window).
    pub total_spend_usd: f64,
}

/// Coerce one JSON amount to USD, tolerating the shapes the cost report can emit: a bare number,
/// a numeric string (`"12.34"`, as Anthropic sends), or an `{ "value": <number> }` wrapper (as
/// the sibling OpenAI cost report sends). Anything else — null, a non-numeric string, an odd
/// object — reads as 0.0, and a non-finite result is flattened to 0.0 so it can never poison the
/// sum (ADR 0015 lossy decoding).
fn coerce_amount(value: &serde_json::Value) -> f64 {
    let amount = match value {
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
        serde_json::Value::String(s) => s.trim().parse::<f64>().unwrap_or(0.0),
        serde_json::Value::Object(_) => value.get("value").map(coerce_amount).unwrap_or(0.0),
        _ => 0.0,
    };
    if amount.is_finite() {
        amount
    } else {
        0.0
    }
}

/// One result's USD amount: prefer its `amount` field (the cost report's shape), else coerce the
/// result node itself — so the parser stays robust whether the amount is nested or bare.
fn result_amount(result: &serde_json::Value) -> f64 {
    coerce_amount(result.get("amount").unwrap_or(result))
}

/// Parse the org cost report. Bad JSON is fatal (an upstream error); everything below that is
/// lossy — a missing `data`/`results` array, an absent or garbled amount, all read as 0.0, never
/// an error (ADR 0015). The report nests daily buckets under `data`, each with a `results` array
/// of USD amounts; we sum every amount across them.
pub fn parse_cost_report(body: &str) -> Result<CostReport, FetchError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| FetchError::Upstream(format!("bad json: {e}")))?;
    let total_spend_usd: f64 = value
        .get("data")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|bucket| bucket.get("results").and_then(serde_json::Value::as_array))
        .flatten()
        .map(result_amount)
        .sum();
    Ok(CostReport { total_spend_usd })
}

/// The honest, user-facing note for a successful cost read: the **real** USD spend as text, never
/// a percentage bar — these endpoints expose spend with no quota, so a percent would be invented,
/// and we refuse to invent one (task 008 AC). Pure formatting, unit-tested apart from any IO.
pub fn spend_note(total_spend_usd: f64) -> String {
    format!("API spend: ${total_spend_usd:.2} over the last 30 days.")
}

/// An Anthropic-authenticated `GET` of `url` asking for JSON. Emits the three headers every
/// Anthropic API call needs — `x-api-key`, the dated `anthropic-version`, and `Accept` — shared
/// by validation and the cost read (mirrors OpenRouter's `bearer_get`, with Anthropic's scheme).
fn anthropic_get(url: &str, key: &str) -> HttpRequest {
    HttpRequest {
        method: "GET".into(),
        url: url.into(),
        headers: vec![
            ("x-api-key".into(), key.into()),
            ("anthropic-version".into(), ANTHROPIC_VERSION.into()),
            ("Accept".into(), "application/json".into()),
        ],
        body: None,
    }
}

/// The API-key fetch strategy for the Anthropic **API** provider: read the user-entered key from
/// our keychain (via the [`SecretStore`] port — never the network) and poll the org cost endpoint.
/// There is no OAuth to refresh, so this strategy only ever *reads* the stored key and writes
/// nothing back. The snapshot reports no account identity: there is no identity endpoint here and
/// we never invent one (ADR 0015 — an all-`None` identity is simply not shown).
pub struct AnthropicStrategy {
    /// Our own keychain, where task 003 stored the user-entered API key. Read-only here.
    pub secrets: Arc<dyn SecretStore>,
    /// The injected HTTP client — the only way this pure-logic strategy reaches the network.
    pub http: Arc<dyn HttpPort>,
    /// The injected clock; stamps `fetched_at` without core ever calling `SystemTime::now()`.
    pub clock: Arc<dyn Clock>,
}

#[async_trait]
impl FetchStrategy for AnthropicStrategy {
    fn kind(&self) -> FetchKind {
        FetchKind::ApiToken
    }

    async fn is_available(&self, ctx: &FetchContext) -> bool {
        self.api_key(ctx).is_some()
    }

    async fn fetch(&self, ctx: &FetchContext) -> Result<UsageSnapshot, FetchError> {
        let key = self.api_key(ctx).ok_or(FetchError::Unavailable)?;
        let resp = self.http.send(anthropic_get(COST_REPORT_URL, &key)).await?;
        let note = match resp.status {
            200 => {
                let report = parse_cost_report(&String::from_utf8_lossy(&resp.body))?;
                spend_note(report.total_spend_usd)
            }
            // 403 is the honest limitation, NOT an error — and that is exactly why it diverges
            // from 401. `validate_key` already proved this key authenticates against `/v1/models`,
            // so a 403 *here* means "valid key, lacks the org-usage scope" — i.e. a normal,
            // non-admin key. A 401 below is different: the key was since revoked, a real error.
            403 => LIMITATION_NOTE.to_string(),
            401 => {
                return Err(FetchError::Upstream(
                    "Anthropic rejected the key — reconnect it".into(),
                ))
            }
            429 => return Err(FetchError::RateLimited),
            s => return Err(FetchError::Upstream(format!("HTTP {s}"))),
        };
        Ok(UsageSnapshot {
            provider: ctx.provider.clone(),
            // No quota means no honest percentage, so we render no window — the note carries the
            // truth (real spend, or the limitation) instead of a misleading 0% bar (task 008 AC).
            windows: Vec::new(),
            status: Status::Ok,
            fetched_at: self.clock.now(),
            account: None,
            note: Some(note),
        })
    }

    fn should_fallback(&self, err: &FetchError) -> bool {
        matches!(err, FetchError::Unavailable)
    }
}

impl AnthropicStrategy {
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
    use parking_lot::Mutex;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

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
            *self.last.lock() = Some(req);
            match self.outcome {
                Ok(status) => Ok(HttpResponse {
                    status,
                    body: Vec::new(),
                }),
                Err(()) => Err(PortError::Io("offline".into())),
            }
        }
    }

    /// Takes the captured request out of a [`FakeHttp`], asserting one was actually sent.
    fn sent_request(http: &FakeHttp) -> HttpRequest {
        http.last.lock().take().expect("a request was sent")
    }

    /// Whether a request carries a header with exactly this name and value.
    fn has_header(req: &HttpRequest, name: &str, value: &str) -> bool {
        req.headers.iter().any(|(k, v)| k == name && v == value)
    }

    #[tokio::test]
    async fn accepts_a_key_the_endpoint_authenticates() {
        let http = FakeHttp::status(200);
        assert_eq!(validate_key(&http, "sk-ant-api-good").await, Ok(()));
        // One authenticated GET to the models endpoint, carrying Anthropic's auth headers.
        let req = sent_request(&http);
        assert_eq!(req.method, "GET");
        assert_eq!(req.url, MODELS_URL);
        assert!(has_header(&req, "x-api-key", "sk-ant-api-good"));
        assert!(has_header(&req, "anthropic-version", "2023-06-01"));
    }

    #[tokio::test]
    async fn accepts_a_valid_but_scoped_key_on_403() {
        // 403 means the key authenticates but lacks some scope — still a usable key, so validation
        // accepts it. The honest-limitation surfaces later, at the usage read, not here.
        let http = FakeHttp::status(403);
        assert_eq!(validate_key(&http, "sk-ant-api-scoped").await, Ok(()));
    }

    #[tokio::test]
    async fn rejects_a_key_the_endpoint_refuses() {
        let http = FakeHttp::status(401);
        assert_eq!(
            validate_key(&http, "sk-ant-api-bad").await,
            Err(ApiKeyError::Rejected)
        );
    }

    #[tokio::test]
    async fn surfaces_an_unexpected_status_distinctly() {
        // Neither a clear accept nor a clear reject: a rate-limit or a server error is reported
        // verbatim rather than guessed at.
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
        assert_eq!(validate_key(&http, "  sk-ant-api-good\n").await, Ok(()));
        let req = sent_request(&http);
        assert!(has_header(&req, "x-api-key", "sk-ant-api-good"));
    }

    // ── Usage reading (task 008) ───────────────────────────────────────────────

    /// Maps a request URL to a scripted `(status, body)` and records every request. An
    /// unscripted URL fails as a transport error, exercising the strategy's error path.
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
            self.sent.lock().push(req);
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

    fn strategy(key: Option<&str>, http: ScriptedHttp) -> AnthropicStrategy {
        AnthropicStrategy {
            secrets: Arc::new(KeySecrets(key.map(String::from))),
            http: Arc::new(http),
            clock: Arc::new(FakeClock(FETCHED_AT)),
        }
    }

    fn ctx() -> FetchContext {
        FetchContext {
            provider: ProviderId::new("anthropic"),
        }
    }

    // ── parse_cost_report / spend_note ──────────────────────────────────────────

    #[test]
    fn parse_cost_report_sums_numeric_amounts_across_buckets() {
        // A `data` array of daily buckets, each with a `results` array of USD amounts.
        let body = r#"{
            "data": [
                { "results": [ { "amount": 1.5 }, { "amount": 0.5 } ] },
                { "results": [ { "amount": 2.0 } ] }
            ]
        }"#;
        assert_eq!(parse_cost_report(body).unwrap().total_spend_usd, 4.0);
    }

    #[test]
    fn parse_cost_report_tolerates_string_and_object_amounts() {
        // Anthropic emits the amount as a numeric string; the sibling OpenAI report wraps it as
        // `{ "value": .. }`. Both coerce to the same number (ADR 0015 lossy decoding).
        let body = r#"{
            "data": [
                { "results": [ { "amount": "1.50" } ] },
                { "results": [ { "amount": { "value": 2.25 } } ] }
            ]
        }"#;
        assert_eq!(parse_cost_report(body).unwrap().total_spend_usd, 3.75);
    }

    #[test]
    fn parse_cost_report_reads_missing_or_garbled_amounts_as_zero() {
        // A missing/garbled amount contributes 0.0, never an error — and a bucket with no
        // `results` (or an empty report) simply adds nothing.
        let body = r#"{
            "data": [
                { "results": [ { "amount": "not-a-number" }, { } ] },
                { "results": [ { "amount": null } ] },
                { },
                { "results": [ { "amount": 3.0 } ] }
            ]
        }"#;
        assert_eq!(parse_cost_report(body).unwrap().total_spend_usd, 3.0);
        // A report with no data at all is a clean zero, not an error.
        assert_eq!(parse_cost_report("{}").unwrap().total_spend_usd, 0.0);
    }

    #[test]
    fn parse_cost_report_rejects_non_json() {
        assert!(parse_cost_report("not json").is_err());
    }

    #[test]
    fn spend_note_formats_two_decimals_of_usd() {
        assert_eq!(spend_note(12.5), "API spend: $12.50 over the last 30 days.");
        assert_eq!(spend_note(0.0), "API spend: $0.00 over the last 30 days.");
    }

    // ── fetch ────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn fetch_reports_real_spend_as_an_honest_note() {
        let body = r#"{"data":[{"results":[{"amount":"2.50"}]},{"results":[{"amount":"1.25"}]}]}"#;
        let http = Arc::new(ScriptedHttp::new(&[(COST_REPORT_URL, 200, body)]));
        let strat = AnthropicStrategy {
            secrets: Arc::new(KeySecrets(Some("sk-ant-api-good".into()))),
            http: http.clone(),
            clock: Arc::new(FakeClock(FETCHED_AT)),
        };
        let snap = strat
            .fetch(&ctx())
            .await
            .expect("a connected key fetches usage");
        assert_eq!(snap.provider, ProviderId::new("anthropic"));
        assert_eq!(snap.status, Status::Ok);
        assert_eq!(
            snap.account, None,
            "the Anthropic API exposes no account identity"
        );
        assert!(
            snap.windows.is_empty(),
            "spend has no quota, so no percentage bar"
        );
        assert_eq!(
            snap.note.as_deref(),
            Some("API spend: $3.75 over the last 30 days.")
        );
        assert_eq!(snap.fetched_at, Timestamp(FETCHED_AT));
        // The single request is a GET to the cost endpoint carrying Anthropic's auth headers.
        let sent = http.sent.lock();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].method, "GET");
        assert_eq!(sent[0].url, COST_REPORT_URL);
        assert!(has_header(&sent[0], "x-api-key", "sk-ant-api-good"));
        assert!(has_header(&sent[0], "anthropic-version", "2023-06-01"));
    }

    #[tokio::test]
    async fn fetch_states_the_limitation_honestly_on_403() {
        // A 403 is the honest limitation — a non-admin key — NOT an error and NOT a 0% window.
        let strat = strategy(Some("k"), ScriptedHttp::new(&[(COST_REPORT_URL, 403, "")]));
        let snap = strat.fetch(&ctx()).await.expect("403 is not an error");
        assert_eq!(snap.status, Status::Ok);
        assert!(snap.windows.is_empty(), "no invented zero window");
        assert_eq!(snap.note.as_deref(), Some(LIMITATION_NOTE));
        assert_eq!(snap.account, None);
    }

    #[tokio::test]
    async fn fetch_maps_a_revoked_key_401_to_an_error() {
        // 401 ≠ 403: the key authenticated at validation but is now refused — a real error.
        let strat = strategy(
            Some("sk-ant-api-bad"),
            ScriptedHttp::new(&[(COST_REPORT_URL, 401, "")]),
        );
        assert!(matches!(
            strat.fetch(&ctx()).await,
            Err(FetchError::Upstream(_))
        ));
    }

    #[tokio::test]
    async fn fetch_maps_a_rate_limit() {
        let strat = strategy(Some("k"), ScriptedHttp::new(&[(COST_REPORT_URL, 429, "")]));
        assert!(matches!(
            strat.fetch(&ctx()).await,
            Err(FetchError::RateLimited)
        ));
    }

    #[tokio::test]
    async fn fetch_maps_a_server_error_to_upstream() {
        let strat = strategy(Some("k"), ScriptedHttp::new(&[(COST_REPORT_URL, 500, "")]));
        assert!(matches!(
            strat.fetch(&ctx()).await,
            Err(FetchError::Upstream(_))
        ));
    }

    #[tokio::test]
    async fn fetch_surfaces_a_transport_error() {
        // An unscripted cost URL fails as a transport error: it must surface as Err, never panic.
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
