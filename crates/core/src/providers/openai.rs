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
//!   spend. On 200 we report the **real USD total as a typed note** — never a percentage bar,
//!   because these endpoints expose spend with no quota, so a percent would be invented
//!   (ADR 0015). The common personal/non-admin key can't read org costs: OpenAI answers it with a
//!   **401** whose body says `insufficient_permissions` / "Missing scopes" — its documented 403 is
//!   geo-restriction only, not a scope refusal. The shared [`cost_provider::cost_note`] reads that
//!   marker and reports the limitation honestly as a typed note with no window — *not* an error
//!   and *not* a rejection (the whole point of task 007).
//!
//! All HTTP IO is injected via [`HttpPort`] and the key is read via [`SecretStore`], so the
//! parsing/decision logic is pure and unit-tested against fakes — no live account is touched in
//! `cargo test` (a live check is the hand-run `openai_live` example).
use crate::domain::{Status, Timestamp, UsageSnapshot};
use crate::ports::{Clock, HttpPort, SecretStore};
use async_trait::async_trait;
use std::sync::Arc;

use super::cost_provider::{self, bearer_get, KeyVerdict};
use super::{FetchContext, FetchError, FetchKind, FetchStrategy};

/// OpenAI's model-list endpoint. A 200 authenticated GET proves the key works; it is the cheapest
/// call every key can make (used by [`validate_key`]). It carries no usage — billing lives behind
/// the Admin-only org endpoints ([`costs_url`]).
pub const MODELS_URL: &str = "https://api.openai.com/v1/models";

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
    match cost_provider::validate_via(http, bearer_get(MODELS_URL, key)).await {
        KeyVerdict::Ok => Ok(()),
        KeyVerdict::Rejected => Err(ApiKeyError::Rejected),
        KeyVerdict::Unreachable => Err(ApiKeyError::Unreachable),
        KeyVerdict::Unexpected(status) => Err(ApiKeyError::Unexpected(status)),
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

/// Parse `GET /v1/organization/costs`. Bad JSON is fatal (an upstream error); every other
/// imperfection is absorbed (ADR 0015): a missing `data`/`results` array, a result with no
/// amount, or a garbled amount all contribute `0.0` rather than failing the snapshot. OpenAI
/// reports dollars, so the shared [`cost_provider::sum_spend`] traversal needs no scaling
/// (`units_per_usd = 1.0`).
pub fn parse_costs(body: &str) -> Result<Costs, FetchError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| FetchError::Upstream(format!("bad json: {e}")))?;
    Ok(Costs {
        total_spend_usd: cost_provider::sum_spend(&value, 1.0),
    })
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
        cost_provider::read_api_key(self.secrets.as_ref(), &ctx.provider).is_some()
    }

    async fn fetch(&self, ctx: &FetchContext) -> Result<UsageSnapshot, FetchError> {
        let key = cost_provider::read_api_key(self.secrets.as_ref(), &ctx.provider)
            .ok_or(FetchError::Unavailable)?;
        let now = self.clock.now();
        let resp = self.http.send(bearer_get(&costs_url(now), &key)).await?;
        let note = cost_provider::cost_note(resp.status, &resp.body, "OpenAI", |body| {
            Ok(parse_costs(body)?.total_spend_usd)
        })?;
        Ok(UsageSnapshot {
            provider: ctx.provider.clone(),
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
    use crate::domain::UsageNote;
    use crate::domain::{ProviderId, Timestamp};
    use crate::providers::cost_provider::test_support::{
        has_header, sent_request, FakeClock, FakeHttp, KeySecrets, ScriptedHttp,
    };
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    #[tokio::test]
    async fn accepts_a_key_the_endpoint_authenticates() {
        let http = FakeHttp::status(200);
        assert_eq!(validate_key(&http, "sk-good").await, Ok(()));
        // One authenticated GET to the models endpoint, carrying the bearer token.
        let req = sent_request(&http);
        assert_eq!(req.method, "GET");
        assert_eq!(req.url, MODELS_URL);
        assert!(has_header(&req, "Authorization", "Bearer sk-good"));
        assert!(has_header(&req, "Accept", "application/json"));
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
        let req = sent_request(&http);
        assert!(has_header(&req, "Authorization", "Bearer sk-good"));
    }

    // ── Usage reading (task 007) ───────────────────────────────────────────────

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
    fn parse_costs_drops_non_finite_string_amounts() {
        // A numeric string like "NaN"/"inf" parses to a non-finite f64; it must be dropped (0.0),
        // never propagated into the total. Only the real 4.00 counts.
        let body = r#"{"data":[{"results":[
            {"amount":{"value":"NaN"}},
            {"amount":{"value":"inf"}},
            {"amount":{"value":"-inf"}},
            {"amount":{"value":4.00,"currency":"usd"}}
        ]}]}"#;
        let costs = parse_costs(body).unwrap();
        assert!(
            costs.total_spend_usd.is_finite(),
            "non-finite amounts must not propagate"
        );
        assert_eq!(costs.total_spend_usd, 4.00);
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
        assert_eq!(snap.fetched_at, Timestamp(FETCHED_AT));
        assert_eq!(snap.note, Some(UsageNote::ApiSpend { usd: 12.50 }));
        // A single bearer GET to the cost endpoint.
        let sent = http.sent.lock();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].method, "GET");
        assert_eq!(sent[0].url, url);
        assert!(has_header(&sent[0], "Authorization", "Bearer sk-good"));
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
        assert_eq!(snap.note, Some(UsageNote::OrgAdminKeyRequired));
    }

    #[tokio::test]
    async fn fetch_reports_the_org_limitation_on_a_401_insufficient_permissions() {
        // OpenAI's REAL non-admin response: the cost endpoint refuses a valid personal key with a
        // 401 `insufficient_permissions` / "Missing scopes" body (its 403 is geo-only). This must
        // read as the honest limitation — Status::Ok, no window, OrgAdminKeyRequired — NOT the
        // "rejected — reconnect it" error a bare 401 yields, or a freshly-connected valid key would
        // immediately look broken (the exact failure task 007 exists to prevent).
        let url = costs_url(Timestamp(FETCHED_AT));
        let body = r#"{"error":{"type":"insufficient_permissions","code":"insufficient_permissions","message":"You have insufficient permissions for this operation. Missing scopes: api.usage.read"}}"#;
        let strat = strategy(
            Some("sk-personal"),
            ScriptedHttp::new(&[(url.as_str(), 401, body)]),
        );
        let snap = strat
            .fetch(&ctx())
            .await
            .expect("an insufficient-permissions 401 is the honest limitation, not an error");
        assert_eq!(snap.status, Status::Ok);
        assert!(snap.windows.is_empty());
        assert_eq!(snap.note, Some(UsageNote::OrgAdminKeyRequired));
    }

    #[tokio::test]
    async fn fetch_maps_a_revoked_key_to_an_error() {
        // A bare 401 (no insufficient-permissions marker) is a since-revoked key — a real error,
        // not a limitation; the marker-bearing non-admin 401 is the honest note tested above.
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
