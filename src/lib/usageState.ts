import type { SourceState, UsageErrorEvent, UsageSnapshot, UsageWindow } from './usage';

export type Tone = 'ok' | 'warn' | 'err' | 'idle';

export interface UsageRecords {
  snapshots: Record<string, UsageSnapshot>;
  errors: Record<string, string>;
}

const STATUS_CONN: Record<UsageSnapshot['status'], { label: string; tone: Tone }> = {
  Ok: { label: 'Connected', tone: 'ok' },
  Stale: { label: 'Stale', tone: 'warn' },
  Error: { label: 'Error', tone: 'err' },
};

export function sourceActive(source: SourceState): boolean {
  return source.credential === 'ApiKey' ? source.enabled : source.present && source.enabled;
}

export function sourceTabLabel(source: SourceState): string {
  if (source.label) return source.label;
  if (source.id.includes(':')) {
    return source.account?.email ?? source.account?.organization ?? source.display_name;
  }
  return source.display_name;
}

export function reportsUsage(id: string): boolean {
  return (
    id === 'claude-code' ||
    id === 'openrouter' ||
    id.startsWith('codex:') ||
    id.startsWith('claude-code:')
  );
}

export function resetCountdown(resetsAt: number | null, now: number): string {
  if (resetsAt == null) return '';
  const ms = resetsAt - now;
  if (ms <= 0) return 'resetting…';
  const mins = Math.floor(ms / 60000);
  const d = Math.floor(mins / 1440);
  const h = Math.floor((mins % 1440) / 60);
  const m = mins % 60;
  if (d > 0) return `resets in ${d}d ${h}h`;
  if (h > 0) return `resets in ${h}h ${m}m`;
  return `resets in ${m}m`;
}

export function usageWindowKey(window: UsageWindow, index: number): string {
  return `${window.kind}:${window.reset_description ?? ''}:${index}`;
}

export function recordUsage(records: UsageRecords, snapshot: UsageSnapshot): void {
  records.snapshots[snapshot.provider] = snapshot;
  delete records.errors[snapshot.provider];
}

export function recordUsageError(records: UsageRecords, event: UsageErrorEvent): void {
  records.errors[event.provider] = event.message;
}

export function clearUsage(records: UsageRecords, provider: string): void {
  delete records.snapshots[provider];
  delete records.errors[provider];
}

export function snapshotFor(records: UsageRecords, provider: string): UsageSnapshot | null {
  return records.snapshots[provider] ?? null;
}

export function errorFor(records: UsageRecords, provider: string): string | null {
  return records.errors[provider] ?? null;
}

export function selectedAccount(
  snapshot: UsageSnapshot | null,
  selected: SourceState | null,
): string | null {
  if (!selected) return null;
  // Siloing invariant: only surface the snapshot's identity when it belongs to the
  // selected provider; never render one provider's identity under another. A mismatched
  // snapshot falls through to the source's own cached account.
  if (snapshot?.provider === selected.id && snapshot.account) {
    return snapshot.account.email ?? snapshot.account.organization;
  }
  return selected.account?.email ?? selected.account?.organization ?? null;
}

export function connectionState(
  selected: SourceState | null,
  snapshot: UsageSnapshot | null,
  error: string | null,
): { label: string; tone: Tone } {
  if (!selected) return { label: 'Not connected', tone: 'idle' };
  if (!reportsUsage(selected.id)) return { label: 'Connected', tone: 'ok' };
  if (error) return snapshot ? { label: 'Stale', tone: 'warn' } : { label: 'Error', tone: 'err' };
  if (snapshot) return STATUS_CONN[snapshot.status];
  return { label: 'Connecting…', tone: 'idle' };
}
