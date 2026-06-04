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
    // Saturating throughout: a malformed `anchor`/`after` (e.g. an out-of-range `fire_at` from a
    // direct command invoke) must never overflow-panic — ADR 0015: the pure core never panics on
    // untrusted input. Saturated values pin the alarm far in the future rather than wrapping into
    // a perpetually-due past instant.
    let delta = after.0.saturating_sub(anchor.0);
    let periods_after_anchor = delta / period + 1;
    Timestamp(
        anchor
            .0
            .saturating_add(periods_after_anchor.saturating_mul(period)),
    )
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

/// Threshold alert config for one provider window. `window_description` distinguishes
/// multiple same-`kind` windows of one provider (e.g. Claude's Opus vs Sonnet 7-day
/// `Custom` windows), matching the UI's `thresholdFor`/`thresholdKey`; `None` is the
/// unlabeled case. Assumes a window's description is a stable key — if a provider ever
/// renames it, the saved config orphans (stops firing) rather than misfiring.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThresholdConfig {
    pub provider: ProviderId,
    pub window: WindowKind,
    #[serde(default)]
    pub window_description: Option<String>,
    pub levels: Vec<u8>,
    pub enabled: bool,
}

/// Per-window armed state, owned by a [`ThresholdStateEntry`] keyed by (provider, kind,
/// description). Keying by `kind` alone let two same-kind windows clobber a single slot.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThresholdState {
    pub instance: Option<Timestamp>,
    pub fired_levels: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThresholdCrossing {
    pub provider: ProviderId,
    pub window: WindowKind,
    pub window_description: Option<String>,
    pub level: u8,
}

/// Normalize threshold levels to the valid, deduped, ascending set the engine expects: keep
/// only `1..=100` (a `0` would fire every poll; `>100` would never fire), each level once.
/// Mirrors the frontend's `parseLevels` so a direct command `invoke` can't bypass the UI guard.
pub fn normalize_levels(levels: &[u8]) -> Vec<u8> {
    let mut seen = [false; 101];
    let mut normalized = Vec::new();
    for &level in levels {
        if (1..=100).contains(&level) && !seen[level as usize] {
            seen[level as usize] = true;
            normalized.push(level);
        }
    }
    normalized.sort_unstable();
    normalized
}

