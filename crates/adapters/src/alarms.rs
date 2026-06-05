//! File-backed [`AlarmStore`]: user alarms and alarm settings/state persisted as small JSON
//! files in the app config dir. Alarm settings are plain preferences/state, not secrets.
use std::io::Write;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use parking_lot::Mutex;

use mlt_core::alarms::{
    self as core_alarms, Alarm, AlarmId, AlarmNotice, AlarmSettings, MissedPolicy, Reconciliation,
};
use mlt_core::domain::{Timestamp, UsageSnapshot};
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
    /// Load alarms and settings from disk. Missing files start empty/default. A file that
    /// exists but fails to parse is moved aside (`.corrupt`) rather than silently defaulted,
    /// so a single bad entry never silently discards every alarm — the data survives for
    /// recovery and the next write does not overwrite it.
    pub fn load(alarms_path: PathBuf, settings_path: PathBuf) -> Self {
        let alarms =
            load_or_quarantine(&alarms_path, |raw| serde_json::from_str::<Vec<Alarm>>(raw));
        let settings = load_or_quarantine(&settings_path, |raw| {
            serde_json::from_str::<AlarmSettings>(raw)
        });
        Self {
            alarms_path,
            settings_path,
            alarms: Mutex::new(alarms),
            settings: Mutex::new(settings),
        }
    }

    fn persist_alarms(&self, alarms: &[Alarm]) -> Result<(), PortError> {
        let json =
            serde_json::to_string_pretty(alarms).map_err(|e| PortError::Io(e.to_string()))?;
        atomic_write(&self.alarms_path, &json)
    }

    fn persist_settings(&self, settings: &AlarmSettings) -> Result<(), PortError> {
        let json =
            serde_json::to_string_pretty(settings).map_err(|e| PortError::Io(e.to_string()))?;
        atomic_write(&self.settings_path, &json)
    }
}

/// Read and parse `path`, quarantining (not overwriting) a corrupt file. Missing/unreadable
/// files yield the default; a parse failure renames the file aside so it can be recovered.
fn load_or_quarantine<T: Default>(
    path: &Path,
    parse: impl FnOnce(&str) -> Result<T, serde_json::Error>,
) -> T {
    // Read raw bytes and decode lossily (ADR 0015): a present-but-non-UTF8 file must still reach
    // the parse failure below and be quarantined, not skip straight to the default — otherwise the
    // next persist would overwrite and lose the original bytes. Missing/unreadable files default.
    let Ok(bytes) = std::fs::read(path) else {
        return T::default();
    };
    let raw = String::from_utf8_lossy(&bytes);
    match parse(&raw) {
        Ok(value) => value,
        Err(error) => {
            quarantine(path, &error);
            T::default()
        }
    }
}

fn quarantine(path: &Path, error: &serde_json::Error) {
    let mut corrupt = path.as_os_str().to_owned();
    corrupt.push(".corrupt");
    let corrupt = PathBuf::from(corrupt);
    if std::fs::rename(path, &corrupt).is_ok() {
        eprintln!(
            "mlt: could not parse {} ({error}); moved aside to {} and started fresh",
            path.display(),
            corrupt.display()
        );
    } else {
        eprintln!(
            "mlt: could not parse {} ({error}) and could not quarantine it; started fresh",
            path.display()
        );
    }
}

