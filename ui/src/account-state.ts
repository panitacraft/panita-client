export type AccountStatus = 'offline' | 'waiting_for_browser' | 'authenticating' | 'ready' | 'sign_in_required' | 'error';
export type AccountView = {
  metadata: { account_id: string; kind: 'microsoft' | 'offline'; username: string; profile_id: string; oauth_client_id: string | null };
  status: AccountStatus;
  error: string | null;
  persisted: boolean;
};
export type AccountsView = {
  revision: number;
  accounts: AccountView[];
  selected_account_id: string | null;
  login_pending: boolean;
  login_error: string | null;
};

export type AccountFeedback = {
  view: AccountsView | null;
  operation: number;
  busy: string | null;
  error: string | null;
  notice: string | null;
  blocked: boolean;
  cleanupWarning: string | null;
};
export const initialAccountFeedback: AccountFeedback = {
  view: null, operation: 0, busy: null, error: null, notice: null, blocked: false, cleanupWarning: null,
};
type FeedbackAction =
  | { type: 'snapshot'; view: AccountsView }
  | { type: 'start'; operation: number; command: string }
  | { type: 'finish'; operation: number; error?: string | null; notice?: string | null; selectedAccountId?: string | null }
  | { type: 'initialization_failed'; error: string; blocked: boolean }
  | { type: 'cleanup_warning'; warning: string };

export function accountFeedback(state: AccountFeedback, action: FeedbackAction): AccountFeedback {
  switch (action.type) {
    case 'snapshot': {
      if (state.view && state.view.revision > action.view.revision) return state;
      const changedAccount = state.view?.selected_account_id !== action.view.selected_account_id;
      return {
        ...state, view: action.view,
        busy: state.busy === 'initialize' ? null : state.busy,
        blocked: false,
        error: changedAccount || state.busy === 'initialize' ? null : state.error,
        notice: changedAccount || action.view.login_error || action.view.login_pending ? null : state.notice,
      };
    }
    case 'start':
      return { ...state, operation: action.operation, busy: action.command, error: null, notice: null };
    case 'finish':
      if (action.operation !== state.operation) return state;
      // Network work for an earlier selection may complete after an account event.
      if (action.selectedAccountId !== undefined && action.selectedAccountId !== state.view?.selected_account_id) {
        return { ...state, busy: null };
      }
      return { ...state, busy: null, error: action.error ?? null, notice: action.notice ?? null };
    case 'initialization_failed':
      return { ...state, busy: null, error: action.error, blocked: action.blocked };
    case 'cleanup_warning':
      // Selection and unrelated successful operations cannot prove durable cleanup.
      return { ...state, cleanupWarning: action.warning };
  }
}
