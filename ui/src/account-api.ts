import { invoke } from '@tauri-apps/api/core';
import type { AccountsView } from './account-state';

// Account metadata and the explicitly requested outbound login link may cross IPC.
// Credentials, callback data and the PKCE verifier stay in Rust.
export type AccountArguments = {
  initialize: undefined;
  account_snapshot: undefined;
  add_offline_account: { username: string };
  start_microsoft_login: { accountId: string | null };
  cancel_microsoft_login: undefined;
  microsoft_login_link: undefined;
  select_account: { accountId: string };
  renew_account: undefined;
  retry_account_save: { accountId: string };
  remove_account: { accountId: string };
  validate_account: undefined;
};
type AccountResults = {
  initialize: void;
  account_snapshot: AccountsView;
  add_offline_account: void;
  start_microsoft_login: string;
  cancel_microsoft_login: void;
  microsoft_login_link: string;
  select_account: void;
  renew_account: void;
  retry_account_save: void;
  remove_account: 'removed' | 'pending' | 'not_durable';
  validate_account: void;
};
export function invokeAccount<C extends keyof AccountArguments>(command: C, args: AccountArguments[C]): Promise<AccountResults[C]> {
  return invoke<AccountResults[C]>(command, args);
}
