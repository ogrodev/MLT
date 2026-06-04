//! File-backed [`AlarmStore`]: user alarms and alarm settings/state persisted as small JSON
//! files in the app config dir. Alarm settings are plain preferences/state, not secrets.
use std::path::PathBuf;

use async_trait::async_trait;
use parking_lot::Mutex;

use mlt_core::alarms::{Alarm, AlarmId, AlarmSettings};
use mlt_core::ports::{AlarmStore, PortError};

/// In-memory alarm caches with write-through to JSON files. The caches are the runtime source of
/// truth; every change is persisted so alarms and derived notification state survive a restart.
#[derive(Debug)]
pub struct FileAlarmStore {
    alarms_path: PathBuf,
    settings_path: PathBuf,
    alarms: Mutex<Vec<Alarm>>,
    settings: Mutex<AlarmSettings>,
}

impl FileAlarmStore {
    /// Load alarms and settings from disk. Best-effort: missing or unparseable files start from an
    /// empty alarm list and default settings, so corrupt settings are never fatal.
    pub fn load(alarms_path: PathBuf, settings_path: PathBuf) -> Self {
        let alarms = std::fs::read_to_string(&alarms_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Vec<Alarm>>(&raw).ok())
            .unwrap_or_default();
        let settings = std::fs::read_to_string(&settings_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<AlarmSettings>(&raw).ok())
            .unwrap_or_default();
        Self {
            alarms_path,
            settings_path,
            alarms: Mutex::new(alarms),
            settings: Mutex::new(settings),
        }
    }

    fn persist_alarms(&self, alarms: &[Alarm]) -> Result<(), PortError> {
        if let Some(parent) = self.alarms_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| PortError::Io(e.to_string()))?;
        }
        let json =
            serde_json::to_string_pretty(alarms).map_err(|e| PortError::Io(e.to_string()))?;
        std::fs::write(&self.alarms_path, json).map_err(|e| PortError::Io(e.to_string()))
    }

    fn persist_settings(&self, settings: &AlarmSettings) -> Result<(), PortError> {
        if let Some(parent) = self.settings_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| PortError::Io(e.to_string()))?;
        }
        let json =
            serde_json::to_string_pretty(settings).map_err(|e| PortError::Io(e.to_string()))?;
        std::fs::write(&self.settings_path, json).map_err(|e| PortError::Io(e.to_string()))
    }
}

#[async_trait]
impl AlarmStore for FileAlarmStore {
    async fn alarms(&self) -> Result<Vec<Alarm>, PortError> {
        Ok(self.alarms.lock().clone())
    }

    async fn upsert_alarm(&self, alarm: &Alarm) -> Result<(), PortError> {
        let mut alarms = self.alarms.lock();
        let previous = alarms.clone();
        if let Some(existing) = alarms
            .iter_mut()
            .find(|entry| entry.id.as_str() == alarm.id.as_str())
        {
            *existing = alarm.clone();
        } else {
            alarms.push(alarm.clone());
        }
        if let Err(e) = self.persist_alarms(alarms.as_slice()) {
            *alarms = previous;
            return Err(e);
        }
        Ok(())
    }

    async fn delete_alarm(&self, id: &AlarmId) -> Result<(), PortError> {
        let mut alarms = self.alarms.lock();
        let previous = alarms.clone();
        alarms.retain(|alarm| alarm.id.as_str() != id.as_str());
        if let Err(e) = self.persist_alarms(alarms.as_slice()) {
            *alarms = previous;
            return Err(e);
        }
        Ok(())
    }

    async fn settings(&self) -> Result<AlarmSettings, PortError> {
        Ok(self.settings.lock().clone())
    }

    async fn save_settings(&self, settings: &AlarmSettings) -> Result<(), PortError> {
        let mut state = self.settings.lock();
        let previous = state.clone();
        *state = settings.clone();
        if let Err(e) = self.persist_settings(&state) {
            *state = previous;
            return Err(e);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mlt_core::alarms::{MissedPolicy, Recurrence};
    use mlt_core::domain::{ProviderId, Timestamp, WindowKind};

    fn temp_path(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "mlt-alarms-test-{}-{}.json",
            std::process::id(),
            tag
        ));
        let _ = std::fs::remove_file(&p);
        p
    }

    fn alarm(id: &str, label: &str, next_fire_at: i64) -> Alarm {
        Alarm {
            id: AlarmId::new(id),
            label: label.to_string(),
            next_fire_at: Timestamp(next_fire_at),
            recurrence: Some(Recurrence::Daily),
        }
    }

    #[tokio::test]
    async fn upsert_persists_across_a_reload() {
        let alarms_path = temp_path("upsert-persists-alarms");
        let settings_path = temp_path("upsert-persists-settings");
        let expected = alarm("standup", "Standup", 1_700_000_000_000);

        let store = FileAlarmStore::load(alarms_path.clone(), settings_path.clone());
        store.upsert_alarm(&expected).await.unwrap();

        let reloaded = FileAlarmStore::load(alarms_path.clone(), settings_path.clone());
        assert_eq!(reloaded.alarms().await.unwrap(), vec![expected]);

        let _ = std::fs::remove_file(&alarms_path);
        let _ = std::fs::remove_file(&settings_path);
    }

