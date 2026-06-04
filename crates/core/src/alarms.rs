//! Pure alarm scheduling and usage-derived notification rules.
use crate::domain::{ProviderId, Timestamp, UsageSnapshot, UsageWindow, WindowKind};
use serde::{Deserialize, Serialize};

pub const DAY_MS: i64 = 86_400_000;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AlarmId(pub String);

impl AlarmId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// RRULE-lite recurrence (OPEN_QUESTIONS Q5). UTC-day arithmetic on epoch-ms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Recurrence {
    Daily,
    Weekly,
    EveryNDays { days: u32 },
}

impl Recurrence {
    pub fn period_ms(self) -> i64 {
        match self {
            Recurrence::Daily => DAY_MS,
            Recurrence::Weekly => 7 * DAY_MS,
            Recurrence::EveryNDays { days } => i64::from(days.max(1)) * DAY_MS,
        }
    }
}

/// A persisted user alarm. `None` recurrence means one-off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alarm {
    pub id: AlarmId,
    pub label: String,
    pub next_fire_at: Timestamp,
    pub recurrence: Option<Recurrence>,
}

/// Downtime catch-up policy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissedPolicy {
    #[default]
    FireEach,
    Coalesce,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DueAlarm {
    pub id: AlarmId,
    pub label: String,
    pub scheduled_for: Timestamp,
}

/// One notification per alarm or a single summary for all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Firing {
    Each(Vec<DueAlarm>),
    Coalesced(Vec<DueAlarm>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reconciliation {
    pub firing: Firing,
    pub updated: Vec<Alarm>,
    pub completed: Vec<AlarmId>,
}

/// Next occurrence strictly after `after` for a recurring schedule anchored at `anchor`.
pub fn next_occurrence(anchor: Timestamp, after: Timestamp, recurrence: Recurrence) -> Timestamp {
    if anchor.0 > after.0 {
        return anchor;
    }

    let period = recurrence.period_ms();
    let periods_after_anchor = (after.0 - anchor.0) / period + 1;
    Timestamp(anchor.0 + periods_after_anchor * period)
}

/// Scan due alarms, decide delivery, advance recurring alarms, and complete one-offs.
pub fn reconcile(now: Timestamp, alarms: &[Alarm], policy: MissedPolicy) -> Reconciliation {
    let mut due = Vec::new();
    let mut updated = Vec::new();
    let mut completed = Vec::new();

    for alarm in alarms {
        if alarm.next_fire_at > now {
            continue;
        }

        due.push(DueAlarm {
            id: alarm.id.clone(),
            label: alarm.label.clone(),
            scheduled_for: alarm.next_fire_at,
        });

        if let Some(recurrence) = alarm.recurrence {
            let mut advanced = alarm.clone();
            advanced.next_fire_at = next_occurrence(alarm.next_fire_at, now, recurrence);
            updated.push(advanced);
        } else {
            completed.push(alarm.id.clone());
        }
    }

    let firing = if due.len() > 1 && policy == MissedPolicy::Coalesce {
        Firing::Coalesced(due)
    } else {
        Firing::Each(due)
    };

    Reconciliation {
        firing,
        updated,
        completed,
    }
}

/// Threshold alert config for one provider/window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThresholdConfig {
    pub provider: ProviderId,
    pub window: WindowKind,
    pub levels: Vec<u8>,
    pub enabled: bool,
}

/// Per-(provider,window) armed state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThresholdState {
    pub instance: Option<Timestamp>,
    pub fired_levels: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThresholdCrossing {
    pub provider: ProviderId,
    pub window: WindowKind,
    pub level: u8,
}

/// Evaluate one window against a threshold config and prior armed state.
pub fn evaluate_thresholds(
    config: &ThresholdConfig,
    window: &UsageWindow,
    prior: &ThresholdState,
) -> (Vec<ThresholdCrossing>, ThresholdState) {
    if !config.enabled {
        return (Vec::new(), prior.clone());
    }

    let mut state = if window.resets_at != prior.instance {
        ThresholdState {
            instance: window.resets_at,
            fired_levels: Vec::new(),
        }
    } else {
        prior.clone()
    };

    let mut crossings = Vec::new();
    for level in &config.levels {
        if window.used_percent >= f64::from(*level) && !state.fired_levels.contains(level) {
            crossings.push(ThresholdCrossing {
                provider: config.provider.clone(),
                window: config.window,
                level: *level,
            });
            state.fired_levels.push(*level);
        }
    }

    (crossings, state)
}

