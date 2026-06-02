//! File-backed [`SourceLabels`]: an optional, user-assigned custom name per source, persisted
//! as a small JSON map (`{ "<source-id>": "<name>" }`) in the app config dir. A label is a
//! non-secret UI preference (a custom title the user types — never an email; the account email
//! is auto-fetched separately), so — like consent — it lives here as plain settings, never in
//! the keychain (ADR 0012). A missing entry means the source shows its default name.
use std::collections::BTreeMap;
use std::path::PathBuf;

use parking_lot::Mutex;

use mlt_core::domain::ProviderId;
use mlt_core::ports::{PortError, SourceLabels};

/// In-memory label map with write-through to a JSON file. The map is the runtime source of
/// truth; every change is persisted so the name survives a restart.
#[derive(Debug)]
pub struct FileLabelStore {
    path: PathBuf,
    state: Mutex<BTreeMap<String, String>>,
}

impl FileLabelStore {
    /// Load labels from `path`. Best-effort: a missing or unparseable file starts empty (every
    /// source shows its default name), so a corrupt settings file is never fatal.
    pub fn load(path: PathBuf) -> Self {
        let state = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<BTreeMap<String, String>>(&raw).ok())
            .unwrap_or_default();
        Self {
            path,
            state: Mutex::new(state),
        }
    }

    fn persist(&self, map: &BTreeMap<String, String>) -> Result<(), PortError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| PortError::Io(e.to_string()))?;
        }
        let json = serde_json::to_string_pretty(map).map_err(|e| PortError::Io(e.to_string()))?;
        std::fs::write(&self.path, json).map_err(|e| PortError::Io(e.to_string()))
    }
}

impl SourceLabels for FileLabelStore {
    fn label(&self, id: &ProviderId) -> Result<Option<String>, PortError> {
        Ok(self.state.lock().get(id.as_str()).cloned())
    }

    fn set_label(&self, id: &ProviderId, label: Option<&str>) -> Result<(), PortError> {
        // Treat a blank name as "clear" so the source falls back to its default.
        let name = label.map(str::trim).filter(|s| !s.is_empty());
        let mut map = self.state.lock();
        let previous = match name {
            Some(name) => map.insert(id.as_str().to_string(), name.to_string()),
            None => map.remove(id.as_str()),
        };
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
            "mlt-labels-test-{}-{}.json",
            std::process::id(),
            tag
        ));
        let _ = std::fs::remove_file(&p);
        p
    }

    fn pid(id: &str) -> ProviderId {
        ProviderId::new(id)
    }

    #[test]
    fn unknown_source_has_no_label() {
        let store = FileLabelStore::load(temp_path("unknown"));
        assert_eq!(store.label(&pid("openrouter")).unwrap(), None);
    }

    #[test]
    fn a_name_persists_across_a_reload() {
        let path = temp_path("persist");
        {
            let store = FileLabelStore::load(path.clone());
            store
                .set_label(&pid("openrouter"), Some("work@example.com"))
                .unwrap();
        }
        // A fresh load reads the same name back from disk.
        let reloaded = FileLabelStore::load(path.clone());
        assert_eq!(
            reloaded.label(&pid("openrouter")).unwrap().as_deref(),
            Some("work@example.com")
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_name_is_trimmed_and_a_blank_name_clears_it() {
        let store = FileLabelStore::load(temp_path("trim"));
        store
            .set_label(&pid("openrouter"), Some("  My Key  "))
            .unwrap();
        assert_eq!(
            store.label(&pid("openrouter")).unwrap().as_deref(),
            Some("My Key")
        );
        // A blank (whitespace-only) name removes the label rather than storing an empty string.
        store.set_label(&pid("openrouter"), Some("   ")).unwrap();
        assert_eq!(store.label(&pid("openrouter")).unwrap(), None);
        // …as does an explicit clear.
        store.set_label(&pid("openrouter"), Some("name")).unwrap();
        store.set_label(&pid("openrouter"), None).unwrap();
        assert_eq!(store.label(&pid("openrouter")).unwrap(), None);
    }

    #[test]
    fn names_are_independent_per_source() {
        let store = FileLabelStore::load(temp_path("independent"));
        store.set_label(&pid("openrouter"), Some("A")).unwrap();
        store.set_label(&pid("claude-code"), Some("B")).unwrap();
        assert_eq!(
            store.label(&pid("openrouter")).unwrap().as_deref(),
            Some("A")
        );
        assert_eq!(
            store.label(&pid("claude-code")).unwrap().as_deref(),
            Some("B")
        );
    }
}
