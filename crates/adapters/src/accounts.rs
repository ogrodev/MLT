//! Shared per-account discovery for reused-login providers (Codex, Claude Code).
//!
//! Both keep OAuth logins the same way: Oh My Pi stores one per profile in a SQLite credential
//! store (`~/.omp[/profiles/*]/agent/agent.db`), and some providers also have a vendor CLI store.
//! This module reads Oh My Pi's store — provider-agnostically, since the credential blob shape is
//! identical across providers (`{ access, refresh, accountId, email, expires }`) — dedupes by
//! account id keeping the freshest token, and exposes both the account list (for the connect
//! catalog) and per-account credentials (for fetching). Reads are best-effort and **read-only**;
//! refreshed tokens are cached under OUR keychain, never written back to either store (AGENTS.md).
//!
//! Adding multi-account discovery to a new provider is one row in [`PROVIDERS`] (its base id +
//! Oh My Pi provider id) plus a per-account strategy builder in that provider's adapter module.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use rusqlite::{Connection, OpenFlags};

use mlt_core::domain::{OAuthTokens, ProviderId, Timestamp};
use mlt_core::ports::{OAuthCredentialSource, PortError};
use mlt_core::providers::codex::token_expiry;
use mlt_core::sources::DiscoveredAccount;

use crate::resilience::{bounded_blocking_probe, BlockingProbe};

/// One discovered login plus its token, before dedup across stores.
pub(crate) struct RawAccount {
    pub base: &'static str,
    pub account_id: String,
    pub email: Option<String>,
    pub origin: String,
    pub tokens: OAuthTokens,
    /// Access-token expiry (ms epoch) — picks the freshest copy when an account appears in
    /// several stores (e.g. both a vendor CLI and an Oh My Pi profile).
    pub expires_ms: i64,
}

/// Base providers that expand into per-account sources, mapped to their Oh My Pi provider id.
/// One row here (plus a strategy builder) gives a provider multi-account discovery.
const PROVIDERS: &[(&str, &str)] = &[("codex", "openai-codex"), ("claude-code", "anthropic")];

/// Enumerate every distinct login across all multi-account providers, deduped per account.
pub fn discovered_accounts() -> Vec<DiscoveredAccount> {
    let mut out = Vec::new();
    for &(base, _) in PROVIDERS {
        for raw in dedup_freshest(enumerate_for(base)) {
            out.push(DiscoveredAccount {
                base: ProviderId::new(raw.base),
                account_id: raw.account_id,
                email: raw.email,
                origin: raw.origin,
            });
        }
    }
    out
}

/// Read every login for one base provider: its Oh My Pi credentials plus its vendor CLI store.
pub(crate) fn enumerate_for(base: &'static str) -> Vec<RawAccount> {
    let mut raw = Vec::new();
    if let Some(&(_, omp_provider)) = PROVIDERS.iter().find(|&&(b, _)| b == base) {
        raw.extend(omp_accounts(omp_provider, base));
    }
    // Vendor CLI stores are provider-specific: Codex keeps a plaintext auth.json; Claude Code's
    // login lives in the keychain and is surfaced separately as the static `claude-code` source.
    if base == "codex" {
        raw.extend(crate::codex::codex_cli_accounts());
    }
    raw
}

/// Collapse logins that share an account id, keeping the latest expiry, in a stable order (by
/// account id) so the source list doesn't reshuffle between polls.
pub(crate) fn dedup_freshest(raw: Vec<RawAccount>) -> Vec<RawAccount> {
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

// ── Oh My Pi store ─────────────────────────────────────────────────────────────

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

fn omp_accounts(omp_provider: &'static str, base: &'static str) -> Vec<RawAccount> {
    let Some(dbs) = bounded_blocking_probe(BlockingProbe::OmpProfiles, || Some(omp_agent_dbs()))
    else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for (profile, db) in dbs {
        let rows = bounded_blocking_probe(BlockingProbe::OmpDb, move || {
            Some(read_omp_db(&db, omp_provider, base, &profile))
        })
        .unwrap_or_default();
        out.extend(rows);
    }
    out
}

/// Open an external SQLite DB **read-only** — MLT never writes Oh My Pi's store. The normal
/// read-only open handles a live DB while Oh My Pi has it open; the immutable URI fallback covers
/// a leftover WAL where SQLite would otherwise try to initialize `-shm` and fail read-only.
fn open_ro(db: &Path) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY).or_else(|_| {
        let uri = sqlite_immutable_uri(db);
        Connection::open_with_flags(
            uri,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )
    })
}

