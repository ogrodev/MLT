//! Shared OAuth token refresh for reused-login providers (Claude Code, Codex).
//!
//! Both providers reuse a vendor CLI's existing OAuth login and only need a *valid* access
//! token at fetch time. This refresher keeps one available: when the vendor's own token is
//! fresh it is returned directly — **without even reading our keychain cache**, so the OS
//! isn't prompted on every poll — and we fall back to our cached refreshed copy (then a
//! network refresh) only when that token is missing or stale. Refreshed tokens are stored in
//! OUR [`SecretStore`], **NEVER** written back to the vendor's own credential store (the
//! AGENTS.md invariant), so we can't break the user's CLI login.
//!
//! The only per-provider differences are the token endpoint, client id, cache key, and an
//! optional `scope` in the refresh body — all injected, so the logic lives here once instead
//! of being copy-pasted per provider.
use crate::domain::*;
use crate::ports::*;
use async_trait::async_trait;
use std::sync::Arc;

/// Refresh a little before expiry so a token doesn't lapse mid-request.
const REFRESH_SKEW_MS: i64 = 60_000;

/// An [`OAuthCredentialSource`] that wraps a vendor-CLI credential source and keeps a valid
/// access token available, refreshing only when necessary (see module docs).
pub struct OAuthRefresher {
    bootstrap: Arc<dyn OAuthCredentialSource>,
    cache: Arc<dyn SecretStore>,
    http: Arc<dyn HttpPort>,
    clock: Arc<dyn Clock>,
    token_url: String,
    client_id: String,
    /// Keychain entry (under MLT's own service) where OUR refreshed copy is cached. Distinct
    /// per provider so two providers' caches never collide; never the vendor's own store.
    cache_key: String,
    /// Extra `scope` to send in the refresh body when the provider requires it (Codex sends
    /// `openid profile email`; Claude sends none).
    scope: Option<String>,
}

impl OAuthRefresher {
    pub fn new(
        bootstrap: Arc<dyn OAuthCredentialSource>,
        cache: Arc<dyn SecretStore>,
        http: Arc<dyn HttpPort>,
        clock: Arc<dyn Clock>,
        token_url: impl Into<String>,
        client_id: impl Into<String>,
        cache_key: impl Into<String>,
    ) -> Self {
        Self {
            bootstrap,
            cache,
            http,
            clock,
            token_url: token_url.into(),
            client_id: client_id.into(),
            cache_key: cache_key.into(),
            scope: None,
        }
    }

    /// Send `scope` in the refresh body (required by Codex's OAuth token endpoint).
    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = Some(scope.into());
        self
    }

    fn is_fresh(&self, t: &OAuthTokens) -> bool {
        match t.expires_at {
            Some(exp) => exp.0 > self.clock.now().0 + REFRESH_SKEW_MS,
            None => true,
        }
    }

    fn load_cached(&self) -> Option<OAuthTokens> {
        self.cache
            .get(&self.cache_key)
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
    }

    async fn refresh(&self, base: &OAuthTokens) -> Result<OAuthTokens, PortError> {
        let refresh_token = base
            .refresh_token
            .as_deref()
            .ok_or_else(|| PortError::Io("token expired and no refresh_token available".into()))?;
        let mut payload = serde_json::json!({
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "client_id": self.client_id,
        });
        if let Some(scope) = &self.scope {
            payload["scope"] = serde_json::Value::String(scope.clone());
        }
        let resp = self
            .http
            .send(HttpRequest {
                method: "POST".into(),
                url: self.token_url.clone(),
                headers: vec![("Content-Type".into(), "application/json".into())],
                body: Some(payload.to_string().into_bytes()),
            })
            .await?;
        if resp.status != 200 {
            return Err(PortError::Io(format!(
                "token refresh failed: HTTP {}",
                resp.status
            )));
        }
        parse_refresh_response(&resp.body, base, self.clock.now())
    }
}

/// Pure parser for the OAuth token response. `base` supplies fallbacks: a rotated refresh
/// token may be omitted, and scopes / subscription / account id carry over when not returned.
fn parse_refresh_response(
    body: &[u8],
    base: &OAuthTokens,
    now: Timestamp,
) -> Result<OAuthTokens, PortError> {
    let v: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| PortError::Io(format!("bad token json: {e}")))?;
    let access_token = v
        .get("access_token")
        .and_then(|x| x.as_str())
        .ok_or_else(|| PortError::Io("no access_token in refresh response".into()))?
        .to_string();
    let refresh_token = v
        .get("refresh_token")
        .and_then(|x| x.as_str())
        .map(String::from)
        .or_else(|| base.refresh_token.clone());
    let expires_at = v
        .get("expires_in")
        .and_then(|x| x.as_i64())
        .map(|secs| Timestamp(now.0 + secs * 1000));
    let scopes = v
        .get("scope")
        .and_then(|x| x.as_str())
        .map(|s| s.split(' ').map(String::from).collect::<Vec<_>>())
        .unwrap_or_else(|| base.scopes.clone());
    Ok(OAuthTokens {
        access_token,
        refresh_token,
        expires_at,
        scopes,
        subscription_type: base.subscription_type.clone(),
        account_id: base.account_id.clone(),
    })
}

