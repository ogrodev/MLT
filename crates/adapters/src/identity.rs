//! File-backed [`IdentityStore`]: the account identity (email/org) resolved from a provider,
//! cached per source as a small JSON map (`{ "<source-id>": { "email": …, "organization": … } }`)
//! in the app config dir. Identity is account-identifying *display* metadata — not a secret and
//! not consent — so, like consent and labels, it lives here as plain settings, never in the
//! keychain. Caching it lets us resolve a provider's identity once instead of on every poll
//! (sparing rate-limited usage endpoints). A missing entry means "not resolved yet".
use std::collections::BTreeMap;
use std::path::PathBuf;

use parking_lot::Mutex;

use mlt_core::domain::{AccountIdentity, ProviderId};
use mlt_core::ports::{IdentityStore, PortError};

/// In-memory identity map with write-through to a JSON file. The map is the runtime source of
/// truth; every change is persisted so a resolved identity survives a restart.
#[derive(Debug)]
pub struct FileIdentityStore {
    path: PathBuf,
    state: Mutex<BTreeMap<String, AccountIdentity>>,
}

impl FileIdentityStore {
    /// Load identities from `path`. Best-effort: a missing or unparseable file starts empty
    /// (each source re-resolves on its next fetch), so a corrupt file is never fatal.
    pub fn load(path: PathBuf) -> Self {
        let state = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<BTreeMap<String, AccountIdentity>>(&raw).ok())
            .unwrap_or_default();
        Self {
            path,
            state: Mutex::new(state),
        }
    }

    fn persist(&self, map: &BTreeMap<String, AccountIdentity>) -> Result<(), PortError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| PortError::Io(e.to_string()))?;
        }
        let json = serde_json::to_string_pretty(map).map_err(|e| PortError::Io(e.to_string()))?;
        std::fs::write(&self.path, json).map_err(|e| PortError::Io(e.to_string()))
    }
}

impl IdentityStore for FileIdentityStore {
    fn identity(&self, id: &ProviderId) -> Result<Option<AccountIdentity>, PortError> {
        Ok(self.state.lock().get(id.as_str()).cloned())
    }

    fn set_identity(&self, id: &ProviderId, identity: &AccountIdentity) -> Result<(), PortError> {
        let mut map = self.state.lock();
        let previous = map.insert(id.as_str().to_string(), identity.clone());
        if let Err(e) = self.persist(&map) {
            // Persist failed — undo the in-memory change so the runtime map never diverges
            // from disk.
            match previous {
                Some(prev) => {
                    map.insert(id.as_str().to_string(), prev);
                }
                None => {
                    map.remove(id.as_str());
                }
            }
            return Err(e);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "mlt-identity-test-{}-{}.json",
            std::process::id(),
            tag
        ));
        let _ = std::fs::remove_file(&p);
        p
    }

    fn pid(id: &str) -> ProviderId {
        ProviderId::new(id)
    }

    fn with_email(email: &str) -> AccountIdentity {
        AccountIdentity {
            email: Some(email.into()),
            organization: None,
        }
    }

    #[test]
    fn unknown_source_has_no_identity() {
        let store = FileIdentityStore::load(temp_path("unknown"));
        assert_eq!(store.identity(&pid("claude-code")).unwrap(), None);
    }

    #[test]
    fn an_identity_persists_across_a_reload() {
        let path = temp_path("persist");
        {
            let store = FileIdentityStore::load(path.clone());
            store
                .set_identity(&pid("claude-code"), &with_email("dev@example.com"))
                .unwrap();
        }
        // A fresh load reads the same identity back from disk — no re-fetch needed on restart.
        let reloaded = FileIdentityStore::load(path.clone());
        assert_eq!(
            reloaded
                .identity(&pid("claude-code"))
                .unwrap()
                .and_then(|a| a.email)
                .as_deref(),
            Some("dev@example.com")
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn identities_are_independent_per_source() {
        let store = FileIdentityStore::load(temp_path("independent"));
        store
            .set_identity(&pid("claude-code"), &with_email("a@example.com"))
            .unwrap();
        store
            .set_identity(&pid("openrouter"), &with_email("b@example.com"))
            .unwrap();
        assert_eq!(
            store
                .identity(&pid("claude-code"))
                .unwrap()
                .and_then(|a| a.email)
                .as_deref(),
            Some("a@example.com")
        );
        assert_eq!(
            store
                .identity(&pid("openrouter"))
                .unwrap()
                .and_then(|a| a.email)
                .as_deref(),
            Some("b@example.com")
        );
    }

    #[test]
    fn set_overwrites_the_previous_identity() {
        let store = FileIdentityStore::load(temp_path("overwrite"));
        store
            .set_identity(&pid("claude-code"), &with_email("old@example.com"))
            .unwrap();
        store
            .set_identity(
                &pid("claude-code"),
                &AccountIdentity {
                    email: Some("new@example.com".into()),
                    organization: Some("Acme".into()),
                },
            )
            .unwrap();
        let got = store.identity(&pid("claude-code")).unwrap().unwrap();
        assert_eq!(got.email.as_deref(), Some("new@example.com"));
        assert_eq!(got.organization.as_deref(), Some("Acme"));
    }
}