fn sqlite_immutable_uri(db: &Path) -> String {
    let path = db.to_string_lossy();
    let mut uri = String::with_capacity("file:?mode=ro&immutable=1".len() + path.len());
    uri.push_str("file:");
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b':' | b'.' | b'-' | b'_' | b'~' => {
                uri.push(char::from(b))
            }
            _ => {
                const HEX: &[u8; 16] = b"0123456789ABCDEF";
                uri.push('%');
                uri.push(char::from(HEX[(b >> 4) as usize]));
                uri.push(char::from(HEX[(b & 0x0F) as usize]));
            }
        }
    }
    uri.push_str("?mode=ro&immutable=1");
    uri
}

fn read_omp_db(
    db: &Path,
    omp_provider: &'static str,
    base: &'static str,
    profile: &str,
) -> Vec<RawAccount> {
    let Ok(conn) = open_ro(db) else {
        return Vec::new();
    };
    let Ok(mut stmt) = conn.prepare(
        "SELECT data FROM auth_credentials WHERE provider = ?1 AND credential_type = 'oauth'",
    ) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([omp_provider], |row| row.get::<_, String>(0)) else {
        return Vec::new();
    };
    rows.flatten()
        .filter_map(|data| parse_omp_credential(&data, base, profile))
        .collect()
}

