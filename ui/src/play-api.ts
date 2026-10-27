import { invoke } from '@tauri-apps/api/core';

type InstanceRequest = { instanceId: string; requestId: string };
export type PlayArguments = {
  begin_play: InstanceRequest;
  begin_install: InstanceRequest;
  begin_repair: InstanceRequest;
  begin_uninstall: InstanceRequest;
  begin_update: InstanceRequest & { update: { from_version: string; to_version: string; backup_worlds: boolean } };
  cancel_play: InstanceRequest;
  confirm_play: InstanceRequest & { confirmationId: string };
  stop_game: InstanceRequest & { force: boolean };
};
export type PlayCommand = keyof PlayArguments;
export function invokePlay<C extends PlayCommand>(command: C, args: PlayArguments[C]): Promise<void> {
  return invoke<void>(command, args);
}
