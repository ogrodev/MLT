//! OpenRouter API-key validation (PRD §4, ADR 0016).
//!
//! Task 003 owns only the *credential* lifecycle: prove a pasted key authenticates before we
//! store it, so an invalid key is rejected with a clear error and never appears connected.
//! Reading and rendering usage with that key is task 006 — deliberately not done here. We
//! validate with the cheapest authenticated call OpenRouter offers, `GET /api/v1/key`, which
//! returns 200 for a usable key and 401/403 for a bad one; we read only the status. The HTTP
//! IO is injected via [`HttpPort`], so the decision (status → verdict) is pure and unit-tested
//! against a fake — no live account is ever touched in `cargo test`.
use crate::ports::{HttpPort, HttpRequest};

/// OpenRouter's key-info endpoint. A successful (200) authenticated GET proves the key works;
/// parsing the usage in its body is task 006, not this module.
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
    let req = HttpRequest {
        method: "GET".into(),
        url: KEY_URL.into(),
        headers: vec![("Authorization".into(), format!("Bearer {key}"))],
        body: None,
    };
    match http.send(req).await {
        Ok(resp) => match resp.status {
            200 => Ok(()),
            401 | 403 => Err(ApiKeyError::Rejected),
            other => Err(ApiKeyError::Unexpected(other)),
        },
        Err(_) => Err(ApiKeyError::Unreachable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{HttpResponse, PortError};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
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
}
