//! Codex credential discovery + provider wiring.
//!
//! Codex logins live in two places on a machine: the Codex CLI's plaintext `~/.codex/auth.json`
//! (or `$CODEX_HOME/auth.json`), and Oh My Pi's per-profile SQLite credential store
//! (`~/.omp[/profiles/*]/agent/agent.db`, provider `openai-codex`). We enumerate both, dedupe by
//! ChatGPT account id (freshest token wins), and surface each distinct account as its own
//! source. Reads are best-effort and **read-only**; refreshed tokens are cached under OUR
//! keychain, never written back to either store (the AGENTS.md invariant).
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use rusqlite::{Connection, OpenFlags};

use mlt_core::domain::{OAuthTokens, Timestamp};
use mlt_core::ports::{
    Clock, HttpPort, IdentityStore, OAuthCredentialSource, PortError, SecretStore,
};
use mlt_core::providers::codex::{
    account_cache_key, parse_identity, token_expiry, CodexStrategy, CLIENT_ID, DEFAULT_USAGE_URL,
    REFRESH_SCOPE, TOKEN_URL,
};
use mlt_core::providers::oauth::OAuthRefresher;
use mlt_core::sources::CodexAccount;

use crate::{KeyringSecretStore, ReqwestHttp, SystemClock, KEYCHAIN_SERVICE};

// ── Discovery ─────────────────────────────────────────────────────────────────

/// One discovered Codex login plus its token, before dedup across stores.
struct RawAccount {
    account_id: String,
    email: Option<String>,
    origin: String,
    tokens: OAuthTokens,
    /// Access-token expiry (ms epoch) — picks the freshest copy when an account appears in
    /// several stores (e.g. both the Codex CLI and an Oh My Pi profile).
    expires_ms: i64,
}

/// Enumerate the distinct Codex logins on this machine for the connect catalog (deduped).
pub fn codex_accounts() -> Vec<CodexAccount> {
    dedup_freshest(read_all())
        .into_iter()
        .map(|raw| CodexAccount {
            account_id: raw.account_id,
            email: raw.email,
            origin: raw.origin,
        })
        .collect()
}

/// Read every Codex login from both stores (un-deduped).
fn read_all() -> Vec<RawAccount> {
    let mut accounts = read_codex_cli();
    accounts.extend(read_omp_profiles());
    accounts
}

/// Collapse logins that share a ChatGPT account id, keeping the one with the latest expiry, in a
/// stable order (by account id) so the source list doesn't reshuffle between polls.
fn dedup_freshest(raw: Vec<RawAccount>) -> Vec<RawAccount> {
    let mut best: BTreeMap<String, RawAccount> = BTreeMap::new();
    for account in raw {
        match best.get(&account.account_id) {
            Some(existing) if existing.expires_ms >= account.expires_ms => {}
            _ => {
                best.insert(account.account_id.clone(), account);
            }
        }
    }
    best.into_values().collect()
}

// ── Codex CLI (~/.codex/auth.json) ─────────────────────────────────────────────

fn codex_cli_auth_path() -> Option<PathBuf> {
    match std::env::var_os("CODEX_HOME") {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir).join("auth.json")),
        _ => dirs::home_dir().map(|h| h.join(".codex/auth.json")),
    }
}

