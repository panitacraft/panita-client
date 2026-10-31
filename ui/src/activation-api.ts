import { invoke } from '@tauri-apps/api/core';
import type { ActivationView } from './activation-state';

export type ActivationArguments = {
  activation_snapshot: undefined;
  start_activation: { identifier: string };
  cancel_activation: undefined;
  retry_activation_save: undefined;
  refresh_activation: undefined;
  sign_out_activation: undefined;
  open_panita_bot: undefined;
};
type ActivationResults = {
  activation_snapshot: ActivationView;
  start_activation: void;
  cancel_activation: void;
  retry_activation_save: void;
  refresh_activation: void;
  // False when the service could not be reached: the session stays open on the server.
  sign_out_activation: { serverClosed: boolean };
  open_panita_bot: void;
};
export function invokeActivation<C extends keyof ActivationArguments>(command: C, args: ActivationArguments[C]): Promise<ActivationResults[C]> {
  return invoke<ActivationResults[C]>(command, args);
}
