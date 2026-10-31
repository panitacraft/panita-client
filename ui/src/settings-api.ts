import { invoke } from '@tauri-apps/api/core';
import type { GlobalLaunchSettings, InstanceSettings, SettingsSnapshot, TrayPreferences } from './settings-state';

export const settingsApi = {
  snapshot: (instanceId: string | null) => invoke<SettingsSnapshot>('settings_snapshot', { instanceId }),
  saveGlobal: (settings: GlobalLaunchSettings) => invoke<void>('save_global_settings', { settings }),
  saveInstance: (instanceId: string, settings: InstanceSettings) => invoke<void>('save_instance_settings', { instanceId, settings }),
  saveTray: (tray: TrayPreferences) => invoke<void>('save_tray_preferences', { closeToTray: tray.closeToTray, hideOnGameStart: tray.hideOnGameStart, reopenOnGameExit: tray.reopenOnGameExit }),
  select: (instanceId: string) => invoke<void>('select_instance', { instanceId }),
};