/// Parse one Oh My Pi OAuth credential blob (`{ access, refresh, accountId, email, expires }`)
/// into a [`RawAccount`]. An `accountId` is **required** — it is the dedup key and the stable
/// source id, so rotated/legacy tokens without one are skipped (these accumulate in the store and
/// must not surface as phantom sources). `expires` is ms epoch; falls back to the JWT `exp`.
fn parse_omp_credential(data: &str, base: &'static str, profile: &str) -> Option<RawAccount> {
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
        base,
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

/// Read a trimmed, non-empty string field by key.
pub(crate) fn json_str(obj: &serde_json::Value, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

// ── Per-account credentials ──────────────────────────────────────────────────--

/// Credentials for one account, resolved by base + account id across every store (freshest token
/// wins). Re-reads on each load, so MLT always uses the latest token the vendor (Oh My Pi or the
/// CLI) has refreshed — and never writes either store.
pub struct AccountCredentials {
    base: &'static str,
    account_id: String,
}

impl AccountCredentials {
    pub fn new(base: &'static str, account_id: impl Into<String>) -> Self {
        Self {
            base,
            account_id: account_id.into(),
        }
    }
}

#[async_trait]
impl OAuthCredentialSource for AccountCredentials {
    async fn load(&self) -> Result<OAuthTokens, PortError> {
        dedup_freshest(enumerate_for(self.base))
            .into_iter()
            .find(|raw| raw.account_id == self.account_id)
            .map(|raw| raw.tokens)
            .ok_or(PortError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::MutexGuard;
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct EnvFixture {
        _guard: MutexGuard<'static, ()>,
        home: PathBuf,
        old_omp: Option<OsString>,
        old_codex: Option<OsString>,
    }

    impl EnvFixture {
        fn new(name: &str) -> Self {
            let guard = crate::TEST_ENV_LOCK.lock();
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time after epoch")
                .as_nanos();
            let home = std::env::temp_dir().join(format!(
                "mlt-accounts-{name}-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&home).expect("temp home");
            let codex_home = home.join("codex-home");
            fs::create_dir_all(&codex_home).expect("temp codex home");
            let old_omp = std::env::var_os("OMP_HOME");
            let old_codex = std::env::var_os("CODEX_HOME");
            std::env::set_var("OMP_HOME", &home);
            std::env::set_var("CODEX_HOME", codex_home);
            Self {
                _guard: guard,
                home,
                old_omp,
                old_codex,
            }
        }
    }

    impl Drop for EnvFixture {
        fn drop(&mut self) {
            match &self.old_omp {
                Some(value) => std::env::set_var("OMP_HOME", value),
                None => std::env::remove_var("OMP_HOME"),
            }
            match &self.old_codex {
                Some(value) => std::env::set_var("CODEX_HOME", value),
                None => std::env::remove_var("CODEX_HOME"),
            }
            let _ = fs::remove_dir_all(&self.home);
        }
    }

    fn create_omp_db(path: &Path, rows: &[(&str, &str, &str)]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("db parent");
        }
        let conn = Connection::open(path).expect("fixture db");
        conn.execute(
            "CREATE TABLE auth_credentials (
                provider TEXT NOT NULL,
                credential_type TEXT NOT NULL,
                data TEXT NOT NULL
            )",
            [],
        )
        .expect("schema");
        for (provider, kind, data) in rows {
            conn.execute(
                "INSERT INTO auth_credentials (provider, credential_type, data) VALUES (?1, ?2, ?3)",
                (provider, kind, data),
            )
            .expect("insert credential");
        }
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

    #[test]
    fn parses_an_omp_oauth_blob_for_any_provider() {
        let data = r#"{"access":"at","refresh":"rt","accountId":"acct-1","email":"a@x.com","expires":1781000000000}"#;
        let raw = parse_omp_credential(data, "claude-code", "work").expect("parse");
        assert_eq!(raw.base, "claude-code");
        assert_eq!(raw.account_id, "acct-1");
        assert_eq!(raw.email.as_deref(), Some("a@x.com"));
        assert_eq!(raw.origin, "Oh My Pi · work");
        assert_eq!(raw.expires_ms, 1_781_000_000_000);
        assert_eq!(raw.tokens.access_token, "at");
        assert_eq!(raw.tokens.account_id.as_deref(), Some("acct-1"));
    }

    #[test]
    fn parse_omp_credential_falls_back_to_jwt_expiry() {
        let jwt = "eyJhbGciOiJSUzI1NiJ9.eyJleHAiOjE4OTM0NTYwMDB9.sig";
        let data = format!(
            r#"{{"access":"{jwt}","refresh":"rt","accountId":"acct-1","email":"a@x.com"}}"#
        );
        let raw = parse_omp_credential(&data, "codex", "default").expect("parse");

        assert_eq!(raw.expires_ms, 1_893_456_000_000);
        assert_eq!(raw.tokens.expires_at, Some(Timestamp(1_893_456_000_000)));
    }

    #[test]
    fn omp_credential_requires_access_and_account() {
        assert!(parse_omp_credential(r#"{"refresh":"r"}"#, "codex", "p").is_none());
        assert!(parse_omp_credential(r#"{"access":"a"}"#, "codex", "p").is_none()); // no accountId
        assert!(parse_omp_credential("not json", "codex", "p").is_none());
    }

    #[test]
    fn dedup_keeps_the_freshest_token_per_account() {
        let mk = |acct: &str, exp: i64, at: &str| RawAccount {
            base: "codex",
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
    fn discovered_accounts_reads_omp_profile_dbs_and_keeps_freshest_token() {
        let fixture = EnvFixture::new("discovery");
        let old = r#"{"access":"old-access","refresh":"old-refresh","accountId":"acct-1","email":"old@example.com","expires":100}"#;
        let fresh = r#"{"access":"fresh-access","refresh":"fresh-refresh","accountId":"acct-1","email":"fresh@example.com","expires":500}"#;
        let ignored_kind =
            r#"{"access":"ignored","refresh":"r","accountId":"ignored","expires":900}"#;
        let missing_account = r#"{"access":"missing-account","refresh":"r","expires":900}"#;
        create_omp_db(
            &fixture.home.join("agent").join("agent.db"),
            &[
                ("openai-codex", "oauth", old),
                ("openai-codex", "api_key", ignored_kind),
                ("openai-codex", "oauth", missing_account),
                (
                    "anthropic",
                    "oauth",
                    r#"{"access":"claude","accountId":"claude-acct","expires":1}"#,
                ),
            ],
        );
        create_omp_db(
            &fixture
                .home
                .join("profiles")
                .join("work")
                .join("agent")
                .join("agent.db"),
            &[("openai-codex", "oauth", fresh)],
        );

        let accounts = discovered_accounts();
        let codex: Vec<_> = accounts
            .iter()
            .filter(|account| account.base.as_str() == "codex")
            .collect();

        assert_eq!(codex.len(), 1);
        assert_eq!(codex[0].account_id, "acct-1");
        assert_eq!(codex[0].email.as_deref(), Some("fresh@example.com"));
        assert_eq!(codex[0].origin, "Oh My Pi · work");

        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        let tokens = runtime
            .block_on(AccountCredentials::new("codex", "acct-1").load())
            .expect("fresh account credentials");
        assert_eq!(tokens.access_token, "fresh-access");
        assert_eq!(tokens.refresh_token.as_deref(), Some("fresh-refresh"));

        let missing = runtime.block_on(AccountCredentials::new("codex", "missing").load());
        assert!(matches!(missing, Err(PortError::NotFound)));
    }
}
