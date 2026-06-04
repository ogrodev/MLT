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
use crate::domain::{Status, Timestamp, UsageSnapshot};
use crate::ports::{Clock, HttpPort, HttpRequest, SecretStore};
use async_trait::async_trait;
use std::sync::Arc;

use super::{cost_provider, FetchContext, FetchError, FetchKind, FetchStrategy};

/// Anthropic's model-list endpoint. An authenticated GET that answers 200 (or 403) proves the
/// key works without needing org scope, so it is the cheapest call for [`validate_key`].
pub const MODELS_URL: &str = "https://api.anthropic.com/v1/models";

/// The base of Anthropic's org **cost** endpoint, read by [`AnthropicStrategy`]; [`cost_report_url`]
/// appends the query. Reports USD spend (sent in cents) with no quota attached.
const COST_REPORT_BASE: &str = "https://api.anthropic.com/v1/organizations/cost_report";

/// Build the cost-report URL for a rolling ~30-day window ending at `now`. `starting_at` is sent as
/// an RFC 3339 UTC day boundary 30 days back: the docs list it as a (non-optional) query parameter
/// while the published example omits it, so we send it — correct either way, it pins the window
/// explicitly and satisfies the documented contract (avoiding a possible 400 on the admin happy
/// path). `bucket_width=1d` with `limit=31` caps the daily buckets (PROVIDERS.md §"Anthropic API").
/// Docs: https://platform.claude.com/docs/en/api/admin/cost_report.
pub fn cost_report_url(now: Timestamp) -> String {
    const THIRTY_DAYS_MS: i64 = 30 * 86_400_000;
    let starting_at = day_start_rfc3339(Timestamp(now.0 - THIRTY_DAYS_MS));
    format!("{COST_REPORT_BASE}?starting_at={starting_at}&bucket_width=1d&limit=31")
}

/// Format `ts` as RFC 3339 at the start of its UTC day (`YYYY-MM-DDT00:00:00Z`), via Howard
/// Hinnant's `civil_from_days` — pure integer arithmetic, so core needs no clock or calendar crate
/// (purity gate). `div_euclid` floors toward negative infinity, keeping pre-epoch instants sound.
fn day_start_rfc3339(ts: Timestamp) -> String {
    let days = ts.0.div_euclid(86_400_000);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    format!("{year:04}-{month:02}-{day:02}T00:00:00Z")
}

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
    match cost_provider::validate_via(http, anthropic_get(MODELS_URL, key)).await {
        cost_provider::KeyVerdict::Ok => Ok(()),
        cost_provider::KeyVerdict::Rejected => Err(ApiKeyError::Rejected),
        cost_provider::KeyVerdict::Unreachable => Err(ApiKeyError::Unreachable),
        cost_provider::KeyVerdict::Unexpected(status) => Err(ApiKeyError::Unexpected(status)),
    }
}

// ── Usage reading (task 008) ───────────────────────────────────────────────────

/// The org spend read from [`cost_report_url`], summed across every daily bucket. Lossy
/// (ADR 0015): a missing or garbled amount reads as 0.0 rather than failing the whole snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct CostReport {
    /// Total USD spend across the report's buckets (the rolling ~30-day window).
    pub total_spend_usd: f64,
}

/// Parse the org cost report. Bad JSON is fatal (an upstream error); everything below that is
/// lossy — a missing `data`/`results` array, an absent or garbled amount, all read as 0.0, never
/// an error (ADR 0015). Anthropic reports each `amount` in the currency's lowest units — cents —
/// as a decimal string (e.g. "123.45" == $1.2345), so the shared [`cost_provider::sum_spend`]
/// traversal divides by `units_per_usd = 100.0` to convert the cent sum to dollars.
/// Docs: https://platform.claude.com/docs/en/api/admin/cost_report.
pub fn parse_cost_report(body: &str) -> Result<CostReport, FetchError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| FetchError::Upstream(format!("bad json: {e}")))?;
    Ok(CostReport {
        total_spend_usd: cost_provider::sum_spend(&value, 100.0),
    })
}