/// Of two optional token sets, keep the one with the later expiry (treating "no expiry" as
/// far future). Lets us prefer the vendor's continually-refreshed token over a stale cache.
fn pick_freshest(a: Option<OAuthTokens>, b: Option<OAuthTokens>) -> Option<OAuthTokens> {
    match (a, b) {
        (Some(a), Some(b)) => {
            let ae = a.expires_at.map(|t| t.0).unwrap_or(i64::MAX);
            let be = b.expires_at.map(|t| t.0).unwrap_or(i64::MAX);
            Some(if ae >= be { a } else { b })
        }
        (Some(only), None) | (None, Some(only)) => Some(only),
        (None, None) => None,
    }
}

#[async_trait]
impl OAuthCredentialSource for OAuthRefresher {
    async fn load(&self) -> Result<OAuthTokens, PortError> {
        let bootstrap = self.bootstrap.load().await.ok();
        // Fast path: the vendor CLI keeps its own token fresh, often in a plaintext file (no
        // keychain). When it's fresh, return it directly and never read our keychain cache —
        // that read is what pops a macOS keychain prompt on every refresh.
        match bootstrap {
            Some(token) if self.is_fresh(&token) => Ok(token),
            // Vendor token stale or absent → consult our cached refreshed copy (a keychain
            // read), prefer whichever is freshest, and refresh only as a last resort.
            bootstrap => {
                let chosen =
                    pick_freshest(self.load_cached(), bootstrap).ok_or(PortError::NotFound)?;
                if self.is_fresh(&chosen) {
                    return Ok(chosen);
                }
                let refreshed = self.refresh(&chosen).await?;
                if let Ok(json) = serde_json::to_string(&refreshed) {
                    let _ = self.cache.set(&self.cache_key, &json);
                }
                Ok(refreshed)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const TEST_TOKEN_URL: &str = "https://example.test/oauth/token";
    const TEST_CLIENT_ID: &str = "client-test";
    const TEST_CACHE_KEY: &str = "oauth.test";

    struct FakeCreds(Option<OAuthTokens>);
    #[async_trait]
    impl OAuthCredentialSource for FakeCreds {
        async fn load(&self) -> Result<OAuthTokens, PortError> {
            self.0.clone().ok_or(PortError::NotFound)
        }
    }

    /// Captures the last request body so tests can assert what was sent to the token endpoint.
    struct FakeHttp {
        status: u16,
        body: String,
        calls: AtomicUsize,
        last_body: Mutex<Option<String>>,
    }
    impl FakeHttp {
        fn new(status: u16, body: &str) -> Self {
            Self {
                status,
                body: body.into(),
                calls: AtomicUsize::new(0),
                last_body: Mutex::new(None),
            }
        }
    }
    #[async_trait]
    impl HttpPort for FakeHttp {
        async fn send(&self, req: HttpRequest) -> Result<HttpResponse, PortError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.last_body.lock() = req.body.map(|b| String::from_utf8_lossy(&b).into_owned());
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
    struct MemSecrets(Mutex<HashMap<String, String>>);
    impl SecretStore for MemSecrets {
        fn get(&self, k: &str) -> Result<Option<String>, PortError> {
            Ok(self.0.lock().get(k).cloned())
        }
        fn set(&self, k: &str, v: &str) -> Result<(), PortError> {
            self.0.lock().insert(k.into(), v.into());
            Ok(())
        }
        fn delete(&self, k: &str) -> Result<(), PortError> {
            self.0.lock().remove(k);
            Ok(())
        }
    }

    fn tokens_expiring_at(exp: i64) -> OAuthTokens {
        OAuthTokens {
            access_token: "old-access".into(),
            refresh_token: Some("rt-old".into()),
            expires_at: Some(Timestamp(exp)),
            scopes: vec!["user:profile".into()],
            subscription_type: Some("team".into()),
            account_id: Some("acct-old".into()),
        }
    }

    fn refresher(
        bootstrap: Option<OAuthTokens>,
        cache: Arc<MemSecrets>,
        http: Arc<FakeHttp>,
        now: i64,
    ) -> OAuthRefresher {
        OAuthRefresher::new(
            Arc::new(FakeCreds(bootstrap)),
            cache,
            http,
            Arc::new(FakeClock(now)),
            TEST_TOKEN_URL,
            TEST_CLIENT_ID,
            TEST_CACHE_KEY,
        )
    }

    #[tokio::test]
    async fn uses_fresh_token_without_calling_http() {
        let http = Arc::new(FakeHttp::new(200, "{}"));
        let r = refresher(
            Some(tokens_expiring_at(10_000_000)),
            Arc::new(MemSecrets::default()),
            http.clone(),
            1_000,
        );
        let t = r.load().await.unwrap();
        assert_eq!(t.access_token, "old-access");
        assert_eq!(
            http.calls.load(Ordering::SeqCst),
            0,
            "fresh token must not refresh"
        );
    }

    #[tokio::test]
    async fn refreshes_expired_token_and_caches_it() {
        let cache = Arc::new(MemSecrets::default());
        let body = r#"{"access_token":"new-access","refresh_token":"rt-new","expires_in":3600,
            "scope":"user:profile"}"#;
        let http = Arc::new(FakeHttp::new(200, body));
        let r = refresher(
            Some(tokens_expiring_at(500)),
            cache.clone(),
            http.clone(),
            1_000,
        );
        let t = r.load().await.unwrap();
        assert_eq!(t.access_token, "new-access");
        assert_eq!(t.refresh_token.as_deref(), Some("rt-new"));
        assert_eq!(t.expires_at, Some(Timestamp(1_000 + 3_600_000)));
        // account id is not returned by a refresh — it must carry forward from the base token.
        assert_eq!(t.account_id.as_deref(), Some("acct-old"));
        assert_eq!(http.calls.load(Ordering::SeqCst), 1);
        assert!(
            cache.get(TEST_CACHE_KEY).unwrap().is_some(),
            "refreshed token must be cached"
        );
    }

    #[tokio::test]
    async fn sends_configured_scope_in_refresh_body() {
        let cache = Arc::new(MemSecrets::default());
        let http = Arc::new(FakeHttp::new(
            200,
            r#"{"access_token":"x","expires_in":3600}"#,
        ));
        let r = refresher(Some(tokens_expiring_at(500)), cache, http.clone(), 1_000)
            .with_scope("openid profile email");
        r.load().await.unwrap();
        let sent = http.last_body.lock().clone().expect("a body was sent");
        assert!(
            sent.contains("\"scope\":\"openid profile email\""),
            "scope must be in body: {sent}"
        );
    }

    #[tokio::test]
    async fn omits_scope_when_not_configured() {
        let cache = Arc::new(MemSecrets::default());
        let http = Arc::new(FakeHttp::new(
            200,
            r#"{"access_token":"x","expires_in":3600}"#,
        ));
        let r = refresher(Some(tokens_expiring_at(500)), cache, http.clone(), 1_000);
        r.load().await.unwrap();
        let sent = http.last_body.lock().clone().expect("a body was sent");
        assert!(!sent.contains("scope"), "no scope should be sent: {sent}");
    }

    #[tokio::test]
    async fn errors_when_expired_with_no_refresh_token() {
        let mut tk = tokens_expiring_at(500);
        tk.refresh_token = None;
        let r = refresher(
            Some(tk),
            Arc::new(MemSecrets::default()),
            Arc::new(FakeHttp::new(200, "{}")),
            1_000,
        );
        assert!(r.load().await.is_err());
    }

    #[tokio::test]
    async fn prefers_the_freshest_source() {
        // Cache holds a fresh token; bootstrap is expired → cache wins, no refresh.
        let cache = Arc::new(MemSecrets::default());
        let mut cached = tokens_expiring_at(10_000_000);
        cached.access_token = "cached-fresh".into();
        cache
            .set(TEST_CACHE_KEY, &serde_json::to_string(&cached).unwrap())
            .unwrap();
        let http = Arc::new(FakeHttp::new(200, "{}"));
        let r = refresher(Some(tokens_expiring_at(500)), cache, http.clone(), 1_000);
        let t = r.load().await.unwrap();
        assert_eq!(t.access_token, "cached-fresh");
        assert_eq!(http.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn skips_the_cache_when_the_vendor_token_is_fresh() {
        // Reading our keychain cache is what pops a macOS keychain prompt. When the vendor
        // token is fresh, the refresher must use it and never touch the cache at all.
        struct ForbiddenCache;
        impl SecretStore for ForbiddenCache {
            fn get(&self, _k: &str) -> Result<Option<String>, PortError> {
                panic!("must not read the keychain cache when the vendor token is fresh");
            }
            fn set(&self, _k: &str, _v: &str) -> Result<(), PortError> {
                panic!("must not write the keychain cache when the vendor token is fresh");
            }
            fn delete(&self, _k: &str) -> Result<(), PortError> {
                Ok(())
            }
        }
        let r = OAuthRefresher::new(
            Arc::new(FakeCreds(Some(tokens_expiring_at(10_000_000)))),
            Arc::new(ForbiddenCache),
            Arc::new(FakeHttp::new(200, "{}")),
            Arc::new(FakeClock(1_000)),
            TEST_TOKEN_URL,
            TEST_CLIENT_ID,
            TEST_CACHE_KEY,
        );
        let t = r.load().await.unwrap();
        assert_eq!(t.access_token, "old-access");
    }
}
