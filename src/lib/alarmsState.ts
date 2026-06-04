import type { Alarm, AlarmPrefs, Recurrence, ThresholdConfig } from './alarms';
import type { WindowKind } from './usage';

export function describeRecurrence(r: Recurrence | null): string {
  if (!r) return 'One-off';

  switch (r.kind) {
    case 'daily':
      return 'Daily';
    case 'weekly':
      return 'Weekly';
    case 'every_n_days':
      return r.days === 1 ? 'Every day' : `Every ${r.days} days`;
    default: {
      r satisfies never;
      return '';
    }
  }
}

export function recurrenceFromForm(
  mode: 'once' | 'daily' | 'weekly' | 'every_n',
  everyNDays: number,
): Recurrence | null {
  switch (mode) {
    case 'once':
      return null;
    case 'daily':
      return { kind: 'daily' };
    case 'weekly':
      return { kind: 'weekly' };
    case 'every_n':
      return { kind: 'every_n_days', days: Math.max(1, Math.floor(everyNDays)) };
    default: {
      mode satisfies never;
      return null;
    }
  }
}

export function validateAlarmDraft(label: string, fireAt: number, now: number): string | null {
  if (!label.trim()) return 'Enter a label';
  if (!Number.isFinite(fireAt) || fireAt <= now) return 'Pick a time in the future';
  return null;
}

export function sortAlarms(alarms: Alarm[]): Alarm[] {
  return [...alarms].sort((a, b) => a.next_fire_at - b.next_fire_at);
}

export function fireCountdown(nextFireAt: number, now: number): string {
  const ms = nextFireAt - now;
  if (ms <= 0) return 'now';
  const mins = Math.floor(ms / 60000);
  const d = Math.floor(mins / 1440);
  const h = Math.floor((mins % 1440) / 60);
  const m = mins % 60;
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

export function thresholdFor(
  prefs: AlarmPrefs,
  provider: string,
  window: WindowKind,
): ThresholdConfig | null {
  return (
    prefs.thresholds.find(
      (threshold) => threshold.provider === provider && threshold.window === window,
    ) ?? null
  );
}

export function resetEnabledFor(prefs: AlarmPrefs, provider: string, window: WindowKind): boolean {
  return prefs.resets.some(
    (reset) => reset.provider === provider && reset.window === window && reset.enabled === true,
  );
}

export function formatLevels(levels: number[]): string {
  return levels.map((level) => `${level}%`).join(', ');
}

export function parseLevels(input: string): number[] {
  const levels = new Set<number>();

  for (const token of input.split(/[\s,]+/)) {
    const level = Number.parseInt(token, 10);
    if (Number.isNaN(level) || level < 1 || level > 100) continue;
    levels.add(level);
  }

  return [...levels].sort((a, b) => a - b);
}