/// An Anthropic-authenticated `GET` of `url` asking for JSON. Emits the three headers every
/// Anthropic API call needs — `x-api-key`, the dated `anthropic-version`, and `Accept` — shared
/// by validation and the cost read (the [`cost_provider::bearer_get`] sibling, with Anthropic's scheme).
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
        cost_provider::read_api_key(self.secrets.as_ref(), &ctx.provider).is_some()
    }

    async fn fetch(&self, ctx: &FetchContext) -> Result<UsageSnapshot, FetchError> {
        let key = cost_provider::read_api_key(self.secrets.as_ref(), &ctx.provider)
            .ok_or(FetchError::Unavailable)?;
        let now = self.clock.now();
        let resp = self
            .http
            .send(anthropic_get(&cost_report_url(now), &key))
            .await?;
        let note = cost_provider::cost_note(resp.status, &resp.body, "Anthropic", |body| {
            Ok(parse_cost_report(body)?.total_spend_usd)
        })?;
        Ok(UsageSnapshot {
            provider: ctx.provider.clone(),
            // No quota means no honest percentage, so we render no window — the note carries the
            // truth (real spend, or the limitation) instead of a misleading 0% bar (task 008 AC).
            windows: Vec::new(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ProviderId, Timestamp, UsageNote};
    use crate::providers::cost_provider::test_support::{
        has_header, sent_request, FakeClock, FakeHttp, KeySecrets, ScriptedHttp,
    };
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

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

    /// The cost-report URL the strategy builds for the test clock — used to script the route and
    /// assert exactly what hits the wire.
    fn cost_url() -> String {
        cost_report_url(Timestamp(FETCHED_AT))
    }

    #[test]
    fn cost_report_url_sends_a_30_day_starting_at_day_boundary() {
        // starting_at is a UTC day boundary 30 days before `now` (the docs list it as a required
        // query param; the example omits it — sending it is correct either way). 2023-11-14 − 30d
        // = 2023-10-15.
        assert_eq!(
            cost_report_url(Timestamp(FETCHED_AT)),
            "https://api.anthropic.com/v1/organizations/cost_report?starting_at=2023-10-15T00:00:00Z&bucket_width=1d&limit=31"
        );
    }

    // ── parse_cost_report ───────────────────────────────────────────────────────

    #[test]
    fn parse_cost_report_sums_numeric_amounts_across_buckets() {
        // A `data` array of daily buckets, each with a `results` array of cent amounts (Anthropic's
        // lowest-currency-unit strings); the parser sums cents and divides by 100 to get dollars.
        let body = r#"{
            "data": [
                { "results": [ { "amount": 150.0 }, { "amount": 50.0 } ] },
                { "results": [ { "amount": 200.0 } ] }
            ]
        }"#;
        assert_eq!(parse_cost_report(body).unwrap().total_spend_usd, 4.0);
    }

    #[test]
    fn parse_cost_report_tolerates_string_and_object_amounts() {
        // Anthropic emits the amount as a numeric string; the sibling OpenAI report wraps it as
        // `{ "value": .. }`. Both shapes are accepted (ADR 0015), summed as cents, converted to USD.
        let body = r#"{
            "data": [
                { "results": [ { "amount": "150.00" } ] },
                { "results": [ { "amount": { "value": 225.0 } } ] }
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
                { "results": [ { "amount": 300.0 } ] }
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
    fn parse_cost_report_reads_amounts_as_cents_not_dollars() {
        // Anthropic sends `amount` in cents (docs: "123.45" USD == $1.23), so the parser divides
        // by 100. A regression to treating it as dollars silently inflates spend 100x.
        let body = r#"{ "data": [ { "results": [ { "amount": "123.45" } ] } ] }"#;
        let usd = parse_cost_report(body).unwrap().total_spend_usd;
        assert!(
            (usd - 1.2345).abs() < 1e-9,
            "123.45 cents == $1.2345, got {usd}"
        );
    }

    // ── fetch ────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn fetch_reports_real_spend_as_an_honest_note() {
        let body =
            r#"{"data":[{"results":[{"amount":"250.00"}]},{"results":[{"amount":"125.00"}]}]}"#;
        let http = Arc::new(ScriptedHttp::new(&[(&cost_url(), 200, body)]));
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
        assert_eq!(snap.note, Some(UsageNote::ApiSpend { usd: 3.75 }));
        assert_eq!(snap.fetched_at, Timestamp(FETCHED_AT));
        // The single request is a GET to the cost endpoint carrying Anthropic's auth headers.
        let sent = http.sent.lock();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].method, "GET");
        assert_eq!(sent[0].url, cost_url());
        assert!(has_header(&sent[0], "x-api-key", "sk-ant-api-good"));
        assert!(has_header(&sent[0], "anthropic-version", "2023-06-01"));
    }

    #[tokio::test]
    async fn fetch_states_the_limitation_honestly_on_403() {
        // A 403 is the honest limitation — a non-admin key — NOT an error and NOT a 0% window.
        let strat = strategy(Some("k"), ScriptedHttp::new(&[(&cost_url(), 403, "")]));
        let snap = strat.fetch(&ctx()).await.expect("403 is not an error");
        assert_eq!(snap.status, Status::Ok);
        assert!(snap.windows.is_empty(), "no invented zero window");
        assert_eq!(snap.note, Some(UsageNote::OrgAdminKeyRequired));
        assert_eq!(snap.account, None);
    }

    #[tokio::test]
    async fn fetch_maps_a_revoked_key_401_to_an_error() {
        // 401 ≠ 403: the key authenticated at validation but is now refused — a real error.
        let strat = strategy(
            Some("sk-ant-api-bad"),
            ScriptedHttp::new(&[(&cost_url(), 401, "")]),
        );
        assert!(matches!(
            strat.fetch(&ctx()).await,
            Err(FetchError::Upstream(_))
        ));
    }

    #[tokio::test]
    async fn fetch_maps_a_rate_limit() {
        let strat = strategy(Some("k"), ScriptedHttp::new(&[(&cost_url(), 429, "")]));
        assert!(matches!(
            strat.fetch(&ctx()).await,
            Err(FetchError::RateLimited)
        ));
    }

    #[tokio::test]
    async fn fetch_maps_a_server_error_to_upstream() {
        let strat = strategy(Some("k"), ScriptedHttp::new(&[(&cost_url(), 500, "")]));
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
