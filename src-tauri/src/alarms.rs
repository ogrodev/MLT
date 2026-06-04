use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use mlt_core::alarms::{
    self as core_alarms, Alarm, AlarmId, AlarmNotice, AlarmPrefs, Firing, MissedPolicy, Recurrence,
    ThresholdConfig,
};
use mlt_core::domain::{ProviderId, Timestamp, UsageSnapshot, WindowKind};
use mlt_core::ports::{AlarmStore, Clock, Notifier, PortError};
use tauri::{Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::Notify;

pub struct TauriNotifier {
    app: tauri::AppHandle,
}

impl TauriNotifier {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

#[async_trait::async_trait]
impl Notifier for TauriNotifier {
    async fn notify(&self, title: &str, body: &str) -> Result<(), PortError> {
        self.app
            .notification()
            .builder()
            .title(title)
            .body(body)
            .show()
            .map_err(|e| PortError::Io(e.to_string()))
    }
}

pub struct AlarmState {
    pub store: Arc<dyn AlarmStore>,
    pub clock: Arc<dyn Clock>,
    pub notifier: Arc<dyn Notifier>,
    pub wake: Arc<Notify>,
    pub seq: AtomicU64,
}

impl AlarmState {
    pub fn next_id(&self) -> AlarmId {
        AlarmId(format!(
            "{}-{}",
            self.clock.now().0,
            self.seq.fetch_add(1, Ordering::Relaxed)
        ))
    }
}

#[tauri::command]
pub async fn list_alarms(state: tauri::State<'_, AlarmState>) -> Result<Vec<Alarm>, String> {
    state.store.alarms().await.map_err(port_error)
}

#[tauri::command]
pub async fn create_alarm(
    state: tauri::State<'_, AlarmState>,
    app: tauri::AppHandle,
    label: String,
    fire_at: i64,
    recurrence: Option<Recurrence>,
) -> Result<Vec<Alarm>, String> {
    let label = validate_label(label)?;
    let alarm = Alarm {
        id: state.next_id(),
        label,
        next_fire_at: Timestamp(validate_fire_at(fire_at)?),
        recurrence,
    };
    state.store.upsert_alarm(&alarm).await.map_err(port_error)?;
    state.wake.notify_one();
    let alarms = state.store.alarms().await.map_err(port_error)?;
    let _ = app.emit("alarms-updated", alarms.clone());
    Ok(alarms)
}

#[tauri::command]
pub async fn update_alarm(
    state: tauri::State<'_, AlarmState>,
    app: tauri::AppHandle,
    id: String,
    label: String,
    fire_at: i64,
    recurrence: Option<Recurrence>,
) -> Result<Vec<Alarm>, String> {
    let label = validate_label(label)?;
    let alarm = Alarm {
        id: AlarmId(id),
        label,
        next_fire_at: Timestamp(validate_fire_at(fire_at)?),
        recurrence,
    };
    state.store.upsert_alarm(&alarm).await.map_err(port_error)?;
    state.wake.notify_one();
    let alarms = state.store.alarms().await.map_err(port_error)?;
    let _ = app.emit("alarms-updated", alarms.clone());
    Ok(alarms)
}

#[tauri::command]
pub async fn delete_alarm(
    state: tauri::State<'_, AlarmState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<Vec<Alarm>, String> {
    state
        .store
        .delete_alarm(&AlarmId(id))
        .await
        .map_err(port_error)?;
    state.wake.notify_one();
    let alarms = state.store.alarms().await.map_err(port_error)?;
    let _ = app.emit("alarms-updated", alarms.clone());
    Ok(alarms)
}

#[tauri::command]
pub async fn get_alarm_prefs(state: tauri::State<'_, AlarmState>) -> Result<AlarmPrefs, String> {
    Ok(state.store.settings().await.map_err(port_error)?.prefs())
}

#[tauri::command]
pub async fn set_threshold_alert(
    state: tauri::State<'_, AlarmState>,
    provider: String,
    window: WindowKind,
    window_description: Option<String>,
    levels: Vec<u8>,
    enabled: bool,
) -> Result<AlarmPrefs, String> {
    let config = ThresholdConfig {
        provider: ProviderId::new(provider),
        window,
        window_description,
        // Sanitize at the boundary so a direct `invoke` can't bypass the UI's guard:
        // drop levels outside 1..=100 (0 fires every poll; >100 never fires), dedupe, sort.
        levels: core_alarms::normalize_levels(&levels),
        enabled,
    };
    let settings = state
        .store
        .update_settings(Box::new(move |mut settings| {
            settings.set_threshold(config);
            settings
        }))
        .await
        .map_err(port_error)?;
    Ok(settings.prefs())
}

#[tauri::command]
pub async fn set_reset_notification(
    state: tauri::State<'_, AlarmState>,
    provider: String,
    window: WindowKind,
    window_description: Option<String>,
    enabled: bool,
) -> Result<AlarmPrefs, String> {
    let provider = ProviderId::new(provider);
    let settings = state
        .store
        .update_settings(Box::new(move |mut settings| {
            settings.set_reset_enabled(&provider, window, window_description.as_deref(), enabled);
            settings
        }))
        .await
        .map_err(port_error)?;
    Ok(settings.prefs())
}

#[tauri::command]
pub async fn set_missed_policy(
    state: tauri::State<'_, AlarmState>,
    policy: MissedPolicy,
) -> Result<AlarmPrefs, String> {
    let settings = state
        .store
        .update_settings(Box::new(move |mut settings| {
            settings.missed_policy = policy;
            settings
        }))
        .await
        .map_err(port_error)?;
    Ok(settings.prefs())
}

pub fn spawn_alarm_scheduler(app: tauri::AppHandle) {
    // After a persistent write failure, back off instead of retrying at the 250ms floor so a
    // disk fault cannot drive a tight reconcile/persist (and, before, notification) loop.
    const WRITE_FAILURE_BACKOFF_MS: u64 = 30_000;
    tauri::async_runtime::spawn(async move {
        loop {
            let (store, clock, notifier, wake) = {
                let state = app.state::<AlarmState>();
                (
                    state.store.clone(),
                    state.clock.clone(),
                    state.notifier.clone(),
                    state.wake.clone(),
                )
            };

            let now = clock.now();
            let missed_policy = store
                .settings()
                .await
                .map(|settings| settings.missed_policy)
                .unwrap_or_default();

            let dur_ms = match store.reconcile_due(now, missed_policy).await {
                Ok(reconciliation) => {
                    // The advance/complete was persisted before we notify, so a crash here drops
                    // a notification rather than re-firing it on next launch.
                    notify_firing(notifier.as_ref(), &reconciliation.firing).await;
                    let alarms = store.alarms().await.unwrap_or_default();
                    if !reconciliation.updated.is_empty() || !reconciliation.completed.is_empty() {
                        let _ = app.emit("alarms-updated", alarms.clone());
                    }
                    next_wake_delay(&alarms, clock.now())
                }
                Err(_) => WRITE_FAILURE_BACKOFF_MS,
            };

            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(dur_ms)) => {},
                _ = wake.notified() => {},
            }
        }
    });
}

