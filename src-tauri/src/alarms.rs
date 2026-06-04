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
use tokio::sync::{Mutex, Notify};

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
    pub settings_lock: Arc<Mutex<()>>,
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
    let alarm = Alarm {
        id: state.next_id(),
        label,
        next_fire_at: Timestamp(fire_at),
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
    let alarm = Alarm {
        id: AlarmId(id),
        label,
        next_fire_at: Timestamp(fire_at),
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
    levels: Vec<u8>,
    enabled: bool,
) -> Result<AlarmPrefs, String> {
    let _guard = state.settings_lock.lock().await;
    let mut settings = state.store.settings().await.map_err(port_error)?;
    settings.set_threshold(ThresholdConfig {
        provider: ProviderId::new(provider),
        window,
        levels,
        enabled,
    });
    state
        .store
        .save_settings(&settings)
        .await
        .map_err(port_error)?;
    Ok(settings.prefs())
}

#[tauri::command]
pub async fn set_reset_notification(
    state: tauri::State<'_, AlarmState>,
    provider: String,
    window: WindowKind,
    enabled: bool,
) -> Result<AlarmPrefs, String> {
    let _guard = state.settings_lock.lock().await;
    let mut settings = state.store.settings().await.map_err(port_error)?;
    let provider = ProviderId::new(provider);
    settings.set_reset_enabled(&provider, window, enabled);
    state
        .store
        .save_settings(&settings)
        .await
        .map_err(port_error)?;
    Ok(settings.prefs())
}

#[tauri::command]
pub async fn set_missed_policy(
    state: tauri::State<'_, AlarmState>,
    policy: MissedPolicy,
) -> Result<AlarmPrefs, String> {
    let _guard = state.settings_lock.lock().await;
    let mut settings = state.store.settings().await.map_err(port_error)?;
    settings.missed_policy = policy;
    state
        .store
        .save_settings(&settings)
        .await
        .map_err(port_error)?;
    Ok(settings.prefs())
}

pub fn spawn_alarm_scheduler(app: tauri::AppHandle) {
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
            let alarms = store.alarms().await.unwrap_or_default();
            let missed_policy = store
                .settings()
                .await
                .map(|settings| settings.missed_policy)
                .unwrap_or_default();
            let reconciliation = core_alarms::reconcile(now, &alarms, missed_policy);

            notify_firing(notifier.as_ref(), &reconciliation.firing).await;

            let mut changed = false;
            for alarm in &reconciliation.updated {
                changed |= store.upsert_alarm(alarm).await.is_ok();
            }
            for id in &reconciliation.completed {
                changed |= store.delete_alarm(id).await.is_ok();
            }
            if changed {
                if let Ok(alarms) = store.alarms().await {
                    let _ = app.emit("alarms-updated", alarms);
                }
            }

            let alarms = store.alarms().await.unwrap_or_default();
            let soonest = alarms.iter().map(|alarm| alarm.next_fire_at.0).min();
            let raw = soonest
                .map(|next| (next - clock.now().0).max(0))
                .unwrap_or(30_000);
            let dur_ms = raw.clamp(250, 30_000);

            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(dur_ms as u64)) => {},
                _ = wake.notified() => {},
            }
        }
    });
}

pub async fn evaluate_usage(app: &tauri::AppHandle, snapshot: &UsageSnapshot) {
    let Some(state) = app.try_state::<AlarmState>() else {
        return;
    };
    let store = state.store.clone();
    let notifier = state.notifier.clone();
    let settings_lock = state.settings_lock.clone();

    let _guard = settings_lock.lock().await;
    let Ok(mut settings) = store.settings().await else {
        return;
    };
    let notices = core_alarms::evaluate_snapshot(snapshot, &mut settings);
    for notice in &notices {
        notify_usage_notice(notifier.as_ref(), notice).await;
    }
    let _ = store.save_settings(&settings).await;
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
            level,
        } => {
            let title = format!(
                "{} · {} at {}%",
                provider_display_name(provider),
                window_label(*window),
                level
            );
            let body = format!("You've used {}% of this window.", level);
            let _ = notifier.notify(&title, &body).await;
        }
        AlarmNotice::Reset { provider, window } => {
            let title = format!(
                "{} · {} reset",
                provider_display_name(provider),
                window_label(*window)
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

fn window_label(window: WindowKind) -> &'static str {
    match window {
        WindowKind::Session => "Session",
        WindowKind::Weekly => "Weekly",
        WindowKind::Monthly => "Monthly",
        WindowKind::Custom => "Usage",
    }
}

fn port_error(e: PortError) -> String {
    e.to_string()
}
