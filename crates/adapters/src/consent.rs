//! File-backed [`ConsentStore`]: the user's per-source opt-in, persisted as a small JSON map
//! (`{ "<source-id>": true }`) in the app config dir. Consent is **not** a secret, so it lives
//! here as plain settings — never in the keychain (ADR 0012). A missing entry means the source
//! is **disabled**, so on a fresh install nothing is read until the user opts in.
use std::collections::BTreeMap;
use std::path::PathBuf;

use parking_lot::Mutex;

use mlt_core::domain::ProviderId;
use mlt_core::ports::{ConsentStore, PortError};

/// In-memory consent map with write-through to a JSON file. The map is the source of truth at
/// runtime (so reads on the refresh hot path never touch disk); every change is persisted so
/// the choice survives a restart.
#[derive(Debug)]
pub struct FileConsentStore {
    path: PathBuf,
    state: Mutex<BTreeMap<String, bool>>,
}

impl FileConsentStore {
    /// Load consent from `path`. Best-effort: a missing or unparseable file starts empty
    /// (every source opted-out), so a corrupt settings file can never accidentally enable a
    /// source — it fails closed.
    pub fn load(path: PathBuf) -> Self {
        let state = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<BTreeMap<String, bool>>(&raw).ok())
            .unwrap_or_default();
        Self {
            path,
            state: Mutex::new(state),
        }
    }

    fn persist(&self, map: &BTreeMap<String, bool>) -> Result<(), PortError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| PortError::Io(e.to_string()))?;
        }
        let json = serde_json::to_string_pretty(map).map_err(|e| PortError::Io(e.to_string()))?;
        std::fs::write(&self.path, json).map_err(|e| PortError::Io(e.to_string()))
    }
}

impl ConsentStore for FileConsentStore {
    fn is_enabled(&self, id: &ProviderId) -> Result<bool, PortError> {
        Ok(self.state.lock().get(id.as_str()).copied().unwrap_or(false))
    }

    fn set_enabled(&self, id: &ProviderId, enabled: bool) -> Result<(), PortError> {
        let mut map = self.state.lock();
        map.insert(id.as_str().to_string(), enabled);
        self.persist(&map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "mlt-consent-test-{}-{tag}/consent.json",
            std::process::id()
        ))
    }

    #[test]
    fn unknown_source_defaults_to_disabled() {
        let store = FileConsentStore::load(temp_path("default"));
        assert!(!store.is_enabled(&ProviderId::new("claude-code")).unwrap());
    }

    #[test]
    fn opt_in_persists_across_a_reload() {
        let path = temp_path("persist");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        let id = ProviderId::new("claude-code");

        let store = FileConsentStore::load(path.clone());
        store.set_enabled(&id, true).unwrap();
        // A fresh instance reading the same file (i.e. an app restart) still sees the opt-in.
        let reloaded = FileConsentStore::load(path.clone());
        assert!(
            reloaded.is_enabled(&id).unwrap(),
            "consent survives restart"
        );

        // Opting back out is likewise persisted.
        reloaded.set_enabled(&id, false).unwrap();
        assert!(!FileConsentStore::load(path.clone())
            .is_enabled(&id)
            .unwrap());

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn per_source_toggles_are_independent() {
        let path = temp_path("independent");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        let store = FileConsentStore::load(path.clone());

        store.set_enabled(&ProviderId::new("a"), true).unwrap();
        store.set_enabled(&ProviderId::new("b"), false).unwrap();
        assert!(store.is_enabled(&ProviderId::new("a")).unwrap());
        assert!(!store.is_enabled(&ProviderId::new("b")).unwrap());

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
