//! Claude Code credential adapter + provider wiring.
//!
//! Reuses the Claude Code CLI's existing OAuth login (ADR 0012, metadata-only discovery →
//! per-source opt-in applies at the app layer). Order: the plain file first, then the macOS
//! Keychain (which may prompt the user the first time).
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;

use mlt_core::domain::{OAuthTokens, Timestamp};
use mlt_core::ports::{Clock, HttpPort, OAuthCredentialSource, PortError, SecretStore};
use mlt_core::providers::claude::{ClaudeCodeStrategy, ClaudeOAuthRefresher};

use crate::{KeyringSecretStore, ReqwestHttp, SystemClock, KEYCHAIN_SERVICE};

/// Reads Claude Code's OAuth tokens from `~/.claude/.credentials.json`, falling back to the
/// macOS Keychain item `Claude Code-credentials`.
#[derive(Debug, Default, Clone, Copy)]
pub struct ClaudeCredentials;

impl ClaudeCredentials {
    /// Metadata-only presence check (ADR 0012): does a Claude Code login *exist* on this
    /// machine? Checks the credentials file's existence and the Keychain item's presence — it
    /// never opens, decrypts, or parses the token. This is the discovery path: the secret is
    /// only read later via [`OAuthCredentialSource::load`], after the user opts the source in.
    pub fn is_present() -> bool {
        file_present() || keychain_present()
    }
}

/// Claude Code's plaintext credentials file under a given home dir. Split out so presence can
/// be unit-tested against a temp home without reading anything.
fn credentials_path(home: &Path) -> PathBuf {
    home.join(".claude/.credentials.json")
}

/// Does the credentials file *exist*? `is_file` is a stat — it never opens the file, so even a
/// present-but-garbage file counts as "a login exists here", which is exactly what proves
/// discovery is decoupled from reading the secret.
fn path_present(home: &Path) -> bool {
    credentials_path(home).is_file()
}

fn file_present() -> bool {
    dirs::home_dir().map(|h| path_present(&h)).unwrap_or(false)
}

#[async_trait]
impl OAuthCredentialSource for ClaudeCredentials {
    async fn load(&self) -> Result<OAuthTokens, PortError> {
        // 1) Plain file (Linux / older macOS setups).
        if let Some(home) = dirs::home_dir() {
            let path = home.join(".claude/.credentials.json");
            if let Ok(raw) = std::fs::read_to_string(&path) {
                if let Ok(tokens) = parse_creds(&raw) {
                    return Ok(tokens);
                }
            }
        }
        // 2) macOS Keychain (the common case on macOS; may prompt once).
        if let Some(raw) = read_keychain() {
            return parse_creds(&raw).map_err(PortError::Io);
        }
        Err(PortError::NotFound)
    }
}

/// Parse the credential blob. Accepts both the wrapped (`{ "claudeAiOauth": { … } }`) and
/// bare object shapes, and snake_case/camelCase keys.
fn parse_creds(raw: &str) -> Result<OAuthTokens, String> {
    let value: serde_json::Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    let o = value.get("claudeAiOauth").unwrap_or(&value);

    let access_token = o
        .get("accessToken")
        .or_else(|| o.get("access_token"))
        .and_then(|x| x.as_str())
        .ok_or("no accessToken in Claude credentials")?
        .to_string();
    let refresh_token = o
        .get("refreshToken")
        .or_else(|| o.get("refresh_token"))
        .and_then(|x| x.as_str())
        .map(String::from);
    let expires_at = o
        .get("expiresAt")
        .or_else(|| o.get("expires_at"))
        .and_then(|x| x.as_i64())
        .map(Timestamp);
    let scopes = o
        .get("scopes")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let subscription_type = o
        .get("subscriptionType")
        .or_else(|| o.get("subscription_type"))
        .and_then(|x| x.as_str())
        .map(String::from);

    Ok(OAuthTokens {
        access_token,
        refresh_token,
        expires_at,
        scopes,
        subscription_type,
    })
}

