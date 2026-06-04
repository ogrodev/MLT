//! The *invariant* policy shared by the API-cost providers (OpenAI task 007, Anthropic task 008).
//!
//! Both are API-cost sources (ADR 0014): the user pastes a normal key, we authenticate it against
//! a cheap models endpoint, then read 30-day spend from a cost endpoint that exposes a USD total
//! with **no quota** (PROVIDERS.md). The *decisions* are identical across the two — what an HTTP
//! status means for key validation and for a cost read, and how the stored key is read from our
//! keychain — so they live here, pure and unit-tested against fakes. Everything *provider-specific*
//! stays in each provider module: the endpoint URLs, the auth header shape, the request bodies, the
//! spend/identity JSON parsers, and the user-facing `Display` wording each maps these verdicts to.
use crate::domain::ProviderId;
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

/// What a cost endpoint's HTTP status means for an API-cost provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostOutcome {
    Ok,
    OrgLimitation,
    Revoked,
    RateLimited,
    Unexpected(u16),
}

/// Classify a cost-endpoint status: 200=>Ok (parse for spend), 403=>OrgLimitation (authenticated
/// but no org-usage scope — the honest limitation, not an error), 401=>Revoked, 429=>RateLimited,
/// else Unexpected.
pub fn classify_cost_status(status: u16) -> CostOutcome {
    match status {
        200 => CostOutcome::Ok,
        403 => CostOutcome::OrgLimitation,
        401 => CostOutcome::Revoked,
        429 => CostOutcome::RateLimited,
        other => CostOutcome::Unexpected(other),
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
    fn classify_cost_status_maps_each_status_to_its_outcome() {
        assert_eq!(classify_cost_status(200), CostOutcome::Ok);
        // 403 is the honest org limitation, not an error.
        assert_eq!(classify_cost_status(403), CostOutcome::OrgLimitation);
        assert_eq!(classify_cost_status(401), CostOutcome::Revoked);
        assert_eq!(classify_cost_status(429), CostOutcome::RateLimited);
        assert_eq!(classify_cost_status(418), CostOutcome::Unexpected(418));
        assert_eq!(classify_cost_status(500), CostOutcome::Unexpected(500));
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
}
