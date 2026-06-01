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

// A row of the connect/sources screen (mirrors mlt-core's `SourceState`). `present` is
// metadata-only discovery; `enabled` is the user's opt-in. The app reads a source's
// credentials only when both are true.
export interface SourceState {
  id: string;
  display_name: string;
  access_note: string;
  present: boolean;
  enabled: boolean;
}

export const fetchClaudeUsage = (): Promise<UsageSnapshot> => invoke('fetch_claude_usage');

// Quit the whole app (the tray right-click menu offers the same action).
export const quitApp = (): Promise<void> => invoke('quit');

// Discover local sources (presence + consent). Reads no secret.
export const listSources = (): Promise<SourceState[]> => invoke('list_sources');

// Opt a source in/out. Takes effect immediately; returns the refreshed source list.
export const setSourceEnabled = (id: string, enabled: boolean): Promise<SourceState[]> =>
  invoke('set_source_enabled', { id, enabled });

export const onUsageUpdated = (cb: (s: UsageSnapshot) => void): Promise<UnlistenFn> =>
  listen<UsageSnapshot>('usage-updated', (e) => cb(e.payload));

export const onUsageError = (cb: (msg: string) => void): Promise<UnlistenFn> =>
  listen<string>('usage-error', (e) => cb(e.payload));