fn read_codex_cli() -> Vec<RawAccount> {
    let Some(path) = codex_cli_auth_path() else {
        return Vec::new();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(tokens) = parse_codex_cli(&raw) else {
        return Vec::new();
    };
    // Only OAuth subscription logins are Codex accounts; an API-key auth.json has no account id
    // and no ChatGPT-subscription usage to show.
    let Some(account_id) = tokens.account_id.clone() else {
        return Vec::new();
    };
    let expires_ms = tokens.expires_at.map(|t| t.0).unwrap_or(0);
    let email = parse_identity(&tokens.access_token).email;
    vec![RawAccount {
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

// ── Oh My Pi profiles (~/.omp[/profiles/*]/agent/agent.db) ──────────────────────

/// Oh My Pi's data home: `$OMP_HOME` if set, else `~/.omp`.
fn omp_home() -> Option<PathBuf> {
    match std::env::var_os("OMP_HOME") {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => dirs::home_dir().map(|h| h.join(".omp")),
    }
}

/// Every profile's credential DB: the default (`agent/agent.db`) plus each named profile under
/// `profiles/<name>/agent/agent.db`, paired with a human label for the source subtitle.
fn omp_agent_dbs() -> Vec<(String, PathBuf)> {
    let Some(home) = omp_home() else {
        return Vec::new();
    };
    let mut dbs = Vec::new();
    let default = home.join("agent").join("agent.db");
    if default.is_file() {
        dbs.push(("default".to_string(), default));
    }
    if let Ok(entries) = std::fs::read_dir(home.join("profiles")) {
        for entry in entries.flatten() {
            let db = entry.path().join("agent").join("agent.db");
            if db.is_file() {
                dbs.push((entry.file_name().to_string_lossy().into_owned(), db));
            }
        }
    }
    dbs
}

fn read_omp_profiles() -> Vec<RawAccount> {
    let mut out = Vec::new();
    for (profile, db) in omp_agent_dbs() {
        out.extend(read_omp_db(&db, &profile));
    }
    out
}

/// Open an external SQLite DB **read-only** — MLT never writes Oh My Pi's store. Read-only is
/// sufficient to read a live WAL database while Oh My Pi has it open.
fn open_ro(db: &Path) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)
}

fn read_omp_db(db: &Path, profile: &str) -> Vec<RawAccount> {
    let Ok(conn) = open_ro(db) else {
        return Vec::new();
    };
    let Ok(mut stmt) =
        conn.prepare("SELECT data FROM auth_credentials WHERE provider = 'openai-codex'")
    else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) else {
        return Vec::new();
    };
    rows.flatten()
        .filter_map(|data| parse_omp_credential(&data, profile))
        .collect()
}

/// Parse one Oh My Pi `openai-codex` credential blob
/// (`{ access, refresh, accountId, email, expires }`) into a [`RawAccount`]. `expires` is ms
/// epoch; we fall back to the access token's JWT `exp` when it's absent.
fn parse_omp_credential(data: &str, profile: &str) -> Option<RawAccount> {
    let value: serde_json::Value = serde_json::from_str(data).ok()?;
    let access_token = json_str(&value, "access")?;
    let account_id = json_str(&value, "accountId")?;
    let refresh_token = json_str(&value, "refresh");
    let email = json_str(&value, "email");
    let expires_ms = value
        .get("expires")
        .and_then(serde_json::Value::as_i64)
        .or_else(|| token_expiry(&access_token).map(|t| t.0))
        .unwrap_or(0);
    Some(RawAccount {
        account_id: account_id.clone(),
        email,
        origin: format!("Oh My Pi · {profile}"),
        tokens: OAuthTokens {
            access_token,
            refresh_token,
            expires_at: Some(Timestamp(expires_ms)),
            scopes: Vec::new(),
            subscription_type: None,
            account_id: Some(account_id),
        },
        expires_ms,
    })
}

// ── Field helpers ───────────────────────────────────────────────────────────--

/// Read a trimmed, non-empty string field by either snake_case or camelCase key.
fn pick(obj: &serde_json::Value, snake: &str, camel: &str) -> Option<String> {
    obj.get(snake)
        .or_else(|| obj.get(camel))
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Read a trimmed, non-empty string field by a single key.
fn json_str(obj: &serde_json::Value, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

// ── Provider wiring ───────────────────────────────────────────────────────────

/// Credentials for one Codex account, resolved by ChatGPT account id across every store
/// (freshest token wins). Re-reads on each load, so MLT always uses the latest token Oh My Pi or
/// the Codex CLI has refreshed — and never writes either store.
pub struct CodexAccountCredentials {
    account_id: String,
}

impl CodexAccountCredentials {
    pub fn new(account_id: impl Into<String>) -> Self {
        Self {
            account_id: account_id.into(),
        }
    }
}

#[async_trait]
impl OAuthCredentialSource for CodexAccountCredentials {
    async fn load(&self) -> Result<OAuthTokens, PortError> {
        dedup_freshest(read_all())
            .into_iter()
            .find(|raw| raw.account_id == self.account_id)
            .map(|raw| raw.tokens)
            .ok_or(PortError::NotFound)
    }
}

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

/// Build a ready-to-run Codex strategy for one account. Credentials flow through the shared
/// refresher with a per-account cache key, so each login refreshes (into OUR keychain, never the
/// vendor store) independently of the others.
pub fn codex_strategy(account_id: &str, identity: Arc<dyn IdentityStore>) -> CodexStrategy {
    let http: Arc<dyn HttpPort> = Arc::new(ReqwestHttp::new());
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let bootstrap: Arc<dyn OAuthCredentialSource> =
        Arc::new(CodexAccountCredentials::new(account_id));
    let cache: Arc<dyn SecretStore> = Arc::new(KeyringSecretStore::new(KEYCHAIN_SERVICE));
    let creds: Arc<dyn OAuthCredentialSource> = Arc::new(
        OAuthRefresher::new(
            bootstrap,
            cache,
            http.clone(),
            clock.clone(),
            TOKEN_URL,
            CLIENT_ID,
            account_cache_key(account_id),
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
        assert_eq!(t.expires_at, Some(Timestamp(1_893_456_000_000)));
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
    fn parses_omp_credential_blob() {
        let data = r#"{"access":"at","refresh":"rt","accountId":"acct-1","email":"a@x.com","expires":1781000000000}"#;
        let raw = parse_omp_credential(data, "work").expect("parse");
        assert_eq!(raw.account_id, "acct-1");
        assert_eq!(raw.email.as_deref(), Some("a@x.com"));
        assert_eq!(raw.origin, "Oh My Pi · work");
        assert_eq!(raw.expires_ms, 1_781_000_000_000);
        assert_eq!(raw.tokens.access_token, "at");
        assert_eq!(raw.tokens.refresh_token.as_deref(), Some("rt"));
        assert_eq!(raw.tokens.account_id.as_deref(), Some("acct-1"));
        assert_eq!(raw.tokens.expires_at, Some(Timestamp(1_781_000_000_000)));
    }

    #[test]
    fn omp_credential_requires_access_and_account() {
        assert!(parse_omp_credential(r#"{"refresh":"r"}"#, "p").is_none());
        assert!(parse_omp_credential(r#"{"access":"a"}"#, "p").is_none()); // no accountId
    }

    #[test]
    fn dedup_keeps_the_freshest_token_per_account() {
        let mk = |acct: &str, exp: i64, at: &str| RawAccount {
            account_id: acct.into(),
            email: None,
            origin: "x".into(),
            tokens: oauth(at, exp),
            expires_ms: exp,
        };
        let deduped = dedup_freshest(vec![
            mk("acct-1", 100, "old"),
            mk("acct-1", 500, "new"), // freshest copy of acct-1 wins
            mk("acct-2", 300, "two"),
        ]);
        assert_eq!(deduped.len(), 2);
        let a1 = deduped.iter().find(|r| r.account_id == "acct-1").unwrap();
        assert_eq!(a1.tokens.access_token, "new");
        assert!(deduped.iter().any(|r| r.account_id == "acct-2"));
    }

    #[test]
    fn detect_user_agent_has_the_codex_prefix() {
        assert!(detect_user_agent().starts_with("codex_cli_rs/"));
    }

    fn oauth(access: &str, exp_ms: i64) -> OAuthTokens {
        OAuthTokens {
            access_token: access.into(),
            refresh_token: Some("r".into()),
            expires_at: Some(Timestamp(exp_ms)),
            scopes: Vec::new(),
            subscription_type: None,
            account_id: Some("x".into()),
        }
    }
}
