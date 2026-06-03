//! Codex adapter: the Codex CLI's own vendor store (`~/.codex/auth.json`) + per-account fetch
//! strategy. Multi-account discovery and the shared Oh My Pi reader live in [`crate::accounts`];
//! this module only supplies what's Codex-specific.
use std::path::PathBuf;
use std::sync::Arc;

use mlt_core::domain::OAuthTokens;
use mlt_core::ports::{Clock, HttpPort, IdentityStore, OAuthCredentialSource, SecretStore};
use mlt_core::providers::codex::{
    parse_identity, token_expiry, CodexStrategy, CLIENT_ID, DEFAULT_USAGE_URL, REFRESH_SCOPE,
    TOKEN_URL,
};
use mlt_core::providers::oauth::OAuthRefresher;
use mlt_core::sources::account_cache_key;

use crate::accounts::{AccountCredentials, RawAccount};
use crate::{KeyringSecretStore, ReqwestHttp, SystemClock, KEYCHAIN_SERVICE};

/// The base id for Codex sources and accounts.
const BASE: &str = "codex";

// ── Codex CLI vendor store (~/.codex/auth.json) ────────────────────────────────

fn codex_cli_auth_path() -> Option<PathBuf> {
    match std::env::var_os("CODEX_HOME") {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir).join("auth.json")),
        _ => dirs::home_dir().map(|h| h.join(".codex/auth.json")),
    }
}

/// Read the Codex CLI's login from `~/.codex/auth.json` as a discoverable account (deduped with
/// Oh My Pi by [`crate::accounts`]). Only an OAuth login carrying an account id qualifies — an
/// API-key auth.json has no subscription account.
pub(crate) fn codex_cli_accounts() -> Vec<RawAccount> {
    let Some(path) = codex_cli_auth_path() else {
        return Vec::new();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(tokens) = parse_codex_cli(&raw) else {
        return Vec::new();
    };
    let Some(account_id) = tokens.account_id.clone() else {
        return Vec::new();
    };
    let expires_ms = tokens.expires_at.map(|t| t.0).unwrap_or(0);
    let email = parse_identity(&tokens.access_token).email;
    vec![RawAccount {
        base: BASE,
        account_id,
        email,
        origin: "Codex CLI".into(),
        tokens,
        expires_ms,
    }]
}

/// Parse `~/.codex/auth.json` into normalized [`OAuthTokens`]. Two shapes are accepted:
/// - OAuth (`{ "tokens": { access_token, refresh_token, account_id, … } }`) — the usual login.
/// - API key (`{ "OPENAI_API_KEY": "…" }`) — no account id, so not a subscription account.
///
/// Expiry comes from the access token's JWT `exp` (the CLI keeps no explicit expiry field).
fn parse_codex_cli(raw: &str) -> Result<OAuthTokens, String> {
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

/// Read a trimmed, non-empty string field by either snake_case or camelCase key.
fn pick(obj: &serde_json::Value, snake: &str, camel: &str) -> Option<String> {
    obj.get(snake)
        .or_else(|| obj.get(camel))
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

// ── Provider wiring ───────────────────────────────────────────────────────────

/// Best-effort Codex CLI version for an honest `User-Agent: codex_cli_rs/<version>` header.
pub fn detect_user_agent() -> String {
    let version = std::process::Command::new("codex")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| {
            // Output is like "codex-cli 0.20.0"; take the first version-looking token.
            s.split_whitespace()
                .find(|t| t.starts_with(|c: char| c.is_ascii_digit()))
                .map(String::from)
        })
        .unwrap_or_else(|| "unknown".into());
    format!("codex_cli_rs/{version}")
}

/// Build a ready-to-run Codex strategy for one account. Credentials resolve to that account's
/// freshest token (across Oh My Pi + the CLI) via the shared refresher with a per-account cache
/// key, refreshing into OUR keychain — never the vendor store.
pub fn codex_strategy(account_id: &str, identity: Arc<dyn IdentityStore>) -> CodexStrategy {
    let http: Arc<dyn HttpPort> = Arc::new(ReqwestHttp::new());
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let bootstrap: Arc<dyn OAuthCredentialSource> =
        Arc::new(AccountCredentials::new(BASE, account_id));
    let cache: Arc<dyn SecretStore> = Arc::new(KeyringSecretStore::new(KEYCHAIN_SERVICE));
    let creds: Arc<dyn OAuthCredentialSource> = Arc::new(
        OAuthRefresher::new(
            bootstrap,
            cache,
            http.clone(),
            clock.clone(),
            TOKEN_URL,
            CLIENT_ID,
            account_cache_key(BASE, account_id),
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
    fn parses_codex_cli_oauth_shape_with_jwt_expiry() {
        let raw = format!(
            r#"{{"tokens":{{"access_token":"{JWT_WITH_EXP}","refresh_token":"rt","account_id":"acct-xyz"}}}}"#
        );
        let t = parse_codex_cli(&raw).unwrap();
        assert_eq!(t.access_token, JWT_WITH_EXP);
        assert_eq!(t.refresh_token.as_deref(), Some("rt"));
        assert_eq!(t.account_id.as_deref(), Some("acct-xyz"));
        assert_eq!(
            t.expires_at,
            Some(mlt_core::domain::Timestamp(1_893_456_000_000))
        );
    }

    #[test]
    fn parses_codex_cli_api_key_shape_without_an_account() {
        let t = parse_codex_cli(r#"{"OPENAI_API_KEY":"sk-x"}"#).unwrap();
        assert_eq!(t.access_token, "sk-x");
        assert!(
            t.account_id.is_none(),
            "api-key auth has no subscription account"
        );
    }

    #[test]
    fn rejects_codex_cli_without_tokens() {
        assert!(parse_codex_cli("not json").is_err());
        assert!(parse_codex_cli("{}").is_err());
    }

    #[test]
    fn detect_user_agent_has_the_codex_prefix() {
        assert!(detect_user_agent().starts_with("codex_cli_rs/"));
    }
}
