//! Codex credential adapter + provider wiring.
//!
//! Reuses the Codex CLI's existing OAuth login: a plaintext `~/.codex/auth.json` (or
//! `$CODEX_HOME/auth.json`) — no Keychain involved, so discovery and reads are simple file
//! operations (ADR 0012; metadata-only discovery → per-source opt-in applies at the app layer).
//! Refreshed tokens are cached under OUR keychain service, never written back to `auth.json`.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;

use mlt_core::domain::OAuthTokens;
use mlt_core::ports::{
    Clock, HttpPort, IdentityStore, OAuthCredentialSource, PortError, SecretStore,
};
use mlt_core::providers::codex::{
    token_expiry, CodexStrategy, CACHE_KEY, CLIENT_ID, DEFAULT_USAGE_URL, REFRESH_SCOPE, TOKEN_URL,
};
use mlt_core::providers::oauth::OAuthRefresher;

use crate::{KeyringSecretStore, ReqwestHttp, SystemClock, KEYCHAIN_SERVICE};

/// Reads the Codex CLI's OAuth tokens from `~/.codex/auth.json` (or `$CODEX_HOME/auth.json`).
#[derive(Debug, Default, Clone, Copy)]
pub struct CodexCredentials;

impl CodexCredentials {
    /// Metadata-only presence check (ADR 0012): does a Codex login *exist* on this machine?
    /// `is_file` is a stat — it never opens, decrypts, or parses the token, so even a
    /// present-but-garbage file counts as "a login exists here". The secret is only read later
    /// via [`OAuthCredentialSource::load`], after the user opts the source in.
    pub fn is_present() -> bool {
        codex_home()
            .map(|home| auth_file_in(&home).is_file())
            .unwrap_or(false)
    }
}

/// The Codex home directory: `$CODEX_HOME` if set, else `~/.codex`.
fn codex_home() -> Option<PathBuf> {
    match std::env::var_os("CODEX_HOME") {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => dirs::home_dir().map(|h| h.join(".codex")),
    }
}

/// The credentials file inside a given Codex home. Split out so presence/parsing can be
/// unit-tested against a temp dir without touching the real `~/.codex`.
fn auth_file_in(home: &Path) -> PathBuf {
    home.join("auth.json")
}

#[async_trait]
impl OAuthCredentialSource for CodexCredentials {
    async fn load(&self) -> Result<OAuthTokens, PortError> {
        let path = codex_home()
            .map(|h| auth_file_in(&h))
            .ok_or(PortError::NotFound)?;
        let raw = std::fs::read_to_string(&path).map_err(|_| PortError::NotFound)?;
        parse_creds(&raw).map_err(PortError::Io)
    }
}

/// Read a string field by either snake_case or camelCase key, trimmed and non-empty.
fn pick(obj: &serde_json::Value, snake: &str, camel: &str) -> Option<String> {
    obj.get(snake)
        .or_else(|| obj.get(camel))
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Parse `auth.json` into normalized [`OAuthTokens`]. Two shapes are accepted:
/// - **OAuth** (`{ "tokens": { access_token, refresh_token, account_id, … } }`): the usual
///   `codex login`. Expiry comes from the access token's own JWT `exp` (the CLI keeps no
///   explicit expiry field), so the shared refresher's clock comparison works as-is.
/// - **API key** (`{ "OPENAI_API_KEY": "…" }`): the key is the bearer; it never expires and
///   has no refresh token, so the refresher uses it directly and never refreshes.
fn parse_creds(raw: &str) -> Result<OAuthTokens, String> {
    let value: serde_json::Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;

    let api_key = value
        .get("OPENAI_API_KEY")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(key) = api_key {
        return Ok(OAuthTokens {
            access_token: key.to_string(),
            refresh_token: None,
            expires_at: None,
            scopes: Vec::new(),
            subscription_type: None,
            account_id: None,
        });
    }

    let tokens = value.get("tokens").ok_or("no tokens in Codex auth.json")?;
    let access_token =
        pick(tokens, "access_token", "accessToken").ok_or("no access_token in Codex auth.json")?;
    let refresh_token = pick(tokens, "refresh_token", "refreshToken");
    let account_id = pick(tokens, "account_id", "accountId");
    let expires_at = token_expiry(&access_token);

    Ok(OAuthTokens {
        access_token,
        refresh_token,
        expires_at,
        scopes: Vec::new(),
        subscription_type: None,
        account_id,
    })
}

/// Best-effort detection of the installed Codex CLI version for an honest
/// `User-Agent: codex_cli_rs/<version>` header (we identify as a Codex client, not a browser).
pub fn detect_user_agent() -> String {
    let version = std::process::Command::new("codex")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| {
            // Output is something like "codex-cli 0.20.0"; take the first version-looking token.
            s.split_whitespace()
                .find(|t| t.starts_with(|c: char| c.is_ascii_digit()))
                .map(String::from)
        })
        .unwrap_or_else(|| "unknown".into());
    format!("codex_cli_rs/{version}")
}