/// Write `contents` durably and atomically: write + fsync a temp file, then rename it over
/// the target. A crash mid-write leaves either the old bytes or the new ones — never a
/// truncated/partial file that would fail to parse and lose every alarm.
fn atomic_write(path: &Path, contents: &str) -> Result<(), PortError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| PortError::Io(e.to_string()))?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    {
        let mut file = std::fs::File::create(&tmp).map_err(|e| PortError::Io(e.to_string()))?;
        file.write_all(contents.as_bytes())
            .map_err(|e| PortError::Io(e.to_string()))?;
        file.sync_all().map_err(|e| PortError::Io(e.to_string()))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        PortError::Io(e.to_string())
    })?;
    // Best-effort: fsync the parent directory so the rename (the directory-entry update) is itself
    // crash-durable, completing the persist-before-notify guarantee — the temp file's sync_all()
    // above commits only the file data, not the rename. Skipped where directory fsync is
    // unsupported; the data is written regardless.
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
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

    async fn reconcile_due(
        &self,
        now: Timestamp,
        policy: MissedPolicy,
    ) -> Result<Reconciliation, PortError> {
        let mut alarms = self.alarms.lock();
        let reconciliation = core_alarms::reconcile(now, &alarms, policy);
        if reconciliation.updated.is_empty() && reconciliation.completed.is_empty() {
            // Nothing came due: don't rewrite the file on an idle scheduler tick.
            return Ok(reconciliation);
        }
        let previous = alarms.clone();
        apply_reconciliation(&mut alarms, &reconciliation);
        if let Err(e) = self.persist_alarms(alarms.as_slice()) {
            *alarms = previous;
            return Err(e);
        }
        Ok(reconciliation)
    }

    async fn settings(&self) -> Result<AlarmSettings, PortError> {
        Ok(self.settings.lock().clone())
    }

    async fn update_settings(
        &self,
        update: Box<dyn FnOnce(AlarmSettings) -> AlarmSettings + Send>,
    ) -> Result<AlarmSettings, PortError> {
        let mut settings = self.settings.lock();
        let previous = settings.clone();
        *settings = update(std::mem::take(&mut *settings));
        if let Err(e) = self.persist_settings(&settings) {
            *settings = previous;
            return Err(e);
        }
        Ok(settings.clone())
    }

    async fn evaluate_usage(
        &self,
        snapshot: &UsageSnapshot,
    ) -> Result<Vec<AlarmNotice>, PortError> {
        let mut settings = self.settings.lock();
        let previous = settings.clone();
        let notices = core_alarms::evaluate_snapshot(snapshot, &mut settings);
        if *settings == previous {
            // Steady state already recorded: nothing to persist this poll.
            return Ok(notices);
        }
        if let Err(e) = self.persist_settings(&settings) {
            *settings = previous;
            return Err(e);
        }
        Ok(notices)
    }
}

