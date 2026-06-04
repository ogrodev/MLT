//! The *invariant* policy shared by the API-cost providers (OpenAI task 007, Anthropic task 008).
//!
//! Both are API-cost sources (ADR 0014): the user pastes a normal key, we authenticate it against
//! a cheap models endpoint, then read 30-day spend from a cost endpoint that exposes a USD total
//! with **no quota** (PROVIDERS.md). Nearly every *decision* is identical across the two — what an
//! HTTP status means for key validation and for a cost read, and how the stored key is read from
//! our keychain — so they live here, pure and unit-tested against fakes. The lone cost-read
//! decision that is *not* shared — which HTTP signal means "needs an org admin key" — is passed in
//! per provider as [`OrgLimitation`] (Anthropic: a 403; OpenAI: a 401 + body marker, its 403 being
//! a geo-restriction). Everything else *provider-specific* stays in each provider module: the
//! endpoint URLs, the auth header shape, the request bodies, the spend JSON parser and its unit
//! scale, and the user-facing `Display` wording each maps these verdicts to.
use super::FetchError;
use crate::domain::{ProviderId, UsageNote};
use crate::ports::{HttpPort, HttpRequest, SecretStore};
use crate::sources::api_key_secret_key;

/// Verdict of authenticating a pasted key against a provider's models endpoint. Each provider
/// maps this to its own `ApiKeyError` (provider-specific Display wording stays in the provider).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyVerdict {
    Ok,
    Rejected,
    Unreachable,
    Unexpected(u16),
}

/// Send a prebuilt validation request and classify the outcome (status-only; the body is never
/// read): 200|403 accept (a 403 still proves the key authenticates), 401 reject, any other
/// status reported distinctly, a transport error is `Unreachable` (fail closed).
pub async fn validate_via(http: &dyn HttpPort, request: HttpRequest) -> KeyVerdict {
    match http.send(request).await {
        Ok(response) => match response.status {
            200 | 403 => KeyVerdict::Ok,
            401 => KeyVerdict::Rejected,
            other => KeyVerdict::Unexpected(other),
        },
        Err(_) => KeyVerdict::Unreachable,
    }
}

/// How a provider signals the honest "needs an org admin key" limitation on its **cost** endpoint
/// — the lone cost-read decision that is *not* shared, because the two providers encode it
/// differently. Anthropic returns a dedicated **403**; OpenAI overloads **401** with an
/// `insufficient_permissions` / "Missing scopes" body marker and reserves **403** for a
/// geo-restriction (an upstream error, never the limitation). Passed to [`cost_note`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OrgLimitation {
    /// The limitation is HTTP **403** (Anthropic); a 401 is always a genuine rejection.
    Forbidden,
    /// The limitation is HTTP **401** carrying the insufficient-org-scope body marker (OpenAI). A
    /// 401 with any other body is a genuine rejection, and a 403 is a different upstream failure
    /// (OpenAI documents it as a geo-restriction) — never the limitation.
    UnauthorizedWithScopeMarker,
}

impl OrgLimitation {
    /// True when `(status, body)` is this provider's honest org-admin-key limitation.
    fn matches(self, status: u16, body: &[u8]) -> bool {
        match self {
            Self::Forbidden => status == 403,
            Self::UnauthorizedWithScopeMarker => status == 401 && is_insufficient_org_scope(body),
        }
    }
}

/// Read the user-entered API key from our keychain for this source — trimmed, non-empty — or
/// `None` when not connected. Reads via the `SecretStore` port; never the network.
pub fn read_api_key(secrets: &dyn SecretStore, provider: &ProviderId) -> Option<String> {
    secrets
        .get(&api_key_secret_key(provider))
        .ok()
        .flatten()
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
}

