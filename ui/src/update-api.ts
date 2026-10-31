import { invoke } from '@tauri-apps/api/core';
import type { LauncherUpdateView, UpdateChannel } from './update-state';

type UpdateArguments = {
  launcher_update_status: undefined;
  check_launcher_update: { force: boolean };
  set_update_channel: { channel: UpdateChannel };
  apply_launcher_update: undefined;
  cancel_launcher_update_apply: undefined;
  open_launcher_release_page: undefined;
};
type UpdateResults = {
  launcher_update_status: LauncherUpdateView;
  check_launcher_update: LauncherUpdateView;
  set_update_channel: LauncherUpdateView;
  apply_launcher_update: LauncherUpdateView;
  cancel_launcher_update_apply: LauncherUpdateView;
  open_launcher_release_page: void;
};
export function invokeUpdate<C extends keyof UpdateArguments>(command: C, args: UpdateArguments[C]): Promise<UpdateResults[C]> {
  return invoke<UpdateResults[C]>(command, args);
}