/// Evaluate one window against a threshold config and prior armed state. Re-arms (clears
/// fired levels) on a genuine window reset: either a new `resets_at` instance, or usage
/// falling back below the lowest configured level — the only reset signal for windows that
/// never expose `resets_at` (e.g. credit-style windows that would otherwise fire once ever).
pub fn evaluate_thresholds(
    config: &ThresholdConfig,
    window: &UsageWindow,
    prior: &ThresholdState,
) -> (Vec<ThresholdCrossing>, ThresholdState) {
    if !config.enabled {
        return (Vec::new(), prior.clone());
    }

    let new_instance = window.resets_at != prior.instance;
    let dropped_below_all = config
        .levels
        .iter()
        .copied()
        .min()
        .is_some_and(|lowest| window.used_percent < f64::from(lowest));

    let mut state = if new_instance || dropped_below_all {
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
                window_description: config.window_description.clone(),
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
    #[serde(default)]
    pub window_description: Option<String>,
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
    #[serde(default)]
    pub window_description: Option<String>,
    pub state: ThresholdState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResetStateEntry {
    pub provider: ProviderId,
    pub window: WindowKind,
    #[serde(default)]
    pub window_description: Option<String>,
    pub state: ResetState,
}

impl AlarmSettings {
    pub fn threshold(
        &self,
        provider: &ProviderId,
        window: WindowKind,
        description: Option<&str>,
    ) -> Option<&ThresholdConfig> {
        find_keyed(&self.thresholds, provider, window, description)
    }

    pub fn set_threshold(&mut self, config: ThresholdConfig) {
        if let Some(entry) = find_keyed_mut(
            &mut self.thresholds,
            &config.provider,
            config.window,
            config.window_description.as_deref(),
        ) {
            *entry = config;
        } else {
            self.thresholds.push(config);
        }
    }

    pub fn reset_enabled(
        &self,
        provider: &ProviderId,
        window: WindowKind,
        description: Option<&str>,
    ) -> bool {
        find_keyed(&self.resets, provider, window, description)
            .map(|entry| entry.enabled)
            .unwrap_or(false)
    }

    pub fn set_reset_enabled(
        &mut self,
        provider: &ProviderId,
        window: WindowKind,
        description: Option<&str>,
        enabled: bool,
    ) {
        if let Some(entry) = find_keyed_mut(&mut self.resets, provider, window, description) {
            entry.enabled = enabled;
        } else {
            self.resets.push(ResetPref {
                provider: provider.clone(),
                window,
                window_description: description.map(str::to_owned),
                enabled,
            });
        }
    }

    pub fn threshold_state(
        &self,
        provider: &ProviderId,
        window: WindowKind,
        description: Option<&str>,
    ) -> ThresholdState {
        find_keyed(&self.threshold_state, provider, window, description)
            .map(|entry| entry.state.clone())
            .unwrap_or_default()
    }

    pub fn set_threshold_state(
        &mut self,
        provider: &ProviderId,
        window: WindowKind,
        description: Option<&str>,
        state: ThresholdState,
    ) {
        if let Some(entry) =
            find_keyed_mut(&mut self.threshold_state, provider, window, description)
        {
            entry.state = state;
        } else {
            self.threshold_state.push(ThresholdStateEntry {
                provider: provider.clone(),
                window,
                window_description: description.map(str::to_owned),
                state,
            });
        }
    }

    pub fn reset_state(
        &self,
        provider: &ProviderId,
        window: WindowKind,
        description: Option<&str>,
    ) -> ResetState {
        find_keyed(&self.reset_state, provider, window, description)
            .map(|entry| entry.state.clone())
            .unwrap_or_default()
    }

    pub fn set_reset_state(
        &mut self,
        provider: &ProviderId,
        window: WindowKind,
        description: Option<&str>,
        state: ResetState,
    ) {
        if let Some(entry) = find_keyed_mut(&mut self.reset_state, provider, window, description) {
            entry.state = state;
        } else {
            self.reset_state.push(ResetStateEntry {
                provider: provider.clone(),
                window,
                window_description: description.map(str::to_owned),
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
        window_description: Option<String>,
        level: u8,
    },
    Reset {
        provider: ProviderId,
        window: WindowKind,
        window_description: Option<String>,
    },
}

pub fn evaluate_snapshot(
    snapshot: &UsageSnapshot,
    settings: &mut AlarmSettings,
) -> Vec<AlarmNotice> {
    let mut notices = Vec::new();

    for window in &snapshot.windows {
        let description = window.reset_description.as_deref();
        if let Some(config) = settings
            .threshold(&snapshot.provider, window.kind, description)
            .cloned()
        {
            if config.enabled {
                let prior = settings.threshold_state(&snapshot.provider, window.kind, description);
                let (crossings, state) = evaluate_thresholds(&config, window, &prior);

                for crossing in crossings {
                    notices.push(AlarmNotice::Threshold {
                        provider: crossing.provider,
                        window: crossing.window,
                        window_description: crossing.window_description,
                        level: crossing.level,
                    });
                }

                settings.set_threshold_state(&snapshot.provider, window.kind, description, state);
            }
        }

        if settings.reset_enabled(&snapshot.provider, window.kind, description) {
            let prior = settings.reset_state(&snapshot.provider, window.kind, description);
            let (fired, state) = detect_reset(window, &prior);
            if fired {
                notices.push(AlarmNotice::Reset {
                    provider: snapshot.provider.clone(),
                    window: window.kind,
                    window_description: window.reset_description.clone(),
                });
            }
            settings.set_reset_state(&snapshot.provider, window.kind, description, state);
        }
    }

    notices
}

/// A settings row keyed by `(provider, window kind, window description)`. The description is
/// what keeps two same-`kind` windows (e.g. Claude's Opus/Sonnet 7-day `Custom` windows)
/// distinct rows instead of one clobbering slot — the root cause this keying fixes.
trait Keyed {
    fn provider(&self) -> &ProviderId;
    fn window(&self) -> WindowKind;
    fn window_description(&self) -> Option<&str>;

    fn matches_key(
        &self,
        provider: &ProviderId,
        window: WindowKind,
        description: Option<&str>,
    ) -> bool {
        self.provider() == provider
            && self.window() == window
            && self.window_description() == description
    }
}

fn find_keyed<'a, T: Keyed>(
    items: &'a [T],
    provider: &ProviderId,
    window: WindowKind,
    description: Option<&str>,
) -> Option<&'a T> {
    items
        .iter()
        .find(|item| item.matches_key(provider, window, description))
}

fn find_keyed_mut<'a, T: Keyed>(
    items: &'a mut [T],
    provider: &ProviderId,
    window: WindowKind,
    description: Option<&str>,
) -> Option<&'a mut T> {
    items
        .iter_mut()
        .find(|item| item.matches_key(provider, window, description))
}

impl Keyed for ThresholdConfig {
    fn provider(&self) -> &ProviderId {
        &self.provider
    }
    fn window(&self) -> WindowKind {
        self.window
    }
    fn window_description(&self) -> Option<&str> {
        self.window_description.as_deref()
    }
}

impl Keyed for ResetPref {
    fn provider(&self) -> &ProviderId {
        &self.provider
    }
    fn window(&self) -> WindowKind {
        self.window
    }
    fn window_description(&self) -> Option<&str> {
        self.window_description.as_deref()
    }
}

impl Keyed for ThresholdStateEntry {
    fn provider(&self) -> &ProviderId {
        &self.provider
    }
    fn window(&self) -> WindowKind {
        self.window
    }
    fn window_description(&self) -> Option<&str> {
        self.window_description.as_deref()
    }
}

impl Keyed for ResetStateEntry {
    fn provider(&self) -> &ProviderId {
        &self.provider
    }
    fn window(&self) -> WindowKind {
        self.window
    }
    fn window_description(&self) -> Option<&str> {
        self.window_description.as_deref()
    }
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
        window_desc(kind, used_percent, resets_at, None)
    }

    fn window_desc(
        kind: WindowKind,
        used_percent: f64,
        resets_at: Option<Timestamp>,
        description: Option<&str>,
    ) -> UsageWindow {
        UsageWindow {
            kind,
            used_percent,
            window_minutes: None,
            resets_at,
            reset_description: description.map(str::to_owned),
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
    fn normalize_levels_keeps_valid_deduped_and_sorted() {
        assert_eq!(normalize_levels(&[95, 80, 80, 95]), vec![80, 95]);
        assert_eq!(normalize_levels(&[100, 1, 50]), vec![1, 50, 100]);
        // 0 (fires every poll) and >100 (never fires) are dropped, not clamped.
        assert_eq!(normalize_levels(&[0, 101, 200]), Vec::<u8>::new());
        assert_eq!(normalize_levels(&[]), Vec::<u8>::new());
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
    fn next_occurrence_saturates_instead_of_overflowing_on_extreme_inputs() {
        // ADR 0015: the pure core must never overflow-panic on untrusted input (e.g. an
        // out-of-range `fire_at` from a direct command invoke). A huge forward gap saturates
        // to i64::MAX rather than wrapping into a past instant.
        assert_eq!(
            next_occurrence(ts(0), ts(i64::MAX), Recurrence::Weekly).0,
            i64::MAX
        );
        // An i64::MIN anchor that is already due must compute without panicking.
        let advanced = next_occurrence(ts(i64::MIN), ts(0), Recurrence::Daily);
        assert!(advanced.0 > i64::MIN);
    }

    #[test]
    fn reconcile_does_not_panic_on_extreme_fire_times() {
        // The scheduler reconciles whatever is persisted; an adversarial alarm at i64::MIN must
        // advance via the saturating `next_occurrence` rather than overflow-panicking the task.
        let due = alarm("min", "Min", ts(i64::MIN), Some(Recurrence::Daily));
        let far = alarm("max", "Max", ts(i64::MAX), Some(Recurrence::Weekly));

        let reconciled = reconcile(ts(0), &[due, far], MissedPolicy::FireEach);

        // Only the due (i64::MIN) alarm fires and advances; the far-future one is untouched.
        assert_eq!(reconciled.updated.len(), 1);
        assert_eq!(reconciled.updated[0].id, AlarmId::new("min"));
        assert!(reconciled.updated[0].next_fire_at.0 > i64::MIN);
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
            window_description: None,
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
                window_description: None,
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
                window_description: None,
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
            window_description: None,
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
                    window_description: None,
                    level: 50,
                },
                ThresholdCrossing {
                    provider: config.provider.clone(),
                    window: WindowKind::Monthly,
                    window_description: None,
                    level: 90,
                },
            ]
        );
        assert_eq!(state.instance, Some(ts(200)));
        assert_eq!(state.fired_levels, vec![50, 90]);
    }

    #[test]
    fn evaluate_thresholds_rearms_when_usage_drops_below_lowest_level() {
        // Credit-style window: never exposes `resets_at`, so a reset can only be inferred
        // from usage falling back below the lowest configured level.
        let config = ThresholdConfig {
            provider: provider("openrouter"),
            window: WindowKind::Monthly,
            window_description: Some("Credit used".into()),
            levels: vec![80],
            enabled: true,
        };

        let (crossings, state) = evaluate_thresholds(
            &config,
            &window_desc(WindowKind::Monthly, 80.0, None, Some("Credit used")),
            &ThresholdState::default(),
        );
        assert_eq!(crossings.len(), 1);
        assert_eq!(state.fired_levels, vec![80]);

        // Stays armed while above the level.
        let (crossings, state) = evaluate_thresholds(
            &config,
            &window_desc(WindowKind::Monthly, 85.0, None, Some("Credit used")),
            &state,
        );
        assert!(crossings.is_empty());
        assert_eq!(state.fired_levels, vec![80]);

        // Drops below the lowest level -> re-arms (clears fired levels).
        let (crossings, state) = evaluate_thresholds(
            &config,
            &window_desc(WindowKind::Monthly, 10.0, None, Some("Credit used")),
            &state,
        );
        assert!(crossings.is_empty());
        assert!(state.fired_levels.is_empty());

        // Climbs back over -> fires again (the bug was: fired once, ever).
        let (crossings, _) = evaluate_thresholds(
            &config,
            &window_desc(WindowKind::Monthly, 90.0, None, Some("Credit used")),
            &state,
        );
        assert_eq!(crossings.len(), 1);
    }

    #[test]
    fn evaluate_thresholds_disabled_config_does_not_mutate_state() {
        let config = ThresholdConfig {
            provider: provider("claude"),
            window: WindowKind::Weekly,
            window_description: None,
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
            window_description: None,
            levels: vec![50, 90],
            enabled: true,
        });
        settings.set_threshold(ThresholdConfig {
            provider: claude.clone(),
            window: WindowKind::Monthly,
            window_description: None,
            levels: vec![75],
            enabled: false,
        });
        settings.set_threshold_state(
            &claude,
            WindowKind::Monthly,
            None,
            ThresholdState {
                instance: Some(ts(10)),
                fired_levels: vec![75],
            },
        );
        settings.set_reset_enabled(&claude, WindowKind::Weekly, None, true);
        settings.set_reset_state(
            &claude,
            WindowKind::Weekly,
            None,
            ResetState {
                last_resets_at: Some(ts(100)),
            },
        );
        settings.set_reset_enabled(&claude, WindowKind::Monthly, None, false);
        settings.set_reset_state(
            &claude,
            WindowKind::Monthly,
            None,
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
                    window_description: None,
                    level: 50,
                },
                AlarmNotice::Threshold {
                    provider: claude.clone(),
                    window: WindowKind::Weekly,
                    window_description: None,
                    level: 90,
                },
                AlarmNotice::Reset {
                    provider: claude.clone(),
                    window: WindowKind::Weekly,
                    window_description: None,
                },
            ]
        );
        assert_eq!(
            settings.threshold_state(&claude, WindowKind::Weekly, None),
            ThresholdState {
                instance: Some(ts(200)),
                fired_levels: vec![50, 90],
            }
        );
        assert_eq!(
            settings.reset_state(&claude, WindowKind::Weekly, None),
            ResetState {
                last_resets_at: Some(ts(200)),
            }
        );
        assert_eq!(
            settings.threshold_state(&claude, WindowKind::Monthly, None),
            ThresholdState {
                instance: Some(ts(10)),
                fired_levels: vec![75],
            }
        );
        assert_eq!(
            settings.reset_state(&claude, WindowKind::Monthly, None),
            ResetState {
                last_resets_at: Some(ts(100)),
            }
        );
    }

    #[test]
    fn evaluate_snapshot_keys_same_kind_windows_by_description() {
        // Reproduces the Blocker: Claude emits >=2 `Custom` 7-day windows. Keyed by `kind`
        // alone they clobber one config/state slot and re-fire every poll. Keyed by
        // description they are independent.
        let claude = provider("claude");
        let mut settings = AlarmSettings::default();
        settings.set_threshold(ThresholdConfig {
            provider: claude.clone(),
            window: WindowKind::Custom,
            window_description: Some("Opus · 7-day".into()),
            levels: vec![80],
            enabled: true,
        });
        settings.set_threshold(ThresholdConfig {
            provider: claude.clone(),
            window: WindowKind::Custom,
            window_description: Some("Sonnet · 7-day".into()),
            levels: vec![90],
            enabled: true,
        });

        let snap = snapshot(
            claude.clone(),
            vec![
                window_desc(
                    WindowKind::Custom,
                    85.0,
                    Some(ts(100)),
                    Some("Opus · 7-day"),
                ),
                window_desc(
                    WindowKind::Custom,
                    95.0,
                    Some(ts(200)),
                    Some("Sonnet · 7-day"),
                ),
            ],
        );

        // First poll: each window fires its own level exactly once.
        let notices = evaluate_snapshot(&snap, &mut settings);
        assert_eq!(
            notices,
            vec![
                AlarmNotice::Threshold {
                    provider: claude.clone(),
                    window: WindowKind::Custom,
                    window_description: Some("Opus · 7-day".into()),
                    level: 80,
                },
                AlarmNotice::Threshold {
                    provider: claude.clone(),
                    window: WindowKind::Custom,
                    window_description: Some("Sonnet · 7-day".into()),
                    level: 90,
                },
            ]
        );

        // State is held per window in distinct slots — no clobber.
        assert_eq!(settings.threshold_state.len(), 2);
        assert_eq!(
            settings
                .threshold_state(&claude, WindowKind::Custom, Some("Opus · 7-day"))
                .fired_levels,
            vec![80]
        );
        assert_eq!(
            settings
                .threshold_state(&claude, WindowKind::Custom, Some("Sonnet · 7-day"))
                .fired_levels,
            vec![90]
        );

        // Second identical poll: nothing re-fires (the storm the Blocker produced).
        let again = evaluate_snapshot(&snap, &mut settings);
        assert!(
            again.is_empty(),
            "same-kind windows must not re-fire on an unchanged poll"
        );
    }

    #[test]
    fn reconcile_coalesces_recurring_and_one_off_due_in_one_pass() {
        let recurring = alarm("weekly", "Weekly", ts(50), Some(Recurrence::Weekly));
        let one_off = alarm("once", "Once", ts(60), None);
        let now = ts(100);

        let reconciled = reconcile(
            now,
            &[recurring.clone(), one_off.clone()],
            MissedPolicy::Coalesce,
        );

        // Both due -> a single coalesced summary.
        match &reconciled.firing {
            Firing::Coalesced(due) => assert_eq!(due.len(), 2),
            other => panic!("expected coalesced firing, got {other:?}"),
        }
        // The recurring alarm advanced past now; the one-off completed.
        assert_eq!(reconciled.updated.len(), 1);
        assert_eq!(reconciled.updated[0].id, recurring.id);
        assert_eq!(
            reconciled.updated[0].next_fire_at,
            next_occurrence(ts(50), now, Recurrence::Weekly)
        );
        assert_eq!(reconciled.completed, vec![one_off.id]);
    }

    #[test]
    fn next_occurrence_advances_weekly_and_every_n_days_past_now() {
        let anchor = ts(0);
        assert_eq!(
            next_occurrence(anchor, ts(7 * DAY_MS), Recurrence::Weekly),
            ts(2 * 7 * DAY_MS)
        );
        assert_eq!(
            next_occurrence(anchor, ts(DAY_MS + 1), Recurrence::EveryNDays { days: 3 }),
            ts(3 * DAY_MS)
        );
        // Zero-day guard: degrades to a 1-day period rather than dividing by zero.
        assert_eq!(
            next_occurrence(anchor, ts(1), Recurrence::EveryNDays { days: 0 }),
            ts(DAY_MS)
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

        assert!(settings
            .threshold(&claude, WindowKind::Weekly, None)
            .is_none());
        assert!(!settings.reset_enabled(&claude, WindowKind::Weekly, None));
        assert_eq!(
            settings.threshold_state(&claude, WindowKind::Weekly, None),
            ThresholdState::default()
        );
        assert_eq!(
            settings.reset_state(&claude, WindowKind::Weekly, None),
            ResetState::default()
        );

        settings.set_threshold(ThresholdConfig {
            provider: claude.clone(),
            window: WindowKind::Weekly,
            window_description: None,
            levels: vec![50],
            enabled: true,
        });
        settings.set_threshold(ThresholdConfig {
            provider: claude.clone(),
            window: WindowKind::Weekly,
            window_description: None,
            levels: vec![80, 95],
            enabled: false,
        });
        settings.set_threshold(ThresholdConfig {
            provider: codex.clone(),
            window: WindowKind::Weekly,
            window_description: None,
            levels: vec![70],
            enabled: true,
        });

        assert_eq!(settings.thresholds.len(), 2);
        assert_eq!(
            settings
                .threshold(&claude, WindowKind::Weekly, None)
                .unwrap()
                .levels
                .as_slice(),
            &[80, 95]
        );
        assert!(
            settings
                .threshold(&codex, WindowKind::Weekly, None)
                .unwrap()
                .enabled
        );

        settings.set_reset_enabled(&claude, WindowKind::Weekly, None, true);
        settings.set_reset_enabled(&claude, WindowKind::Weekly, None, false);
        assert!(!settings.reset_enabled(&claude, WindowKind::Weekly, None));
        assert_eq!(settings.resets.len(), 1);

        settings.set_threshold_state(
            &claude,
            WindowKind::Weekly,
            None,
            ThresholdState {
                instance: Some(ts(10)),
                fired_levels: vec![80],
            },
        );
        settings.set_threshold_state(
            &claude,
            WindowKind::Weekly,
            None,
            ThresholdState {
                instance: Some(ts(20)),
                fired_levels: vec![95],
            },
        );
        settings.set_reset_state(
            &claude,
            WindowKind::Weekly,
            None,
            ResetState {
                last_resets_at: Some(ts(30)),
            },
        );

        assert_eq!(settings.threshold_state.len(), 1);
        assert_eq!(
            settings.threshold_state(&claude, WindowKind::Weekly, None),
            ThresholdState {
                instance: Some(ts(20)),
                fired_levels: vec![95],
            }
        );
        assert_eq!(
            settings.reset_state(&claude, WindowKind::Weekly, None),
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
