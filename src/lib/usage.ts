import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

// These mirror mlt-core's serde shapes by hand. ADR 0010 plans to replace this module with
// tauri-specta-generated bindings so the boundary is type-checked rather than hand-synced.
export type WindowKind = 'Session' | 'Weekly' | 'Monthly' | 'Custom';
export type Status = 'Ok' | 'Stale' | 'Error';

export interface UsageWindow {
  kind: WindowKind;
  used_percent: number;
  window_minutes: number | null;
  resets_at: number | null; // unix ms
  reset_description: string | null;
}

export interface UsageSnapshot {
  provider: string;
  windows: UsageWindow[];
  status: Status;
  fetched_at: number; // unix ms
}

export const fetchClaudeUsage = (): Promise<UsageSnapshot> => invoke('fetch_claude_usage');

export const onUsageUpdated = (cb: (s: UsageSnapshot) => void): Promise<UnlistenFn> =>
  listen<UsageSnapshot>('usage-updated', (e) => cb(e.payload));

export const onUsageError = (cb: (msg: string) => void): Promise<UnlistenFn> =>
  listen<string>('usage-error', (e) => cb(e.payload));