/// Apply a reconciliation to the in-memory list: replace advanced recurring alarms in place
/// (by id) and drop completed one-offs.
fn apply_reconciliation(alarms: &mut Vec<Alarm>, reconciliation: &Reconciliation) {
    for advanced in &reconciliation.updated {
        if let Some(slot) = alarms.iter_mut().find(|alarm| alarm.id == advanced.id) {
            *slot = advanced.clone();
        }
    }
    alarms.retain(|alarm| !reconciliation.completed.contains(&alarm.id));
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

    fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
        let mut p = path.as_os_str().to_owned();
        p.push(suffix);
        PathBuf::from(p)
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
        let store = FileAlarmStore::load(alarms_path.clone(), settings_path.clone());
        let saved = store
            .update_settings(Box::new(move |mut settings| {
                settings.missed_policy = MissedPolicy::Coalesce;
                settings.set_reset_enabled(&provider, WindowKind::Weekly, None, true);
                settings
            }))
            .await
            .unwrap();

        let reloaded = FileAlarmStore::load(alarms_path.clone(), settings_path.clone());
        assert_eq!(reloaded.settings().await.unwrap(), saved);

        let _ = std::fs::remove_file(&alarms_path);
        let _ = std::fs::remove_file(&settings_path);
    }

    #[tokio::test]
    async fn a_corrupt_file_is_quarantined_not_silently_overwritten() {
        let corrupt_alarms_path = temp_path("corrupt-alarms");
        let corrupt_settings_path = temp_path("corrupt-settings");
        // Valid JSON of the wrong shape: parses as JSON, fails as `Vec<Alarm>` — the "one bad
        // entry drops every alarm" case. It must be preserved, not silently defaulted away.
        std::fs::write(&corrupt_alarms_path, "{\"not\":\"an array\"}").unwrap();
        std::fs::write(&corrupt_settings_path, "not-json").unwrap();

        let store =
            FileAlarmStore::load(corrupt_alarms_path.clone(), corrupt_settings_path.clone());
        assert_eq!(store.alarms().await.unwrap(), Vec::<Alarm>::new());
        assert_eq!(store.settings().await.unwrap(), AlarmSettings::default());

        // The bad file was moved aside (recoverable), not left in place to be overwritten.
        let alarms_corrupt = with_suffix(&corrupt_alarms_path, ".corrupt");
        let settings_corrupt = with_suffix(&corrupt_settings_path, ".corrupt");
        assert!(alarms_corrupt.exists());
        assert!(!corrupt_alarms_path.exists());

        // A subsequent write lands cleanly at the original path.
        store
            .upsert_alarm(&alarm("standup", "Standup", 1_700_000_000_000))
            .await
            .unwrap();
        assert!(corrupt_alarms_path.exists());

        for p in [
            &corrupt_alarms_path,
            &alarms_corrupt,
            &corrupt_settings_path,
            &settings_corrupt,
        ] {
            let _ = std::fs::remove_file(p);
        }
    }

    #[tokio::test]
    async fn a_non_utf8_file_is_quarantined_not_silently_overwritten() {
        // Invalid UTF-8 bytes: `read_to_string` would have errored and skipped quarantine
        // entirely, letting the next persist overwrite (lose) the file. Lossy decode routes it to
        // the parse failure and quarantine instead.
        let alarms_path = temp_path("non-utf8-alarms");
        let settings_path = temp_path("non-utf8-settings"); // missing -> default
        std::fs::write(&alarms_path, [0xffu8, 0xfe, 0x00, 0x80]).unwrap();

        let store = FileAlarmStore::load(alarms_path.clone(), settings_path.clone());
        assert_eq!(store.alarms().await.unwrap(), Vec::<Alarm>::new());

        // The non-UTF8 file was moved aside (recoverable), not left in place to be overwritten.
        let alarms_corrupt = with_suffix(&alarms_path, ".corrupt");
        assert!(alarms_corrupt.exists(), "non-UTF8 file must be quarantined");
        assert!(!alarms_path.exists());

        let _ = std::fs::remove_file(&alarms_corrupt);
        let _ = std::fs::remove_file(&settings_path);
    }

    #[tokio::test]
    async fn missing_files_load_as_empty_and_default() {
        let missing_alarms_path = temp_path("missing-alarms");
        let missing_settings_path = temp_path("missing-settings");
        let missing =
            FileAlarmStore::load(missing_alarms_path.clone(), missing_settings_path.clone());
        assert_eq!(missing.alarms().await.unwrap(), Vec::<Alarm>::new());
        assert_eq!(missing.settings().await.unwrap(), AlarmSettings::default());

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
        let result = store
            .update_settings(Box::new(|mut settings| {
                settings.missed_policy = MissedPolicy::Coalesce;
                settings
            }))
            .await;
        assert!(result.is_err());
        assert_eq!(store.settings().await.unwrap(), AlarmSettings::default());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn reconcile_due_advances_recurring_alarms_and_persists() {
        let alarms_path = temp_path("reconcile-due-alarms");
        let settings_path = temp_path("reconcile-due-settings");
        let store = FileAlarmStore::load(alarms_path.clone(), settings_path.clone());
        // `alarm` builds a Daily recurring alarm; due at t=1000.
        store
            .upsert_alarm(&alarm("standup", "Standup", 1_000))
            .await
            .unwrap();

        let reconciliation = store
            .reconcile_due(Timestamp(1_000), MissedPolicy::FireEach)
            .await
            .unwrap();
        assert_eq!(reconciliation.updated.len(), 1);
        assert!(reconciliation.completed.is_empty());
        assert!(reconciliation.updated[0].next_fire_at.0 > 1_000);

        // The advanced fire time is persisted across a reload.
        let reloaded = FileAlarmStore::load(alarms_path.clone(), settings_path.clone());
        assert_eq!(reloaded.alarms().await.unwrap(), reconciliation.updated);

        let _ = std::fs::remove_file(&alarms_path);
        let _ = std::fs::remove_file(&settings_path);
    }

    #[tokio::test]
    async fn reconcile_due_skips_the_write_when_nothing_is_due() {
        let alarms_path = temp_path("reconcile-idle-alarms");
        let settings_path = temp_path("reconcile-idle-settings");
        let store = FileAlarmStore::load(alarms_path.clone(), settings_path.clone());
        store
            .upsert_alarm(&alarm("future", "Future", 9_000_000_000_000))
            .await
            .unwrap();
        // Remove the persisted file; an idle reconcile must not recreate it.
        std::fs::remove_file(&alarms_path).unwrap();

        let reconciliation = store
            .reconcile_due(Timestamp(1_000), MissedPolicy::FireEach)
            .await
            .unwrap();
        assert!(reconciliation.updated.is_empty());
        assert!(reconciliation.completed.is_empty());
        assert!(
            !alarms_path.exists(),
            "an idle reconcile must not write the file"
        );

        let _ = std::fs::remove_file(&settings_path);
    }

    #[tokio::test]
    async fn evaluate_usage_persists_armed_state_so_a_reload_does_not_refire() {
        use mlt_core::alarms::ThresholdConfig;
        use mlt_core::domain::{Status, UsageSnapshot, UsageWindow};

        let alarms_path = temp_path("evaluate-usage-alarms");
        let settings_path = temp_path("evaluate-usage-settings");
        let store = FileAlarmStore::load(alarms_path.clone(), settings_path.clone());
        let provider = ProviderId::new("claude");
        store
            .update_settings(Box::new({
                let provider = provider.clone();
                move |mut settings| {
                    settings.set_threshold(ThresholdConfig {
                        provider,
                        window: WindowKind::Weekly,
                        window_description: None,
                        levels: vec![80],
                        enabled: true,
                    });
                    settings
                }
            }))
            .await
            .unwrap();

        let snapshot = UsageSnapshot {
            provider: provider.clone(),
            windows: vec![UsageWindow {
                kind: WindowKind::Weekly,
                used_percent: 90.0,
                window_minutes: None,
                resets_at: Some(Timestamp(100)),
                reset_description: None,
            }],
            status: Status::Ok,
            fetched_at: Timestamp(1),
            account: None,
            note: None,
        };

        let notices = store.evaluate_usage(&snapshot).await.unwrap();
        assert_eq!(notices.len(), 1);

        // The armed state was persisted: a fresh load re-evaluating the same snapshot does not re-fire.
        let reloaded = FileAlarmStore::load(alarms_path.clone(), settings_path.clone());
        let again = reloaded.evaluate_usage(&snapshot).await.unwrap();
        assert!(again.is_empty());

        let _ = std::fs::remove_file(&alarms_path);
        let _ = std::fs::remove_file(&settings_path);
    }

    #[tokio::test]
    async fn evaluate_usage_skips_the_write_at_steady_state() {
        use mlt_core::alarms::ThresholdConfig;
        use mlt_core::domain::{Status, UsageSnapshot, UsageWindow};

        let alarms_path = temp_path("evaluate-idle-alarms");
        let settings_path = temp_path("evaluate-idle-settings");
        let store = FileAlarmStore::load(alarms_path.clone(), settings_path.clone());
        let provider = ProviderId::new("claude");
        store
            .update_settings(Box::new({
                let provider = provider.clone();
                move |mut settings| {
                    settings.set_threshold(ThresholdConfig {
                        provider,
                        window: WindowKind::Weekly,
                        window_description: None,
                        levels: vec![80],
                        enabled: true,
                    });
                    settings
                }
            }))
            .await
            .unwrap();

        let snapshot = UsageSnapshot {
            provider: provider.clone(),
            windows: vec![UsageWindow {
                kind: WindowKind::Weekly,
                used_percent: 90.0,
                window_minutes: None,
                resets_at: Some(Timestamp(100)),
                reset_description: None,
            }],
            status: Status::Ok,
            fetched_at: Timestamp(1),
            account: None,
            note: None,
        };

        // First evaluation fires and persists the armed state.
        assert_eq!(store.evaluate_usage(&snapshot).await.unwrap().len(), 1);
        // Remove the persisted file; an unchanged second evaluation must not recreate it.
        std::fs::remove_file(&settings_path).unwrap();

        let again = store.evaluate_usage(&snapshot).await.unwrap();
        assert!(again.is_empty());
        assert!(
            !settings_path.exists(),
            "a steady-state evaluation must not write the settings file"
        );

        let _ = std::fs::remove_file(&alarms_path);
    }

    #[tokio::test]
    async fn update_settings_applies_and_persists_atomically() {
        let alarms_path = temp_path("update-settings-alarms");
        let settings_path = temp_path("update-settings-settings");
        let store = FileAlarmStore::load(alarms_path.clone(), settings_path.clone());

        let returned = store
            .update_settings(Box::new(|mut settings| {
                settings.missed_policy = MissedPolicy::Coalesce;
                settings
            }))
            .await
            .unwrap();
        assert_eq!(returned.missed_policy, MissedPolicy::Coalesce);

        let reloaded = FileAlarmStore::load(alarms_path.clone(), settings_path.clone());
        assert_eq!(
            reloaded.settings().await.unwrap().missed_policy,
            MissedPolicy::Coalesce
        );

        let _ = std::fs::remove_file(&alarms_path);
        let _ = std::fs::remove_file(&settings_path);
    }
}