#[cfg(target_os = "macos")]
fn read_keychain() -> Option<String> {
    let out = std::process::Command::new("/usr/bin/security")
        .args([
            "find-generic-password",
            "-s",
            "Claude Code-credentials",
            "-w",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

#[cfg(not(target_os = "macos"))]
fn read_keychain() -> Option<String> {
    None
}

/// Does the macOS Keychain hold Claude Code's credential item? Runs `find-generic-password`
/// **without `-w`**, so it returns only the item's attributes (exit 0 if present) and never the
/// password — presence only, no secret read (ADR 0012). Contrast [`read_keychain`], which
/// passes `-w` to read the token and runs only on the consented `load` path.
#[cfg(target_os = "macos")]
fn keychain_present() -> bool {
    std::process::Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", "Claude Code-credentials"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(not(target_os = "macos"))]
fn keychain_present() -> bool {
    false
}

/// Best-effort detection of the installed Claude Code CLI version for the required
/// `User-Agent: claude-code/<version>` header (without it, the endpoint 429s hard).
pub fn detect_user_agent() -> String {
    let version = std::process::Command::new("claude")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.split_whitespace().next().map(String::from))
        .unwrap_or_else(|| "unknown".into());
    format!("claude-code/{version}")
}

/// Build a ready-to-run Claude Code strategy wired with the real adapters. Credentials flow
/// through the refresher: it reuses Claude Code's live token when fresh and only refreshes
/// (caching into OUR keychain, never Claude Code's) when that token has expired.
pub fn claude_strategy() -> ClaudeCodeStrategy {
    let http: Arc<dyn HttpPort> = Arc::new(ReqwestHttp::new());
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let bootstrap: Arc<dyn OAuthCredentialSource> = Arc::new(ClaudeCredentials);
    let cache: Arc<dyn SecretStore> = Arc::new(KeyringSecretStore::new(KEYCHAIN_SERVICE));
    let creds: Arc<dyn OAuthCredentialSource> = Arc::new(ClaudeOAuthRefresher::new(
        bootstrap,
        cache,
        http.clone(),
        clock.clone(),
    ));
    ClaudeCodeStrategy {
        creds,
        http,
        clock,
        user_agent: detect_user_agent(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wrapped_credentials_shape() {
        let raw = r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat-x","refreshToken":"sk-ant-ort-y",
            "expiresAt":1780234362680,"scopes":["user:inference","user:profile"],
            "subscriptionType":"team"}}"#;
        let t = parse_creds(raw).unwrap();
        assert_eq!(t.access_token, "sk-ant-oat-x");
        assert_eq!(t.refresh_token.as_deref(), Some("sk-ant-ort-y"));
        assert_eq!(t.expires_at, Some(Timestamp(1780234362680)));
        assert!(t.scopes.iter().any(|s| s == "user:profile"));
        assert_eq!(t.subscription_type.as_deref(), Some("team"));
    }

    #[test]
    fn detect_user_agent_has_the_claude_code_prefix() {
        assert!(detect_user_agent().starts_with("claude-code/"));
    }

    #[test]
    fn presence_is_metadata_only_and_never_reads_the_secret() {
        let base = std::env::temp_dir().join(format!("mlt-presence-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);

        // No credentials file ⇒ not present.
        assert!(
            !path_present(&base),
            "absent credentials file ⇒ not present"
        );

        std::fs::create_dir_all(base.join(".claude")).unwrap();
        // Deliberately INVALID content: discovery reports "present" from existence alone,
        // while the real secret path (`parse_creds`) rejects it — proof the probe never reads
        // or parses the credential it discovered.
        std::fs::write(credentials_path(&base), "not a real credential").unwrap();
        assert!(path_present(&base), "existing file ⇒ present (stat only)");
        assert!(
            parse_creds("not a real credential").is_err(),
            "the secret-reading path would reject this content"
        );

        let _ = std::fs::remove_dir_all(&base);
    }
}
