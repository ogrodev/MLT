import { describe, expect, it } from 'vitest';

import type { Alarm, AlarmPrefs } from './alarms';
import {
  describeRecurrence,
  fireCountdown,
  formatLevels,
  parseLevels,
  recurrenceFromForm,
  resetEnabledFor,
  sortAlarms,
  thresholdFor,
  validateAlarmDraft,
} from './alarmsState';

function alarm(overrides: Partial<Alarm>): Alarm {
  return {
    id: 'alarm-1',
    label: 'Alarm',
    next_fire_at: 1_700_000_000_000,
    recurrence: null,
    ...overrides,
  };
}

function prefs(overrides: Partial<AlarmPrefs>): AlarmPrefs {
  return {
    thresholds: [],
    resets: [],
    missed_policy: 'fire_each',
    ...overrides,
  };
}

describe('alarm state', () => {
  it('describes one-off and recurring schedules', () => {
    expect(describeRecurrence(null)).toBe('One-off');
    expect(describeRecurrence({ kind: 'daily' })).toBe('Daily');
    expect(describeRecurrence({ kind: 'weekly' })).toBe('Weekly');
    expect(describeRecurrence({ kind: 'every_n_days', days: 1 })).toBe('Every day');
    expect(describeRecurrence({ kind: 'every_n_days', days: 3 })).toBe('Every 3 days');
  });

  it('builds recurrence values from form controls', () => {
    expect(recurrenceFromForm('once', 7)).toBeNull();
    expect(recurrenceFromForm('daily', 7)).toEqual({ kind: 'daily' });
    expect(recurrenceFromForm('weekly', 7)).toEqual({ kind: 'weekly' });
    expect(recurrenceFromForm('every_n', 3.9)).toEqual({ kind: 'every_n_days', days: 3 });
    expect(recurrenceFromForm('every_n', 0)).toEqual({ kind: 'every_n_days', days: 1 });
  });

  it('validates labels and future fire times', () => {
    const now = 1_700_000_000_000;

    expect(validateAlarmDraft('  ', now + 60_000, now)).toBe('Enter a label');
    expect(validateAlarmDraft('Stand up', now, now)).toBe('Pick a time in the future');
    expect(validateAlarmDraft('Stand up', now - 1, now)).toBe('Pick a time in the future');
    expect(validateAlarmDraft('Stand up', Number.POSITIVE_INFINITY, now)).toBe(
      'Pick a time in the future',
    );
    expect(validateAlarmDraft('Stand up', now + 1, now)).toBeNull();
  });

  it('returns a new alarm array sorted by next fire time without reordering ties', () => {
    const late = alarm({ id: 'late', next_fire_at: 300 });
    const sameA = alarm({ id: 'same-a', next_fire_at: 200 });
    const early = alarm({ id: 'early', next_fire_at: 100 });
    const sameB = alarm({ id: 'same-b', next_fire_at: 200 });
    const input = [late, sameA, early, sameB];

    const sorted = sortAlarms(input);

    expect(sorted).not.toBe(input);
    expect(sorted.map((item) => item.id)).toEqual(['early', 'same-a', 'same-b', 'late']);
    expect(input.map((item) => item.id)).toEqual(['late', 'same-a', 'early', 'same-b']);
  });

  it('formats fire countdowns from the current clock', () => {
    const now = 1_700_000_000_000;

    expect(fireCountdown(now, now)).toBe('now');
    expect(fireCountdown(now - 1, now)).toBe('now');
    expect(fireCountdown(now + 5 * 60_000, now)).toBe('5m');
    expect(fireCountdown(now + 3 * 60 * 60_000 + 20 * 60_000, now)).toBe('3h 20m');
    expect(fireCountdown(now + 2 * 24 * 60 * 60_000 + 4 * 60 * 60_000, now)).toBe('2d 4h');
  });

  it('finds threshold prefs by provider and window', () => {
    const weekly = {
      provider: 'claude-code',
      window: 'Weekly' as const,
      levels: [80, 95],
      enabled: true,
    };
    const settings = prefs({ thresholds: [weekly] });

    expect(thresholdFor(settings, 'claude-code', 'Weekly')).toBe(weekly);
    expect(thresholdFor(settings, 'codex', 'Weekly')).toBeNull();
    expect(thresholdFor(settings, 'claude-code', 'Monthly')).toBeNull();
  });

  it('defaults reset notifications off unless the matching pref is enabled', () => {
    const settings = prefs({
      resets: [
        { provider: 'claude-code', window: 'Weekly', enabled: true },
        { provider: 'claude-code', window: 'Monthly', enabled: false },
      ],
    });

    expect(resetEnabledFor(settings, 'claude-code', 'Weekly')).toBe(true);
    expect(resetEnabledFor(settings, 'claude-code', 'Monthly')).toBe(false);
    expect(resetEnabledFor(settings, 'codex', 'Weekly')).toBe(false);
  });

  it('formats threshold levels for display', () => {
    expect(formatLevels([80, 95])).toBe('80%, 95%');
    expect(formatLevels([])).toBe('');
  });

  it('parses threshold levels by clamping, deduping, and sorting valid integers', () => {
    expect(parseLevels('95, 80, 80, 0, 200')).toEqual([80, 95]);
    expect(parseLevels('100\n1 nope 50')).toEqual([1, 50, 100]);
    expect(parseLevels('')).toEqual([]);
  });
});
