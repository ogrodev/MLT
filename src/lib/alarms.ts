import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { WindowKind } from './usage';

export type Recurrence =
  | { kind: 'daily' }
  | { kind: 'weekly' }
  | { kind: 'every_n_days'; days: number };
export type MissedPolicy = 'fire_each' | 'coalesce';
export interface Alarm {
  id: string;
  label: string;
  next_fire_at: number;
  recurrence: Recurrence | null;
}
export interface ThresholdConfig {
  provider: string;
  window: WindowKind;
  window_description: string | null;
  levels: number[];
  enabled: boolean;
}
export interface ResetPref {
  provider: string;
  window: WindowKind;
  window_description: string | null;
  enabled: boolean;
}
export interface AlarmPrefs {
  thresholds: ThresholdConfig[];
  resets: ResetPref[];
  missed_policy: MissedPolicy;
}

export const listAlarms = (): Promise<Alarm[]> => invoke('list_alarms');
export const createAlarm = (
  label: string,
  fireAt: number,
  recurrence: Recurrence | null,
): Promise<Alarm[]> => invoke('create_alarm', { label, fireAt, recurrence });
export const updateAlarm = (
  id: string,
  label: string,
  fireAt: number,
  recurrence: Recurrence | null,
): Promise<Alarm[]> => invoke('update_alarm', { id, label, fireAt, recurrence });
export const deleteAlarm = (id: string): Promise<Alarm[]> => invoke('delete_alarm', { id });
export const getAlarmPrefs = (): Promise<AlarmPrefs> => invoke('get_alarm_prefs');
export const setThresholdAlert = (
  provider: string,
  window: WindowKind,
  windowDescription: string | null,
  levels: number[],
  enabled: boolean,
): Promise<AlarmPrefs> =>
  invoke('set_threshold_alert', { provider, window, windowDescription, levels, enabled });
export const setResetNotification = (
  provider: string,
  window: WindowKind,
  windowDescription: string | null,
  enabled: boolean,
): Promise<AlarmPrefs> =>
  invoke('set_reset_notification', { provider, window, windowDescription, enabled });
export const setMissedPolicy = (policy: MissedPolicy): Promise<AlarmPrefs> =>
  invoke('set_missed_policy', { policy });
export const onAlarmsUpdated = (cb: (a: Alarm[]) => void): Promise<UnlistenFn> =>
  listen<Alarm[]>('alarms-updated', (e) => cb(e.payload));
