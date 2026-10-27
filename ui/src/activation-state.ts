export type ActivationStateName = 'no_session' | 'creating' | 'pending' | 'active' | 'active_offline';
export type PanitaUser = {
  id: number;
  ign: string | null;
  discord_name: string;
  avatar_url: string | null;
  minecraft_status: 'premium' | 'cracked' | null;
  capabilities: Record<string, boolean>;
};
// Only the nonsecret snapshot crosses this boundary. The session token stays in Rust.
export type ActivationView = {
  revision: number;
  state: ActivationStateName;
  user: PanitaUser | null;
  verified_at: number | null;
  serial: string | null;
  expires_at: number | null;
  persisted: boolean;
  error: string | null;
  retry_after: number | null;
};

export type ActivationFeedback = {
  view: ActivationView | null;
  operation: number;
  busy: string | null;
  error: string | null;
  notice: string | null;
  initializationError: string | null;
};
export const initialActivationFeedback: ActivationFeedback = {
  view: null, operation: 0, busy: null, error: null, notice: null, initializationError: null,
};
type FeedbackAction =
  | { type: 'snapshot'; view: ActivationView }
  | { type: 'start'; operation: number; command: string }
  | { type: 'finish'; operation: number; error?: string | null; notice?: string | null }
  | { type: 'initialization_failed'; error: string }
  | { type: 'retry_initialization' };

export function activationFeedback(state: ActivationFeedback, action: FeedbackAction): ActivationFeedback {
  switch (action.type) {
    case 'snapshot': {
      // A late first snapshot must not undo a newer event.
      if (state.view && state.view.revision > action.view.revision) return state;
      const entered = state.view?.state !== action.view.state;
      // A new serial or a finished activation makes earlier feedback obsolete.
      const fresh = entered && (action.view.state === 'pending' || action.view.state === 'creating' || action.view.state === 'active');
      return {
        ...state, view: action.view, initializationError: null,
        error: fresh ? null : state.error,
        notice: fresh ? null : state.notice,
      };
    }
    case 'start':
      return { ...state, operation: action.operation, busy: action.command, error: null, notice: null };
    case 'finish':
      if (action.operation !== state.operation) return state;
      return { ...state, busy: null, error: action.error ?? null, notice: action.notice ?? null };
    case 'initialization_failed':
      return { ...state, busy: null, initializationError: action.error };
    case 'retry_initialization':
      return { ...state, initializationError: null };
  }
}

export type GateStage = 'loading' | 'activation' | 'onboarding' | 'launcher';

/**
 * What the window shows. Outside the desktop shell there is nothing to activate.
 * An accounts problem leaves the launcher open, because that is where it is explained.
 */
export function gateStage(input: {
  desktop: boolean;
  view: ActivationView | null;
  accountCount: number | null;
  accountsFailed: boolean;
  initializationError: string | null;
}): GateStage {
  if (!input.desktop) return 'launcher';
  if (input.initializationError) return 'activation';
  if (!input.view) return 'loading';
  if (input.view.state === 'no_session' || input.view.state === 'creating' || input.view.state === 'pending') return 'activation';
  if (input.accountsFailed) return 'launcher';
  if (input.accountCount === null) return 'loading';
  return input.accountCount === 0 ? 'onboarding' : 'launcher';
}

export function remainingSeconds(expiresAt: number | null, nowMs: number): number {
  if (expiresAt === null) return 0;
  return Math.max(0, Math.ceil(expiresAt - nowMs / 1000));
}

export function countdownText(seconds: number): string {
  const whole = Math.max(0, Math.floor(seconds));
  return `${Math.floor(whole / 60)}:${String(whole % 60).padStart(2, '0')}`;
}

export function retryAfterText(seconds: number | null): string {
  if (!seconds) return '';
  return ` Espera ${seconds} ${seconds === 1 ? 'segundo' : 'segundos'} antes de pedir otro código.`;
}

export function memberName(user: PanitaUser): string {
  return user.ign ?? user.discord_name;
}

/** Minecraft status is only a hint: it never decides what the member can do. */
export function registrationHint(user: PanitaUser): string | null {
  return user.minecraft_status === null
    ? 'Termina tu registro con /register en el servidor de Discord para indicar cómo juegas.'
    : null;
}

/** The nickname is only a suggestion for an offline account, never a requirement. */
export function offlineNameSuggestion(user: PanitaUser): string {
  return user.ign !== null && /^[A-Za-z0-9_]{3,16}$/.test(user.ign) ? user.ign : '';
}

export function signOutNotice(serverClosed: boolean): string {
  return serverClosed
    ? 'Cerraste la sesión de Panitacraft en este equipo.'
    : 'Cerraste la sesión en este equipo, pero no pudimos avisar al servicio. Tu sesión sigue abierta en el servidor hasta que la cierres con /sessions en Discord.';
}
