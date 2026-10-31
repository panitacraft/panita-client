import { invoke, isTauri } from '@tauri-apps/api/core';
import type { DiagnosticSnapshot } from './diagnostic-state';

export function openLogsFolder(): Promise<boolean> {
  return isTauri() ? invoke<boolean>('open_logs_folder') : Promise.resolve(false);
}

export function loadDiagnostics(): Promise<DiagnosticSnapshot> {
  // Deliberately independent of initialize: storage failure must remain visible.
  return isTauri() ? invoke<DiagnosticSnapshot>('diagnostic_snapshot') : Promise.resolve({ capacity: 256, records: [] });
}