/// Last-seen reset marker for window-reset detection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResetState {
    pub last_resets_at: Option<Timestamp>,
}

/// Detect whether the window rolled to a new instance since the prior observation.
pub fn detect_reset(window: &UsageWindow, prior: &ResetState) -> (bool, ResetState) {
    let fired = match (prior.last_resets_at, window.resets_at) {
        (Some(previous), Some(current)) => current > previous,
        _ => false,
    };

    (
        fired,
        ResetState {
            last_resets_at: window.resets_at,
        },
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResetPref {
    pub provider: ProviderId,
    pub window: WindowKind,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlarmSettings {
    #[serde(default)]
    pub thresholds: Vec<ThresholdConfig>,
    #[serde(default)]
    pub resets: Vec<ResetPref>,
    #[serde(default)]
    pub missed_policy: MissedPolicy,
    #[serde(default)]
    pub threshold_state: Vec<ThresholdStateEntry>,
    #[serde(default)]
    pub reset_state: Vec<ResetStateEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThresholdStateEntry {
    pub provider: ProviderId,
    pub window: WindowKind,
    pub state: ThresholdState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResetStateEntry {
    pub provider: ProviderId,
    pub window: WindowKind,
    pub state: ResetState,
}

impl AlarmSettings {
    pub fn threshold(&self, provider: &ProviderId, window: WindowKind) -> Option<&ThresholdConfig> {
        self.thresholds
            .iter()
            .find(|entry| alarm_key_matches(&entry.provider, entry.window, provider, window))
    }

    pub fn set_threshold(&mut self, config: ThresholdConfig) {
        if let Some(entry) = self.thresholds.iter_mut().find(|entry| {
            alarm_key_matches(
                &entry.provider,
                entry.window,
                &config.provider,
                config.window,
            )
        }) {
            *entry = config;
        } else {
            self.thresholds.push(config);
        }
    }

    pub fn reset_enabled(&self, provider: &ProviderId, window: WindowKind) -> bool {
        self.resets
            .iter()
            .find(|entry| alarm_key_matches(&entry.provider, entry.window, provider, window))
            .map(|entry| entry.enabled)
            .unwrap_or(false)
    }

    pub fn set_reset_enabled(&mut self, provider: &ProviderId, window: WindowKind, enabled: bool) {
        if let Some(entry) = self
            .resets
            .iter_mut()
            .find(|entry| alarm_key_matches(&entry.provider, entry.window, provider, window))
        {
            entry.enabled = enabled;
        } else {
            self.resets.push(ResetPref {
                provider: provider.clone(),
                window,
                enabled,
            });
        }
    }

    pub fn threshold_state(&self, provider: &ProviderId, window: WindowKind) -> ThresholdState {
        self.threshold_state
            .iter()
            .find(|entry| alarm_key_matches(&entry.provider, entry.window, provider, window))
            .map(|entry| entry.state.clone())
            .unwrap_or_default()
    }

    pub fn set_threshold_state(
        &mut self,
        provider: &ProviderId,
        window: WindowKind,
        state: ThresholdState,
    ) {
        if let Some(entry) = self
            .threshold_state
            .iter_mut()
            .find(|entry| alarm_key_matches(&entry.provider, entry.window, provider, window))
        {
            entry.state = state;
        } else {
            self.threshold_state.push(ThresholdStateEntry {
                provider: provider.clone(),
                window,
                state,
            });
        }
    }

    pub fn reset_state(&self, provider: &ProviderId, window: WindowKind) -> ResetState {
        self.reset_state
            .iter()
            .find(|entry| alarm_key_matches(&entry.provider, entry.window, provider, window))
            .map(|entry| entry.state.clone())
            .unwrap_or_default()
    }

    pub fn set_reset_state(
        &mut self,
        provider: &ProviderId,
        window: WindowKind,
        state: ResetState,
    ) {
        if let Some(entry) = self
            .reset_state
            .iter_mut()
            .find(|entry| alarm_key_matches(&entry.provider, entry.window, provider, window))
        {
            entry.state = state;
        } else {
            self.reset_state.push(ResetStateEntry {
                provider: provider.clone(),
                window,
                state,
            });
        }
    }

    pub fn prefs(&self) -> AlarmPrefs {
        AlarmPrefs {
            thresholds: self.thresholds.clone(),
            resets: self.resets.clone(),
            missed_policy: self.missed_policy,
        }
    }
}

/// UI-facing projection of alarm prefs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlarmPrefs {
    pub thresholds: Vec<ThresholdConfig>,
    pub resets: Vec<ResetPref>,
    pub missed_policy: MissedPolicy,
}

/// Structured fact that a usage-derived notification should fire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlarmNotice {
    Threshold {
        provider: ProviderId,
        window: WindowKind,
        level: u8,
    },
    Reset {
        provider: ProviderId,
        window: WindowKind,
    },
}

/// Evaluate a usage snapshot against settings and return notices to deliver.
pub fn evaluate_snapshot(
    snapshot: &UsageSnapshot,
    settings: &mut AlarmSettings,
) -> Vec<AlarmNotice> {
    let mut notices = Vec::new();

    for window in &snapshot.windows {
        if let Some(config) = settings.threshold(&snapshot.provider, window.kind).cloned() {
            if config.enabled {
                let prior = settings.threshold_state(&snapshot.provider, window.kind);
                let (crossings, state) = evaluate_thresholds(&config, window, &prior);

                for crossing in crossings {
                    notices.push(AlarmNotice::Threshold {
                        provider: crossing.provider,
                        window: crossing.window,
                        level: crossing.level,
                    });
                }

                settings.set_threshold_state(&snapshot.provider, window.kind, state);
            }
        }

        if settings.reset_enabled(&snapshot.provider, window.kind) {
            let prior = settings.reset_state(&snapshot.provider, window.kind);
            let (fired, state) = detect_reset(window, &prior);
            if fired {
                notices.push(AlarmNotice::Reset {
                    provider: snapshot.provider.clone(),
                    window: window.kind,
                });
            }
            settings.set_reset_state(&snapshot.provider, window.kind, state);
        }
    }

    notices
}

fn alarm_key_matches(
    entry_provider: &ProviderId,
    entry_window: WindowKind,
    provider: &ProviderId,
    window: WindowKind,
) -> bool {
    entry_provider == provider && entry_window == window
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Status, UsageNote};

    fn ts(ms: i64) -> Timestamp {
        Timestamp(ms)
    }

    fn provider(id: &str) -> ProviderId {
        ProviderId::new(id)
    }

    fn window(kind: WindowKind, used_percent: f64, resets_at: Option<Timestamp>) -> UsageWindow {
        UsageWindow {
            kind,
            used_percent,
            window_minutes: None,
            resets_at,
            reset_description: None,
        }
    }

    fn alarm(
        id: &str,
        label: &str,
        next_fire_at: Timestamp,
        recurrence: Option<Recurrence>,
    ) -> Alarm {
        Alarm {
            id: AlarmId::new(id),
            label: label.to_string(),
            next_fire_at,
            recurrence,
        }
    }

    fn snapshot(provider: ProviderId, windows: Vec<UsageWindow>) -> UsageSnapshot {
        UsageSnapshot {
            provider,
            windows,
            status: Status::Ok,
            fetched_at: ts(1),
            account: None,
            note: None::<UsageNote>,
        }
    }

    #[test]
    fn alarm_id_and_recurrence_helpers_return_contract_values() {
        let id = AlarmId::new("alarm-1");

        assert_eq!(id.as_str(), "alarm-1");
        assert_eq!(Recurrence::Daily.period_ms(), DAY_MS);
        assert_eq!(Recurrence::Weekly.period_ms(), 7 * DAY_MS);
        assert_eq!(Recurrence::EveryNDays { days: 0 }.period_ms(), DAY_MS);
        assert_eq!(Recurrence::EveryNDays { days: 3 }.period_ms(), 3 * DAY_MS);
    }

    #[test]
    fn next_occurrence_advances_strictly_after_anchor_and_large_gaps() {
        let anchor = ts(1_000);

        assert_eq!(next_occurrence(anchor, ts(999), Recurrence::Daily), anchor);
        assert_eq!(
            next_occurrence(anchor, anchor, Recurrence::Daily),
            ts(1_000 + DAY_MS)
        );
        assert_eq!(
            next_occurrence(anchor, ts(1_000 + (365 * DAY_MS) + 42), Recurrence::Daily),
            ts(1_000 + (366 * DAY_MS))
        );
    }

    #[test]
    fn reconcile_completes_one_off_and_leaves_not_due_alarm_untouched() {
        let due = alarm("due", "Due", ts(100), None);
        let future = alarm("future", "Future", ts(200), None);

        let reconciled = reconcile(ts(100), &[due.clone(), future], MissedPolicy::FireEach);

        assert_eq!(
            reconciled.firing,
            Firing::Each(vec![DueAlarm {
                id: due.id.clone(),
                label: due.label.clone(),
                scheduled_for: due.next_fire_at,
            }])
        );
        assert_eq!(reconciled.updated, Vec::<Alarm>::new());
        assert_eq!(reconciled.completed, vec![due.id]);
    }

    #[test]
    fn reconcile_advances_recurring_alarm_once_past_now_after_downtime() {
        let due = alarm("daily", "Daily", ts(100), Some(Recurrence::Daily));
        let now = ts(100 + (10 * DAY_MS) + 5);

        let reconciled = reconcile(now, std::slice::from_ref(&due), MissedPolicy::FireEach);

        assert_eq!(
            reconciled.firing,
            Firing::Each(vec![DueAlarm {
                id: due.id.clone(),
                label: due.label.clone(),
                scheduled_for: due.next_fire_at,
            }])
        );
        assert_eq!(reconciled.completed, Vec::<AlarmId>::new());
        assert_eq!(reconciled.updated.len(), 1);
        assert_eq!(reconciled.updated[0].next_fire_at, ts(100 + (11 * DAY_MS)));
    }

    #[test]
    fn reconcile_fire_each_vs_coalesce_and_single_due_coalesce_is_each() {
        let first = alarm("one", "One", ts(10), None);
        let second = alarm("two", "Two", ts(20), None);
        let due = vec![
            DueAlarm {
                id: first.id.clone(),
                label: first.label.clone(),
                scheduled_for: first.next_fire_at,
            },
            DueAlarm {
                id: second.id.clone(),
                label: second.label.clone(),
                scheduled_for: second.next_fire_at,
            },
        ];

        assert_eq!(
            reconcile(
                ts(20),
                &[first.clone(), second.clone()],
                MissedPolicy::FireEach
            )
            .firing,
            Firing::Each(due.clone())
        );
        assert_eq!(
            reconcile(ts(20), &[first.clone(), second], MissedPolicy::Coalesce).firing,
            Firing::Coalesced(due)
        );
        assert_eq!(
            reconcile(ts(10), std::slice::from_ref(&first), MissedPolicy::Coalesce).firing,
            Firing::Each(vec![DueAlarm {
                id: first.id,
                label: first.label,
                scheduled_for: first.next_fire_at,
            }])
        );
    }

    #[test]
    fn reconcile_never_double_fires_recurring_alarm_at_same_now() {
        let due = alarm("daily", "Daily", ts(100), Some(Recurrence::Daily));
        let now = ts(100);
        let first = reconcile(now, &[due], MissedPolicy::FireEach);

        let second = reconcile(now, &first.updated, MissedPolicy::FireEach);

        assert_eq!(second.firing, Firing::Each(Vec::new()));
        assert_eq!(second.updated, Vec::<Alarm>::new());
        assert_eq!(second.completed, Vec::<AlarmId>::new());
    }

    #[test]
    fn evaluate_thresholds_fires_once_per_crossing_until_reset() {
        let config = ThresholdConfig {
            provider: provider("claude"),
            window: WindowKind::Weekly,
            levels: vec![50, 90],
            enabled: true,
        };
        let weekly = window(WindowKind::Weekly, 50.0, Some(ts(100)));

        let (crossings, state) = evaluate_thresholds(&config, &weekly, &ThresholdState::default());
        assert_eq!(
            crossings,
            vec![ThresholdCrossing {
                provider: config.provider.clone(),
                window: WindowKind::Weekly,
                level: 50,
            }]
        );
        assert_eq!(state.instance, Some(ts(100)));
        assert_eq!(state.fired_levels, vec![50]);

        let (crossings, state) = evaluate_thresholds(
            &config,
            &window(WindowKind::Weekly, 60.0, Some(ts(100))),
            &state,
        );
        assert_eq!(crossings, Vec::<ThresholdCrossing>::new());
        assert_eq!(state.fired_levels, vec![50]);

        let (crossings, state) = evaluate_thresholds(
            &config,
            &window(WindowKind::Weekly, 95.0, Some(ts(100))),
            &state,
        );
        assert_eq!(
            crossings,
            vec![ThresholdCrossing {
                provider: config.provider.clone(),
                window: WindowKind::Weekly,
                level: 90,
            }]
        );
        assert_eq!(state.fired_levels, vec![50, 90]);
    }

    #[test]
    fn evaluate_thresholds_rearms_after_reset_and_fires_two_levels_at_once() {
        let config = ThresholdConfig {
            provider: provider("claude"),
            window: WindowKind::Monthly,
            levels: vec![50, 90],
            enabled: true,
        };
        let prior = ThresholdState {
            instance: Some(ts(100)),
            fired_levels: vec![50, 90],
        };

        let (crossings, state) = evaluate_thresholds(
            &config,
            &window(WindowKind::Monthly, 95.0, Some(ts(200))),
            &prior,
        );

        assert_eq!(
            crossings,
            vec![
                ThresholdCrossing {
                    provider: config.provider.clone(),
                    window: WindowKind::Monthly,
                    level: 50,
                },
                ThresholdCrossing {
                    provider: config.provider.clone(),
                    window: WindowKind::Monthly,
                    level: 90,
                },
            ]
        );
        assert_eq!(state.instance, Some(ts(200)));
        assert_eq!(state.fired_levels, vec![50, 90]);
    }

    #[test]
    fn evaluate_thresholds_disabled_config_does_not_mutate_state() {
        let config = ThresholdConfig {
            provider: provider("claude"),
            window: WindowKind::Weekly,
            levels: vec![50],
            enabled: false,
        };
        let prior = ThresholdState {
            instance: Some(ts(100)),
            fired_levels: vec![50],
        };

        let (crossings, state) = evaluate_thresholds(
            &config,
            &window(WindowKind::Weekly, 100.0, Some(ts(200))),
            &prior,
        );

        assert_eq!(crossings, Vec::<ThresholdCrossing>::new());
        assert_eq!(state, prior);
    }

    #[test]
    fn detect_reset_records_first_observation_then_fires_on_advance_only() {
        let (fired, state) = detect_reset(
            &window(WindowKind::Weekly, 0.0, Some(ts(100))),
            &ResetState::default(),
        );
        assert!(!fired);
        assert_eq!(state.last_resets_at, Some(ts(100)));

        let (fired, state) = detect_reset(&window(WindowKind::Weekly, 0.0, Some(ts(200))), &state);
        assert!(fired);
        assert_eq!(state.last_resets_at, Some(ts(200)));

        let (fired, state) = detect_reset(&window(WindowKind::Weekly, 0.0, Some(ts(200))), &state);
        assert!(!fired);
        assert_eq!(state.last_resets_at, Some(ts(200)));
    }

    #[test]
    fn evaluate_snapshot_emits_notices_mutates_state_and_respects_enabled_flags() {
        let claude = provider("claude");
        let mut settings = AlarmSettings::default();
        settings.set_threshold(ThresholdConfig {
            provider: claude.clone(),
            window: WindowKind::Weekly,
            levels: vec![50, 90],
            enabled: true,
        });
        settings.set_threshold(ThresholdConfig {
            provider: claude.clone(),
            window: WindowKind::Monthly,
            levels: vec![75],
            enabled: false,
        });
        settings.set_threshold_state(
            &claude,
            WindowKind::Monthly,
            ThresholdState {
                instance: Some(ts(10)),
                fired_levels: vec![75],
            },
        );
        settings.set_reset_enabled(&claude, WindowKind::Weekly, true);
        settings.set_reset_state(
            &claude,
            WindowKind::Weekly,
            ResetState {
                last_resets_at: Some(ts(100)),
            },
        );
        settings.set_reset_enabled(&claude, WindowKind::Monthly, false);
        settings.set_reset_state(
            &claude,
            WindowKind::Monthly,
            ResetState {
                last_resets_at: Some(ts(100)),
            },
        );

        let notices = evaluate_snapshot(
            &snapshot(
                claude.clone(),
                vec![
                    window(WindowKind::Weekly, 95.0, Some(ts(200))),
                    window(WindowKind::Monthly, 100.0, Some(ts(300))),
                ],
            ),
            &mut settings,
        );

        assert_eq!(
            notices,
            vec![
                AlarmNotice::Threshold {
                    provider: claude.clone(),
                    window: WindowKind::Weekly,
                    level: 50,
                },
                AlarmNotice::Threshold {
                    provider: claude.clone(),
                    window: WindowKind::Weekly,
                    level: 90,
                },
                AlarmNotice::Reset {
                    provider: claude.clone(),
                    window: WindowKind::Weekly,
                },
            ]
        );
        assert_eq!(
            settings.threshold_state(&claude, WindowKind::Weekly),
            ThresholdState {
                instance: Some(ts(200)),
                fired_levels: vec![50, 90],
            }
        );
        assert_eq!(
            settings.reset_state(&claude, WindowKind::Weekly),
            ResetState {
                last_resets_at: Some(ts(200)),
            }
        );
        assert_eq!(
            settings.threshold_state(&claude, WindowKind::Monthly),
            ThresholdState {
                instance: Some(ts(10)),
                fired_levels: vec![75],
            }
        );
        assert_eq!(
            settings.reset_state(&claude, WindowKind::Monthly),
            ResetState {
                last_resets_at: Some(ts(100)),
            }
        );
    }

    #[test]
    fn alarm_settings_helpers_upsert_default_and_project_prefs() {
        let claude = provider("claude");
        let codex = provider("codex");
        let mut settings = AlarmSettings {
            missed_policy: MissedPolicy::Coalesce,
            ..AlarmSettings::default()
        };

        assert!(settings.threshold(&claude, WindowKind::Weekly).is_none());
        assert!(!settings.reset_enabled(&claude, WindowKind::Weekly));
        assert_eq!(
            settings.threshold_state(&claude, WindowKind::Weekly),
            ThresholdState::default()
        );
        assert_eq!(
            settings.reset_state(&claude, WindowKind::Weekly),
            ResetState::default()
        );

        settings.set_threshold(ThresholdConfig {
            provider: claude.clone(),
            window: WindowKind::Weekly,
            levels: vec![50],
            enabled: true,
        });
        settings.set_threshold(ThresholdConfig {
            provider: claude.clone(),
            window: WindowKind::Weekly,
            levels: vec![80, 95],
            enabled: false,
        });
        settings.set_threshold(ThresholdConfig {
            provider: codex.clone(),
            window: WindowKind::Weekly,
            levels: vec![70],
            enabled: true,
        });

        assert_eq!(settings.thresholds.len(), 2);
        assert_eq!(
            settings
                .threshold(&claude, WindowKind::Weekly)
                .unwrap()
                .levels
                .as_slice(),
            &[80, 95]
        );
        assert!(
            settings
                .threshold(&codex, WindowKind::Weekly)
                .unwrap()
                .enabled
        );

        settings.set_reset_enabled(&claude, WindowKind::Weekly, true);
        settings.set_reset_enabled(&claude, WindowKind::Weekly, false);
        assert!(!settings.reset_enabled(&claude, WindowKind::Weekly));
        assert_eq!(settings.resets.len(), 1);

        settings.set_threshold_state(
            &claude,
            WindowKind::Weekly,
            ThresholdState {
                instance: Some(ts(10)),
                fired_levels: vec![80],
            },
        );
        settings.set_threshold_state(
            &claude,
            WindowKind::Weekly,
            ThresholdState {
                instance: Some(ts(20)),
                fired_levels: vec![95],
            },
        );
        settings.set_reset_state(
            &claude,
            WindowKind::Weekly,
            ResetState {
                last_resets_at: Some(ts(30)),
            },
        );

        assert_eq!(settings.threshold_state.len(), 1);
        assert_eq!(
            settings.threshold_state(&claude, WindowKind::Weekly),
            ThresholdState {
                instance: Some(ts(20)),
                fired_levels: vec![95],
            }
        );
        assert_eq!(
            settings.reset_state(&claude, WindowKind::Weekly),
            ResetState {
                last_resets_at: Some(ts(30)),
            }
        );

        let prefs = settings.prefs();
        assert_eq!(prefs.thresholds, settings.thresholds);
        assert_eq!(prefs.resets, settings.resets);
        assert_eq!(prefs.missed_policy, MissedPolicy::Coalesce);
    }
}
