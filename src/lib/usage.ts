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

// A provider account's identity, fetched from the provider (never user-entered) so the UI can
// show *which* account a panel reports. Siloed per provider; any field the provider omits is null.
export interface AccountIdentity {
  email: string | null;
  organization: string | null;
}

export interface UsageSnapshot {
  provider: string;
  windows: UsageWindow[];
  status: Status;
  fetched_at: number; // unix ms
  // Which account this snapshot reports (email/org), or null when unknown. Provider-fetched.
  account: AccountIdentity | null;
}

// A row of the connect/sources screen (mirrors mlt-core's `SourceState`). `present` is
// metadata-only discovery; `enabled` is the user's opt-in. `credential` says how the source
// connects: a `LocalLogin` source reuses a login found on the machine (toggle on/off), while
// an `ApiKey` source is connected by storing a validated key — there is nothing to detect, so
// `enabled` alone means "a key is stored". The key is never sent back over this boundary.
export type CredentialKind = 'LocalLogin' | 'ApiKey';

export interface SourceState {
  id: string;
  display_name: string;
  access_note: string;
  present: boolean;
  enabled: boolean;
  credential: CredentialKind;
  // A user-assigned custom name (nickname/title), shown as the panel title, or null for none.
  label: string | null;
  // Provider-fetched account identity (email/org) for display, or null if not resolved yet.
  account: AccountIdentity | null;
}

export const fetchClaudeUsage = (): Promise<UsageSnapshot> => invoke('fetch_claude_usage');

// Quit the whole app (the tray right-click menu offers the same action).
export const quitApp = (): Promise<void> => invoke('quit');

// Discover local sources (presence + consent). Reads no secret.
export const listSources = (): Promise<SourceState[]> => invoke('list_sources');

// Opt a local-login source in/out. Takes effect immediately; returns the refreshed list.
export const setSourceEnabled = (id: string, enabled: boolean): Promise<SourceState[]> =>
  invoke('set_source_enabled', { id, enabled });

// Enter or replace an API key for a source that needs one. The backend validates the key
// against the provider before storing it (in the OS keychain only) — a rejected key throws
// with a clear message and the source stays disconnected. Returns the refreshed source list.
export const setApiKey = (id: string, key: string): Promise<SourceState[]> =>
  invoke('set_api_key', { id, key });

// Remove a stored API key and disconnect the source. Returns the refreshed source list.
export const removeApiKey = (id: string): Promise<SourceState[]> =>
  invoke('remove_api_key', { id });

// Set (or clear, with an empty string) a source's display name. Returns the refreshed list.
export const setSourceLabel = (id: string, name: string): Promise<SourceState[]> =>
  invoke('set_source_label', { id, name });

export const onUsageUpdated = (cb: (s: UsageSnapshot) => void): Promise<UnlistenFn> =>
  listen<UsageSnapshot>('usage-updated', (e) => cb(e.payload));

export const onUsageError = (cb: (msg: string) => void): Promise<UnlistenFn> =>
  listen<string>('usage-error', (e) => cb(e.payload));