/// Build a ready-to-run Codex strategy wired with the real adapters. Credentials flow through
/// the shared refresher: it reuses the Codex CLI's live token when fresh and only refreshes
/// (caching into OUR keychain, never `~/.codex/auth.json`) when that token has expired.
pub fn codex_strategy(identity: Arc<dyn IdentityStore>) -> CodexStrategy {
    let http: Arc<dyn HttpPort> = Arc::new(ReqwestHttp::new());
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let bootstrap: Arc<dyn OAuthCredentialSource> = Arc::new(CodexCredentials);
    let cache: Arc<dyn SecretStore> = Arc::new(KeyringSecretStore::new(KEYCHAIN_SERVICE));
    let creds: Arc<dyn OAuthCredentialSource> = Arc::new(
        OAuthRefresher::new(
            bootstrap,
            cache,
            http.clone(),
            clock.clone(),
            TOKEN_URL,
            CLIENT_ID,
            CACHE_KEY,
        )
        .with_scope(REFRESH_SCOPE),
    );
    CodexStrategy {
        creds,
        http,
        clock,
        user_agent: detect_user_agent(),
        usage_url: DEFAULT_USAGE_URL.into(),
        identity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A JWT (header.payload.sig) whose payload is {"exp":1893456000,"email":"exp@example.com"}.
    const JWT_WITH_EXP: &str = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJleHAiOjE4OTM0NTYwMDAsImVtYWlsIjoiZXhwQGV4YW1wbGUuY29tIn0.sig";

    #[test]
    fn parses_oauth_shape_with_account_id_and_jwt_expiry() {
        let raw = format!(
            r#"{{"tokens":{{"access_token":"{JWT_WITH_EXP}","refresh_token":"rt-codex",
                "id_token":"id-tok","account_id":"acct-xyz"}},"last_refresh":"2026-06-01T12:00:00Z"}}"#
        );
        let t = parse_creds(&raw).unwrap();
        assert_eq!(t.access_token, JWT_WITH_EXP);
        assert_eq!(t.refresh_token.as_deref(), Some("rt-codex"));
        assert_eq!(t.account_id.as_deref(), Some("acct-xyz"));
        // Expiry is read from the access token's JWT `exp` (1893456000 s → ms).
        assert_eq!(
            t.expires_at,
            Some(mlt_core::domain::Timestamp(1_893_456_000_000))
        );
    }

    #[test]
    fn parses_camel_case_keys() {
        let raw = format!(
            r#"{{"tokens":{{"accessToken":"{JWT_WITH_EXP}","refreshToken":"rt","accountId":"a1"}}}}"#
        );
        let t = parse_creds(&raw).unwrap();
        assert_eq!(t.refresh_token.as_deref(), Some("rt"));
        assert_eq!(t.account_id.as_deref(), Some("a1"));
    }

    #[test]
    fn parses_api_key_shape_as_a_non_refreshing_token() {
        let t = parse_creds(r#"{"OPENAI_API_KEY":"sk-proj-abc"}"#).unwrap();
        assert_eq!(t.access_token, "sk-proj-abc");
        assert!(t.refresh_token.is_none());
        assert!(
            t.expires_at.is_none(),
            "an API key never expires / never refreshes"
        );
        assert!(t.account_id.is_none());
    }

    #[test]
    fn rejects_credentials_without_tokens() {
        assert!(parse_creds("not json").is_err());
        assert!(parse_creds("{}").is_err());
        assert!(parse_creds(r#"{"tokens":{"refresh_token":"r"}}"#).is_err()); // no access_token
    }

    #[test]
    fn detect_user_agent_has_the_codex_prefix() {
        assert!(detect_user_agent().starts_with("codex_cli_rs/"));
    }

    #[test]
    fn presence_is_metadata_only_and_never_reads_the_secret() {
        let base = std::env::temp_dir().join(format!("mlt-codex-presence-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);

        assert!(
            !auth_file_in(&base).is_file(),
            "absent auth.json ⇒ not present"
        );

        std::fs::create_dir_all(&base).unwrap();
        // Deliberately INVALID content: presence is decided from existence alone, while the
        // secret-reading path (`parse_creds`) rejects it — proof the probe never reads the token.
        std::fs::write(auth_file_in(&base), "not a real credential").unwrap();
        assert!(
            auth_file_in(&base).is_file(),
            "existing file ⇒ present (stat only)"
        );
        assert!(parse_creds("not a real credential").is_err());

        let _ = std::fs::remove_dir_all(&base);
    }
}