/// Sleep until the soonest alarm is due, floored at 250ms (so a freshly-created near alarm
/// fires promptly) and capped at 30s (so we re-check periodically).
fn next_wake_delay(alarms: &[Alarm], now: Timestamp) -> u64 {
    let soonest = alarms.iter().map(|alarm| alarm.next_fire_at.0).min();
    let raw = soonest
        .map(|next| next.saturating_sub(now.0).max(0))
        .unwrap_or(30_000);
    raw.clamp(250, 30_000) as u64
}

pub async fn evaluate_usage(app: &tauri::AppHandle, snapshot: &UsageSnapshot) {
    let Some(state) = app.try_state::<AlarmState>() else {
        return;
    };
    let store = state.store.clone();
    let notifier = state.notifier.clone();

    // The store persists the updated armed/reset state before returning, so we notify only
    // after the write lands (no settings lock held across notification IO, no double-fire on
    // crash).
    let Ok(notices) = store.evaluate_usage(snapshot).await else {
        return;
    };
    for notice in &notices {
        notify_usage_notice(notifier.as_ref(), notice).await;
    }
}

async fn notify_firing(notifier: &dyn Notifier, firing: &Firing) {
    match firing {
        Firing::Each(due) => {
            for alarm in due {
                let _ = notifier.notify(&alarm.label, "Reminder").await;
            }
        }
        Firing::Coalesced(due) => {
            if due.is_empty() {
                return;
            }
            let body = format!("{} reminders came due while MLT was away", due.len());
            let _ = notifier.notify("Missed reminders", &body).await;
        }
    }
}