    #[tokio::test]
    async fn upsert_replaces_by_id_without_duplicating() {
        let alarms_path = temp_path("upsert-replaces-alarms");
        let settings_path = temp_path("upsert-replaces-settings");
        let first = alarm("standup", "Standup", 1_700_000_000_000);
        let replacement = Alarm {
            id: first.id.clone(),
            label: "Updated standup".to_string(),
            next_fire_at: Timestamp(1_700_086_400_000),
            recurrence: None,
        };

        let store = FileAlarmStore::load(alarms_path.clone(), settings_path.clone());
        store.upsert_alarm(&first).await.unwrap();
        store.upsert_alarm(&replacement).await.unwrap();

        assert_eq!(store.alarms().await.unwrap(), vec![replacement]);

        let _ = std::fs::remove_file(&alarms_path);
        let _ = std::fs::remove_file(&settings_path);
    }

    #[tokio::test]
    async fn delete_removes_alarm() {
        let alarms_path = temp_path("delete-removes-alarms");
        let settings_path = temp_path("delete-removes-settings");
        let deleted = alarm("standup", "Standup", 1_700_000_000_000);
        let kept = alarm("review", "Review", 1_700_000_060_000);

        let store = FileAlarmStore::load(alarms_path.clone(), settings_path.clone());
        store.upsert_alarm(&deleted).await.unwrap();
        store.upsert_alarm(&kept).await.unwrap();
        store.delete_alarm(&deleted.id).await.unwrap();

        assert_eq!(store.alarms().await.unwrap(), vec![kept]);

        let _ = std::fs::remove_file(&alarms_path);
        let _ = std::fs::remove_file(&settings_path);
    }

    #[tokio::test]
    async fn deleting_an_absent_alarm_is_idempotent() {
        let alarms_path = temp_path("delete-absent-alarms");
        let settings_path = temp_path("delete-absent-settings");
        let existing = alarm("standup", "Standup", 1_700_000_000_000);

        let store = FileAlarmStore::load(alarms_path.clone(), settings_path.clone());
        store.upsert_alarm(&existing).await.unwrap();
        store
            .delete_alarm(&AlarmId::new("does-not-exist"))
            .await
            .unwrap();

        assert_eq!(store.alarms().await.unwrap(), vec![existing]);

        let _ = std::fs::remove_file(&alarms_path);
        let _ = std::fs::remove_file(&settings_path);
    }

    #[tokio::test]
    async fn settings_persist_across_a_reload() {
        let alarms_path = temp_path("settings-persists-alarms");
        let settings_path = temp_path("settings-persists-settings");
        let provider = ProviderId::new("claude-code");
        let mut expected = AlarmSettings {
            missed_policy: MissedPolicy::Coalesce,
            ..AlarmSettings::default()
        };
        expected.set_reset_enabled(&provider, WindowKind::Weekly, true);

        let store = FileAlarmStore::load(alarms_path.clone(), settings_path.clone());
        store.save_settings(&expected).await.unwrap();

        let reloaded = FileAlarmStore::load(alarms_path.clone(), settings_path.clone());
        assert_eq!(reloaded.settings().await.unwrap(), expected);

        let _ = std::fs::remove_file(&alarms_path);
        let _ = std::fs::remove_file(&settings_path);
    }

    #[tokio::test]
    async fn corrupt_or_missing_files_load_as_empty_and_default() {
        let corrupt_alarms_path = temp_path("corrupt-alarms");
        let corrupt_settings_path = temp_path("corrupt-settings");
        std::fs::write(&corrupt_alarms_path, "not-json").unwrap();
        std::fs::write(&corrupt_settings_path, "not-json").unwrap();

        let corrupt =
            FileAlarmStore::load(corrupt_alarms_path.clone(), corrupt_settings_path.clone());
        assert_eq!(corrupt.alarms().await.unwrap(), Vec::<Alarm>::new());
        assert_eq!(corrupt.settings().await.unwrap(), AlarmSettings::default());

        let missing_alarms_path = temp_path("missing-alarms");
        let missing_settings_path = temp_path("missing-settings");
        let missing =
            FileAlarmStore::load(missing_alarms_path.clone(), missing_settings_path.clone());
        assert_eq!(missing.alarms().await.unwrap(), Vec::<Alarm>::new());
        assert_eq!(missing.settings().await.unwrap(), AlarmSettings::default());

        let _ = std::fs::remove_file(&corrupt_alarms_path);
        let _ = std::fs::remove_file(&corrupt_settings_path);
        let _ = std::fs::remove_file(&missing_alarms_path);
        let _ = std::fs::remove_file(&missing_settings_path);
    }

    #[tokio::test]
    async fn failed_alarm_persist_rolls_back_in_memory_change() {
        let dir = std::env::temp_dir().join(format!(
            "mlt-alarms-test-{}-alarm-rollback",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, "x").unwrap();
        let store = FileAlarmStore::load(
            blocker.join("alarms.json"),
            temp_path("alarm-rollback-settings"),
        );

        assert!(store
            .upsert_alarm(&alarm("standup", "Standup", 1_700_000_000_000))
            .await
            .is_err());
        assert_eq!(store.alarms().await.unwrap(), Vec::<Alarm>::new());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn failed_settings_persist_rolls_back_in_memory_change() {
        let dir = std::env::temp_dir().join(format!(
            "mlt-alarms-test-{}-settings-rollback",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, "x").unwrap();
        let store = FileAlarmStore::load(
            temp_path("settings-rollback-alarms"),
            blocker.join("settings.json"),
        );
        let settings = AlarmSettings {
            missed_policy: MissedPolicy::Coalesce,
            ..AlarmSettings::default()
        };

        assert!(store.save_settings(&settings).await.is_err());
        assert_eq!(store.settings().await.unwrap(), AlarmSettings::default());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
