import { invoke, isTauri } from '@tauri-apps/api/core';
import type { DiagnosticSnapshot } from './diagnostic-state';

export function openLogsFolder(): Promise<boolean> {
  return isTauri() ? invoke<boolean>('open_logs_folder') : Promise.resolve(false);
}

export type ClearedLogs = { removed: number; failed: number };

/** Deletes the logs of previous sessions. The running session's file is never touched. */
export function clearOldLogs(): Promise<ClearedLogs> {
  return isTauri() ? invoke<ClearedLogs>('clear_old_logs') : Promise.resolve({ removed: 0, failed: 0 });
}

export function loadDiagnostics(): Promise<DiagnosticSnapshot> {
  // Deliberately independent of initialize: storage failure must remain visible.
  return isTauri() ? invoke<DiagnosticSnapshot>('diagnostic_snapshot') : Promise.resolve({ capacity: 256, records: [] });
}