/// A bearer-authenticated `GET` of `url` asking for JSON — the request shape shared by the
/// bearer-token cost providers (OpenAI, OpenRouter). Anthropic uses its own `x-api-key` scheme and
/// builds its requests in its own module.
pub(crate) fn bearer_get(url: &str, key: &str) -> HttpRequest {
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

/// Coerce one JSON amount — a number, a numeric string (`"12.34"`, as Anthropic sends), or an
/// `{ "value": <number> }` wrapper (as OpenAI sends) — to an `f64`. Anything else (null, a
/// non-numeric string, an odd object) reads as 0.0, and a non-finite result (`"NaN"`, `"inf"`) is
/// flattened to 0.0 so it can never poison the sum (ADR 0015 lossy decoding).
pub(crate) fn coerce_amount(value: &serde_json::Value) -> f64 {
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

/// Sum every amount in a cost report's `data → results → amount` tree and convert to USD dollars.
/// Both API-cost reports share this exact shape; only the wire's unit differs, so the caller passes
/// `units_per_usd` — the count of the wire's smallest units in one dollar (OpenAI reports dollars
/// → `1.0`; Anthropic reports cents → `100.0`). Keeping the unit a caller-supplied divisor — never
/// folded into [`coerce_amount`] — is deliberate: hardcoding Anthropic's `/100` here would silently
/// inflate OpenAI spend 100×. Lossy throughout (ADR 0015): a missing array, or an absent/garbled
/// amount, each contribute 0.0 rather than failing.
pub(crate) fn sum_spend(report: &serde_json::Value, units_per_usd: f64) -> f64 {
    let total: f64 = report
        .get("data")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|bucket| bucket.get("results").and_then(serde_json::Value::as_array))
        .flatten()
        // Prefer the result's `amount` field (the cost-report shape); fall back to coercing the
        // result node itself so a bare amount is still summed (ADR 0015 lossy decoding).
        .map(|result| coerce_amount(result.get("amount").unwrap_or(result)))
        .sum();
    total / units_per_usd
}

/// Map a cost-endpoint response to a typed [`UsageNote`] (or a [`FetchError`]). The status policy
/// is shared; the one provider-specific decision — which HTTP signal means the honest "needs an org
/// admin key" limitation — is passed in as [`OrgLimitation`]. The body parser is provider-specific
/// too (each cost report has its own struct and unit scale), so it is injected as `parse_usd`;
/// `provider_display` names the provider in the revoked-key message.
///
/// Decisions, in order:
/// - **200** → parse the body into a real-spend note.
/// - the provider's org-admin-key limitation ([`OrgLimitation`]) → [`UsageNote::OrgAdminKeyRequired`]:
///   never an error or a fabricated zero, so a valid non-admin key (the common case, tasks 007/008)
///   never falsely reads "reconnect it". Anthropic signals it with a 403; OpenAI with a 401 whose
///   body carries the insufficient-org-scope marker.
/// - **401** that is *not* that limitation → a revoked-key error naming the provider.
/// - **429** → [`FetchError::RateLimited`].
/// - any other status (including OpenAI's geo-restricted **403**) → an upstream error.
pub(crate) fn cost_note<F>(
    status: u16,
    body: &[u8],
    provider_display: &str,
    org_limitation: OrgLimitation,
    parse_usd: F,
) -> Result<UsageNote, FetchError>
where
    F: FnOnce(&str) -> Result<f64, FetchError>,
{
    if status == 200 {
        return Ok(UsageNote::ApiSpend {
            usd: parse_usd(&String::from_utf8_lossy(body))?,
        });
    }
    if org_limitation.matches(status, body) {
        return Ok(UsageNote::OrgAdminKeyRequired);
    }
    match status {
        401 => Err(FetchError::Upstream(format!(
            "{provider_display} rejected the key — reconnect it"
        ))),
        429 => Err(FetchError::RateLimited),
        other => Err(FetchError::Upstream(format!("HTTP {other}"))),
    }
}

/// True when an error body carries the marker a provider uses for an *authenticated* key that
/// simply lacks the org-usage scope — `insufficient_permissions` / "Missing scopes" (OpenAI's
/// `type`/`code` token and message wording). OpenAI overloads HTTP 401 for this case (Anthropic
/// uses a distinct 403), so the body — not the status alone — is what tells an org-scope
/// limitation apart from a genuinely revoked key, which carries a different marker
/// (`invalid_api_key`). Matched case- and underscore-insensitively so both the `type`/`code`
/// token and the prose message trip it. Lossy decode (ADR 0015): a non-UTF8 body simply misses.
fn is_insufficient_org_scope(body: &[u8]) -> bool {
    let text = String::from_utf8_lossy(body)
        .to_ascii_lowercase()
        .replace('_', " ");
    text.contains("insufficient permission") || text.contains("missing scope")
}

/// In-memory port fakes shared by the OpenAI and Anthropic provider tests, so both exercise the
/// byte-identical request shapes against one set of fixtures. Test-only.
#[cfg(test)]
pub(crate) mod test_support {
    use crate::domain::Timestamp;
    use crate::ports::{Clock, HttpPort, HttpRequest, HttpResponse, PortError, SecretStore};
    use async_trait::async_trait;
    use parking_lot::Mutex;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Scripts one HTTP outcome, counts how many requests were sent, and captures the last
    /// request so a test can assert exactly what hit the wire.
    pub(crate) struct FakeHttp {
        outcome: Result<u16, ()>,
        pub(crate) calls: AtomicUsize,
        last: Mutex<Option<HttpRequest>>,
    }
    impl FakeHttp {
        /// A fake that answers every request with `status`.
        pub(crate) fn status(status: u16) -> Self {
            Self {
                outcome: Ok(status),
                calls: AtomicUsize::new(0),
                last: Mutex::new(None),
            }
        }
        /// A fake whose every request fails as a transport error.
        pub(crate) fn transport_error() -> Self {
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

    /// Maps a request URL to a scripted `(status, body)` and records every request. An unscripted
    /// URL fails as a transport error, so a test exercises that path simply by scripting nothing.
    pub(crate) struct ScriptedHttp {
        routes: HashMap<String, (u16, String)>,
        pub(crate) sent: Mutex<Vec<HttpRequest>>,
    }
    impl ScriptedHttp {
        /// Build a fake from `(url, status, body)` routes matched on the exact request URL.
        pub(crate) fn new(routes: &[(&str, u16, &str)]) -> Self {
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
    pub(crate) struct KeySecrets(pub Option<String>);
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

    /// A clock frozen at a fixed epoch-millis instant.
    pub(crate) struct FakeClock(pub i64);
    impl Clock for FakeClock {
        fn now(&self) -> Timestamp {
            Timestamp(self.0)
        }
    }

    /// True when `req` carries a header whose name and value both match exactly.
    pub(crate) fn has_header(req: &HttpRequest, name: &str, value: &str) -> bool {
        req.headers.iter().any(|(k, v)| k == name && v == value)
    }

    /// Take the single request a [`FakeHttp`] recorded, panicking if none was sent.
    pub(crate) fn sent_request(http: &FakeHttp) -> HttpRequest {
        http.last.lock().take().expect("a request was sent")
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{has_header, sent_request, FakeHttp, KeySecrets};
    use super::*;
    use std::sync::atomic::Ordering;

    /// A minimal bearer GET, standing in for a provider's models-endpoint validation request.
    fn validation_request() -> HttpRequest {
        HttpRequest {
            method: "GET".to_string(),
            url: "https://provider.example/v1/models".to_string(),
            headers: vec![("Authorization".to_string(), "Bearer sk-test".to_string())],
            body: None,
        }
    }

    #[tokio::test]
    async fn validate_via_accepts_200_and_403_forwarding_the_request_unchanged() {
        // 200 lists models; 403 authenticates but forbids the scope — both prove the key is valid.
        for status in [200u16, 403] {
            let http = FakeHttp::status(status);
            assert_eq!(
                validate_via(&http, validation_request()).await,
                KeyVerdict::Ok,
                "status {status} must accept the key"
            );
            // The prebuilt request is sent verbatim, exactly once.
            assert_eq!(http.calls.load(Ordering::SeqCst), 1);
            let sent = sent_request(&http);
            assert_eq!(sent.method, "GET");
            assert_eq!(sent.url, "https://provider.example/v1/models");
            assert!(has_header(&sent, "Authorization", "Bearer sk-test"));
        }
    }

    #[tokio::test]
    async fn validate_via_rejects_a_401() {
        let http = FakeHttp::status(401);
        assert_eq!(
            validate_via(&http, validation_request()).await,
            KeyVerdict::Rejected
        );
    }

    #[tokio::test]
    async fn validate_via_reports_an_unexpected_status_distinctly() {
        // A 500 is neither accept nor reject — it surfaces as its own verdict, never guessed.
        let http = FakeHttp::status(500);
        assert_eq!(
            validate_via(&http, validation_request()).await,
            KeyVerdict::Unexpected(500)
        );
    }

    #[tokio::test]
    async fn validate_via_fails_closed_on_a_transport_error() {
        // We couldn't verify, so the key is never treated as good.
        let http = FakeHttp::transport_error();
        assert_eq!(
            validate_via(&http, validation_request()).await,
            KeyVerdict::Unreachable
        );
    }

    #[test]
    fn read_api_key_returns_the_trimmed_stored_key() {
        let secrets = KeySecrets(Some("  sk-live\n".to_string()));
        assert_eq!(
            read_api_key(&secrets, &ProviderId::new("openai")).as_deref(),
            Some("sk-live")
        );
    }

    #[test]
    fn read_api_key_is_none_when_no_key_is_stored() {
        let secrets = KeySecrets(None);
        assert_eq!(read_api_key(&secrets, &ProviderId::new("openai")), None);
    }

    #[test]
    fn read_api_key_treats_a_whitespace_only_key_as_absent() {
        let secrets = KeySecrets(Some("   ".to_string()));
        assert_eq!(read_api_key(&secrets, &ProviderId::new("anthropic")), None);
    }

    #[test]
    fn bearer_get_builds_an_authenticated_json_get() {
        let req = bearer_get("https://api.example/v1/costs", "sk-live");
        assert_eq!(req.method, "GET");
        assert_eq!(req.url, "https://api.example/v1/costs");
        assert!(has_header(&req, "Authorization", "Bearer sk-live"));
        assert!(has_header(&req, "Accept", "application/json"));
        assert!(req.body.is_none());
    }

    #[test]
    fn coerce_amount_reads_numbers_strings_and_value_objects() {
        assert_eq!(coerce_amount(&serde_json::json!(12.5)), 12.5);
        assert_eq!(coerce_amount(&serde_json::json!("3.25")), 3.25);
        assert_eq!(
            coerce_amount(&serde_json::json!({ "value": 4.0, "currency": "usd" })),
            4.0
        );
    }

    #[test]
    fn coerce_amount_drops_garbled_and_non_finite_amounts_to_zero() {
        assert_eq!(coerce_amount(&serde_json::json!(null)), 0.0);
        assert_eq!(coerce_amount(&serde_json::json!("not-a-number")), 0.0);
        assert_eq!(coerce_amount(&serde_json::json!("NaN")), 0.0);
        assert_eq!(coerce_amount(&serde_json::json!("inf")), 0.0);
        assert_eq!(
            coerce_amount(&serde_json::json!({ "currency": "usd" })),
            0.0
        );
    }

    #[test]
    fn sum_spend_divides_by_units_per_usd_so_cents_are_not_dollars() {
        // The 100x-trap guard: the same report read as dollars (1.0) vs cents (100.0) must differ
        // by exactly 100x. Folding the unit into the coercer would silently inflate a dollars
        // provider's spend 100x.
        let report = serde_json::json!({
            "data": [
                { "results": [ { "amount": "150.00" }, { "amount": 50.0 } ] },
                { "results": [ { "amount": { "value": 200.0 } } ] }
            ]
        });
        assert_eq!(sum_spend(&report, 1.0), 400.0); // OpenAI reports dollars.
        assert_eq!(sum_spend(&report, 100.0), 4.0); // Anthropic reports cents.
    }

    #[test]
    fn sum_spend_is_lossy_for_missing_or_garbled_shapes() {
        // No data array, an amountless result, and a garbled amount each contribute 0.0, never an
        // error — only the one well-formed 7.25 is counted (ADR 0015).
        assert_eq!(sum_spend(&serde_json::json!({}), 1.0), 0.0);
        let report = serde_json::json!({
            "data": [
                { "results": [ { "amount": "x" }, {}, { "amount": 7.25 } ] },
                {}
            ]
        });
        assert_eq!(sum_spend(&report, 1.0), 7.25);
    }

    #[test]
    fn cost_note_maps_a_200_to_a_parsed_spend_note() {
        let note = cost_note(200, b"42.0", "OpenAI", OrgLimitation::Forbidden, |b| {
            Ok(b.trim().parse::<f64>().unwrap_or(0.0))
        })
        .expect("200 parses to a note");
        assert_eq!(note, UsageNote::ApiSpend { usd: 42.0 });
    }

    #[test]
    fn cost_note_maps_an_anthropic_403_to_the_honest_limitation_without_parsing() {
        // Anthropic signals the org-admin-key limitation with a 403 — an honest note, never an
        // error, and the spend parser must never run on it.
        let note = cost_note(
            403,
            b"ignored",
            "Anthropic",
            OrgLimitation::Forbidden,
            |_| -> Result<f64, FetchError> { unreachable!("403 must never parse the body") },
        )
        .expect("an Anthropic 403 is the honest limitation, not an error");
        assert_eq!(note, UsageNote::OrgAdminKeyRequired);
    }

    #[test]
    fn cost_note_maps_an_openai_403_to_an_error_not_the_limitation() {
        // OpenAI reserves 403 for a geo-restriction; its limitation is a 401 + body marker. A 403
        // here is a genuine upstream error, NOT the org-admin-key note — a regression would make a
        // geo-blocked OpenAI key look benignly "needs an admin key" with Status::Ok.
        let err = cost_note(
            403,
            b"ignored",
            "OpenAI",
            OrgLimitation::UnauthorizedWithScopeMarker,
            |_| -> Result<f64, FetchError> { unreachable!("403 must never parse the body") },
        )
        .unwrap_err();
        assert!(
            matches!(err, FetchError::Upstream(_)),
            "an OpenAI 403 is a geo-restriction error: {err}"
        );
    }

    #[test]
    fn cost_note_maps_revoked_rate_limited_and_unexpected_to_errors() {
        let revoked =
            cost_note(401, b"", "Anthropic", OrgLimitation::Forbidden, |_| Ok(0.0)).unwrap_err();
        assert!(
            matches!(&revoked, FetchError::Upstream(m) if m.contains("Anthropic")),
            "401 names the provider in the reconnect message: {revoked}"
        );
        assert!(matches!(
            cost_note(
                429,
                b"",
                "OpenAI",
                OrgLimitation::UnauthorizedWithScopeMarker,
                |_| Ok(0.0)
            ),
            Err(FetchError::RateLimited)
        ));
        assert!(matches!(
            cost_note(
                500,
                b"",
                "OpenAI",
                OrgLimitation::UnauthorizedWithScopeMarker,
                |_| Ok(0.0)
            ),
            Err(FetchError::Upstream(_))
        ));
    }

    #[test]
    fn cost_note_reads_a_401_insufficient_permissions_body_as_the_org_limitation() {
        // OpenAI overloads 401 for a valid key that lacks the org-usage scope (`insufficient_
        // permissions` / "Missing scopes"). That is the SAME honest limitation as a 403 — never a
        // rejection — so a valid non-admin key (the common case) doesn't falsely read "reconnect
        // it". The spend parser must never run on this path.
        let body = br#"{"error":{"type":"insufficient_permissions","code":"insufficient_permissions","message":"You have insufficient permissions for this operation. Missing scopes: api.usage.read"}}"#;
        let note = cost_note(
            401,
            body,
            "OpenAI",
            OrgLimitation::UnauthorizedWithScopeMarker,
            |_| -> Result<f64, FetchError> {
                unreachable!("a limitation 401 must never parse the body")
            },
        )
        .expect("an insufficient-permissions 401 is the honest limitation, not an error");
        assert_eq!(note, UsageNote::OrgAdminKeyRequired);

        // The prose "Missing scopes" message alone (no type token) trips it too.
        let note = cost_note(
            401,
            b"Missing scopes: api.usage.read",
            "OpenAI",
            OrgLimitation::UnauthorizedWithScopeMarker,
            |_| Ok(0.0),
        )
        .expect("the missing-scopes marker is enough");
        assert_eq!(note, UsageNote::OrgAdminKeyRequired);
    }

    #[test]
    fn cost_note_keeps_a_non_scope_401_as_a_revoked_key() {
        // A genuinely invalid/revoked key carries a different marker (`invalid_api_key`), not the
        // org-scope one — so it stays a reconnect error that names the provider, not the limitation.
        let err = cost_note(
            401,
            br#"{"error":{"code":"invalid_api_key","message":"Incorrect API key provided"}}"#,
            "OpenAI",
            OrgLimitation::UnauthorizedWithScopeMarker,
            |_| Ok(0.0),
        )
        .unwrap_err();
        assert!(
            matches!(&err, FetchError::Upstream(m) if m.contains("OpenAI")),
            "an invalid-key 401 stays a revoked-key error: {err}"
        );
    }

    #[test]
    fn cost_note_propagates_a_parser_error_on_200() {
        let err = cost_note(200, b"garbage", "OpenAI", OrgLimitation::Forbidden, |_| {
            Err(FetchError::Upstream("bad json".into()))
        })
        .unwrap_err();
        assert!(matches!(err, FetchError::Upstream(_)));
    }
}
