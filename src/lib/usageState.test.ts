import { describe, expect, it } from 'vitest';

import type { SourceState, UsageSnapshot } from './usage';
import {
  clearUsage,
  connectionState,
  errorFor,
  recordUsage,
  recordUsageError,
  reportsUsage,
  resetCountdown,
  selectedAccount,
  snapshotFor,
  sourceActive,
  sourceTabLabel,
  type UsageRecords,
  usageWindowKey,
} from './usageState';

function source(overrides: Partial<SourceState>): SourceState {
  return {
    id: 'codex:acct-1',
    display_name: 'Codex',
    access_note: 'note',
    present: true,
    enabled: true,
    credential: 'LocalLogin',
    label: null,
    account: null,
    ...overrides,
  };
}

function snapshot(provider: string, email: string | null): UsageSnapshot {
  return {
    provider,
    windows: [
      {
        kind: 'Session',
        used_percent: 42,
        window_minutes: 300,
        resets_at: 1_700_003_600_000,
        reset_description: null,
      },
    ],
    status: 'Ok',
    fetched_at: 1_700_000_000_000,
    account: email ? { email, organization: null } : null,
    note: null,
  };
}

function records(): UsageRecords {
  return { snapshots: {}, errors: {} };
}

describe('usage state', () => {
  it('keeps snapshots, errors, and identity siloed by provider id', () => {
    const state = records();
    const claude = source({
      id: 'claude-code',
      display_name: 'Claude Code',
      account: null,
    });
    const codex = source({ id: 'codex:acct-1', account: null });

    recordUsage(state, snapshot('claude-code', 'claude@example.com'));
    recordUsageError(state, { provider: 'codex:acct-1', message: 'offline' });

    expect(snapshotFor(state, 'claude-code')?.account?.email).toBe('claude@example.com');
    expect(errorFor(state, 'codex:acct-1')).toBe('offline');
    expect(selectedAccount(snapshotFor(state, 'claude-code'), claude)).toBe('claude@example.com');
    expect(selectedAccount(snapshotFor(state, 'codex:acct-1'), codex)).toBeNull();
  });

  it('never renders a cross-provider snapshot identity from selectedAccount', () => {
    const codex = source({
      id: 'codex:acct-1',
      account: { email: 'codex@example.com', organization: null },
    });
    // A snapshot belonging to another provider must not leak its identity here.
    expect(selectedAccount(snapshot('claude-code', 'claude@example.com'), codex)).toBe(
      'codex@example.com',
    );
    // The matching-provider snapshot identity is still preferred.
    expect(selectedAccount(snapshot('codex:acct-1', 'codex-live@example.com'), codex)).toBe(
      'codex-live@example.com',
    );
  });

  it('clears only the provider that receives a fresh usage snapshot', () => {
    const state = records();
    recordUsageError(state, { provider: 'claude-code', message: 'claude failed' });
    recordUsageError(state, { provider: 'codex:acct-1', message: 'codex failed' });

    recordUsage(state, snapshot('codex:acct-1', 'codex@example.com'));

    expect(errorFor(state, 'codex:acct-1')).toBeNull();
    expect(errorFor(state, 'claude-code')).toBe('claude failed');
    expect(snapshotFor(state, 'codex:acct-1')?.account?.email).toBe('codex@example.com');
  });

  it('reports stale/error connection states without calling a connected source disconnected', () => {
    const codex = source({ id: 'codex:acct-1' });

    expect(connectionState(codex, null, 'offline')).toEqual({ label: 'Error', tone: 'err' });
    expect(connectionState(codex, snapshot('codex:acct-1', null), 'offline')).toEqual({
      label: 'Stale',
      tone: 'warn',
    });
    // OpenRouter now reports usage, so before its first fetch it reads as "Connecting…" — not
    // the generic "Connected" shown for providers that have no usage tracking.
    expect(connectionState(source({ id: 'openrouter', credential: 'ApiKey' }), null, null)).toEqual(
      {
        label: 'Connecting…',
        tone: 'idle',
      },
    );
  });

  it('matches backend usage routing and source activation rules', () => {
    expect(reportsUsage('claude-code')).toBe(true);
    expect(reportsUsage('codex:acct-1')).toBe(true);
    expect(reportsUsage('claude-code:acct-2')).toBe(true);
    expect(reportsUsage('openrouter')).toBe(true);
    expect(reportsUsage('openai')).toBe(true);
    expect(reportsUsage('anthropic')).toBe(true);

    expect(sourceActive(source({ credential: 'LocalLogin', present: true, enabled: true }))).toBe(
      true,
    );
    expect(sourceActive(source({ credential: 'LocalLogin', present: false, enabled: true }))).toBe(
      false,
    );
    expect(sourceActive(source({ credential: 'ApiKey', present: false, enabled: true }))).toBe(
      true,
    );
  });

  it('uses account organization to disambiguate per-account tab labels', () => {
    expect(
      sourceTabLabel(
        source({
          account: { email: null, organization: 'Acme Team' },
        }),
      ),
    ).toBe('Acme Team');
    expect(
      sourceTabLabel(
        source({
          account: { email: 'codex@example.com', organization: 'Acme Team' },
        }),
      ),
    ).toBe('codex@example.com');
    expect(sourceTabLabel(source({ label: 'Work Codex' }))).toBe('Work Codex');
    expect(
      sourceTabLabel(
        source({
          id: 'openrouter',
          display_name: 'OpenRouter',
          account: { email: null, organization: 'Acme Team' },
        }),
      ),
    ).toBe('OpenRouter');
  });

  it('keys duplicate custom usage windows without collisions', () => {
    const first = snapshot('codex:acct-1', null).windows[0];
    const second = { ...first, kind: 'Custom' as const, reset_description: null };
    const third = { ...second };

    expect(usageWindowKey(second, 0)).toBe('Custom::0');
    expect(usageWindowKey(third, 1)).toBe('Custom::1');
    expect(usageWindowKey(second, 0)).not.toBe(usageWindowKey(third, 1));
  });

  it('formats reset countdowns from the current clock', () => {
    const now = 1_700_000_000_000;

    expect(resetCountdown(null, now)).toBe('');
    expect(resetCountdown(now - 1, now)).toBe('resetting…');
    expect(resetCountdown(now + 45 * 60_000, now)).toBe('resets in 45m');
    expect(resetCountdown(now + 3 * 60 * 60_000 + 7 * 60_000, now)).toBe('resets in 3h 7m');
    expect(resetCountdown(now + 2 * 24 * 60 * 60_000 + 4 * 60 * 60_000, now)).toBe(
      'resets in 2d 4h',
    );
  });

  it('clears one disconnected provider without touching another provider', () => {
    const state = records();
    recordUsage(state, snapshot('claude-code', 'claude@example.com'));
    recordUsage(state, snapshot('codex:acct-1', 'codex@example.com'));
    recordUsageError(state, { provider: 'codex:acct-1', message: 'offline' });

    clearUsage(state, 'codex:acct-1');

    expect(snapshotFor(state, 'codex:acct-1')).toBeNull();
    expect(errorFor(state, 'codex:acct-1')).toBeNull();
    expect(snapshotFor(state, 'claude-code')?.account?.email).toBe('claude@example.com');
  });
});
