import { invoke } from '@tauri-apps/api/core';
import type { GlobalLaunchSettings, InstanceSettings, SettingsSnapshot } from './settings-state';

export const settingsApi = {
  snapshot: (instanceId: string | null) => invoke<SettingsSnapshot>('settings_snapshot', { instanceId }),
  saveGlobal: (settings: GlobalLaunchSettings) => invoke<void>('save_global_settings', { settings }),
  saveInstance: (instanceId: string, settings: InstanceSettings) => invoke<void>('save_instance_settings', { instanceId, settings }),
  select: (instanceId: string) => invoke<void>('select_instance', { instanceId }),
};