async fn notify_usage_notice(notifier: &dyn Notifier, notice: &AlarmNotice) {
    match notice {
        AlarmNotice::Threshold {
            provider,
            window,
            window_description,
            level,
        } => {
            let title = format!(
                "{} · {} at {}%",
                provider_display_name(provider),
                window_label(*window, window_description.as_deref()),
                level
            );
            let body = format!("You've used {}% of this window.", level);
            let _ = notifier.notify(&title, &body).await;
        }
        AlarmNotice::Reset {
            provider,
            window,
            window_description,
        } => {
            let title = format!(
                "{} · {} reset",
                provider_display_name(provider),
                window_label(*window, window_description.as_deref())
            );
            let _ = notifier
                .notify(&title, "This usage window has reset.")
                .await;
        }
    }
}

fn provider_display_name(provider: &ProviderId) -> &str {
    if let Some(descriptor) = crate::descriptor_for(provider) {
        descriptor.display_name
    } else {
        provider.as_str()
    }
}

/// Human label for a window: prefer its provider-supplied description (so two same-`kind`
/// windows like Claude's Opus/Sonnet 7-day read distinctly), falling back to the kind.
fn window_label(window: WindowKind, description: Option<&str>) -> String {
    match description {
        Some(label) if !label.is_empty() => label.to_string(),
        _ => match window {
            WindowKind::Session => "Session",
            WindowKind::Weekly => "Weekly",
            WindowKind::Monthly => "Monthly",
            WindowKind::Custom => "Usage",
        }
        .to_string(),
    }
}

fn port_error(e: PortError) -> String {
    e.to_string()
}

/// Reject a blank alarm label at the command boundary (the UI guards it too, but a direct
/// `invoke` could otherwise create a blank-title notification). Returns the trimmed label.
fn validate_label(label: String) -> Result<String, String> {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return Err("Alarm label cannot be empty".to_string());
    }
    Ok(trimmed.to_string())
}

/// Accept only a sane epoch-ms `fire_at` at the command boundary. The UI's `datetime-local`
/// can't produce a bad value, but a direct `invoke` could pass an out-of-range `i64` that the
/// scheduler's `next_occurrence`/wake-delay arithmetic would otherwise turn into a perpetually
/// "due" past instant (a 250ms notify/persist storm). The core math also saturates as a
/// backstop (ADR 0015); this rejects the input outright so no bogus alarm is ever persisted.
/// Upper bound leaves head-room above any realistic reminder while staying far from i64::MAX.
fn validate_fire_at(fire_at: i64) -> Result<i64, String> {
    const MAX_FIRE_AT_MS: i64 = i64::MAX / 2;
    if !(0..=MAX_FIRE_AT_MS).contains(&fire_at) {
        return Err("Alarm time is out of range".to_string());
    }
    Ok(fire_at)
}

#[cfg(test)]
mod tests {
    use super::{next_wake_delay, validate_fire_at};
    use mlt_core::alarms::{Alarm, AlarmId, Recurrence};
    use mlt_core::domain::Timestamp;

    #[test]
    fn validate_fire_at_rejects_out_of_range_and_accepts_normal() {
        assert!(validate_fire_at(1_700_000_000_000).is_ok());
        assert!(validate_fire_at(0).is_ok());
        assert!(validate_fire_at(-1).is_err());
        assert!(validate_fire_at(i64::MIN).is_err());
        assert!(validate_fire_at(i64::MAX).is_err());
    }

    #[test]
    fn next_wake_delay_is_clamped_and_never_overflows() {
        // Empty -> the 30s re-check cap.
        assert_eq!(next_wake_delay(&[], Timestamp(0)), 30_000);
        // A soon alarm floors at 250ms.
        let soon = Alarm {
            id: AlarmId::new("a"),
            label: "a".into(),
            next_fire_at: Timestamp(100),
            recurrence: Some(Recurrence::Daily),
        };
        assert_eq!(
            next_wake_delay(std::slice::from_ref(&soon), Timestamp(0)),
            250
        );
        // An extreme past timestamp must saturate (not overflow) and floor at 250ms.
        let bad = Alarm {
            id: AlarmId::new("b"),
            label: "b".into(),
            next_fire_at: Timestamp(i64::MIN),
            recurrence: None,
        };
        assert_eq!(
            next_wake_delay(std::slice::from_ref(&bad), Timestamp(i64::MAX)),
            250
        );
    }
}
