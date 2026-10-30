import { createContext, useContext, useEffect, useReducer, useRef, useState, type ReactNode } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { WindowControls } from './components/window-controls';
import { WindowDragRegion } from './components/window-drag-region';
import { TooltipProvider } from './components/ui/tooltip';
import logoWhite from './assets/logo-white.svg';
import { Accounts } from './accounts';
import { invokeAccount } from './account-api';
import type { AccountsView } from './account-state';
import { invokeActivation } from './activation-api';
import {
  activationFeedback, countdownText, gateStage, initialActivationFeedback, isConnectivityError, memberName,
  offlineNameSuggestion, registrationHint, remainingSeconds, retryAfterText, signOutNotice, type ActivationView,
} from './activation-state';
import { messageForError } from './errors';
import { CrashNotifications } from './game-output';
import { PlayControls, usePlaySnapshot } from './play';

type Activation = {
  view: ActivationView | null;
  busy: string | null;
  error: string | null;
  notice: string | null;
  initializationError: string | null;
  start: (identifier: string) => Promise<void>;
  cancel: () => Promise<void>;
  retrySave: () => Promise<void>;
  refresh: () => Promise<void>;
  signOut: () => Promise<void>;
  retryInitialization: () => void;
};
const ActivationContext = createContext<Activation | null>(null);

export function useActivation() { return useContext(ActivationContext); }

export function ActivationProvider({ children }: { children: ReactNode }) {
  const desktop = isTauri();
  const [state, dispatch] = useReducer(activationFeedback, initialActivationFeedback);
  const [attempt, setAttempt] = useState(0);
  const sequence = useRef(0);
  const inFlight = useRef(new Set<string>());

  useEffect(() => {
    if (!desktop) return;
    let active = true;
    let stop: (() => void) | undefined;
    void (async () => {
      const unlisten = await listen<ActivationView>('activation-changed', event => {
        if (active) dispatch({ type: 'snapshot', view: event.payload });
      });
      if (!active) { unlisten(); return; }
      stop = unlisten;
      await invoke<void>('initialize');
      const initial = await invokeActivation('activation_snapshot', undefined);
      if (active) dispatch({ type: 'snapshot', view: initial });
    })().catch(reason => {
      if (active) dispatch({ type: 'initialization_failed', error: String(reason) });
    });
    return () => { active = false; stop?.(); };
  }, [desktop, attempt]);

  async function run<T>(command: string, work: () => Promise<T>, notice?: (result: T) => string) {
    // A second click before the first reply must not repeat the command.
    if (inFlight.current.has(command)) return;
    inFlight.current.add(command);
    const operation = ++sequence.current;
    dispatch({ type: 'start', operation, command });
    let error: string | null = null;
    let message: string | null = null;
    try {
      const result = await work();
      message = notice?.(result) ?? null;
      // The event carries the new state; this read only survives a missed event.
      dispatch({ type: 'snapshot', view: await invokeActivation('activation_snapshot', undefined) });
    } catch (reason) {
      // Cancelling is the member's own doing, not a failure to report.
      error = reason === 'operation_cancelled' ? null : String(reason);
    } finally {
      inFlight.current.delete(command);
      dispatch({ type: 'finish', operation, error, notice: message });
    }
  }

  const value: Activation = {
    view: state.view, busy: state.busy, error: state.error, notice: state.notice, initializationError: state.initializationError,
    start: identifier => run('start_activation', () => invokeActivation('start_activation', { identifier })),
    cancel: () => run('cancel_activation', () => invokeActivation('cancel_activation', undefined)),
    retrySave: () => run('retry_activation_save', () => invokeActivation('retry_activation_save', undefined), () => 'Tu sesión se guardó correctamente.'),
    refresh: () => run('refresh_activation', () => invokeActivation('refresh_activation', undefined)),
    signOut: () => run('sign_out_activation', () => invokeActivation('sign_out_activation', undefined), result => signOutNotice(result.serverClosed)),
    retryInitialization: () => { dispatch({ type: 'retry_initialization' }); setAttempt(previous => previous + 1); },
  };

  // A session that could not be verified gets another check whenever the window regains focus.
  const unverified = state.view?.state === 'active_offline';
  useEffect(() => {
    if (!desktop || !unverified) return;
    const onFocus = () => { void value.refresh(); };
    window.addEventListener('focus', onFocus);
    return () => window.removeEventListener('focus', onFocus);
  }, [desktop, unverified]);

  return <ActivationContext.Provider value={value}>{children}</ActivationContext.Provider>;
}

function useNow(active: boolean) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [active]);
  return now;
}

/** How many Minecraft accounts are saved, to decide whether onboarding is still needed. */
function useAccountCount(enabled: boolean) {
  const [count, setCount] = useState<number | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (!enabled) return;
    let active = true;
    let stop: (() => void) | undefined;
    let latest = -1;
    const accept = (view: AccountsView) => {
      if (!active || view.revision < latest) return;
      latest = view.revision;
      setCount(view.accounts.length);
      setFailed(false);
    };
    void (async () => {
      const unlisten = await listen<AccountsView>('accounts-changed', event => accept(event.payload));
      if (!active) { unlisten(); return; }
      stop = unlisten;
      await invokeAccount('initialize', undefined);
      accept(await invokeAccount('account_snapshot', undefined));
    })().catch(() => { if (active) setFailed(true); });
    return () => { active = false; stop?.(); };
  }, [enabled]);
  return { count, failed };
}

/**
 * A game that is already open is a separate process: ending or never having a Panitacraft
 * session does not close it, so its controls must stay reachable outside the launcher.
 */
function RunningGames() {
  const snapshot = usePlaySnapshot();
  const running = snapshot?.instances.filter(entry => entry.operation === 'play' && (entry.phase === 'running' || entry.phase === 'stopping')) ?? [];
  if (running.length === 0) return null;
  return <section aria-labelledby="running-games-heading">
    <h2 id="running-games-heading">Minecraft sigue en ejecución</h2>
    <p>Activar Panita Client no cierra un juego que ya está abierto. Puedes cerrarlo desde aquí.</p>
    {running.map(entry => <PlayControls key={entry.instanceId} instanceId={entry.instanceId} allowed={false} />)}
  </section>;
}

function Shell({ children }: { children: ReactNode }) {
  return <TooltipProvider><main className="launcher-shell setup-shell">
    <header className="launcher-toolbar">
      <WindowDragRegion><span className="setup-brand"><img src={logoWhite} alt="" aria-hidden="true" draggable={false} /><span className="toolbar-brand">PANITA <span>CLIENT</span></span></span></WindowDragRegion>
      <WindowControls separator={false} />
    </header>
    <div className="setup-content">{children}<RunningGames /><CrashNotifications names={{}} /></div>
  </main></TooltipProvider>;
}

/** The window opens on activation, then onboarding, and only then on the launcher. */
export function ActivationGate({ children }: { children: ReactNode }) {
  const activation = useActivation();
  const desktop = isTauri();
  const signedIn = activation?.view?.state === 'active' || activation?.view?.state === 'active_offline';
  const { count, failed } = useAccountCount(desktop && signedIn);
  const stage = gateStage({
    desktop, view: activation?.view ?? null, accountCount: count, accountsFailed: failed,
    initializationError: activation?.initializationError ?? null,
  });
  if (stage === 'launcher') return <>{children}</>;
  if (stage === 'activation') return <ActivationScreen />;
  if (stage === 'onboarding') return <Onboarding />;
  return <Shell><section><p role="status">Cargando…</p></section></Shell>;
}

function ActivationScreen() {
  const activation = useActivation();
  const [identifier, setIdentifier] = useState('');
  const [copy, setCopy] = useState<'idle' | 'copied' | 'failed'>('idle');
  const view = activation?.view ?? null;
  const pending = view?.state === 'pending';
  const now = useNow(pending);
  if (!activation) return null;

  const serial = view?.serial ?? null;
  const starting = activation.busy === 'start_activation' || view?.state === 'creating';
  const code = view?.error ?? activation.error;
  const shown = code ? `${messageForError(code)}${code === 'activation_rate_limited' ? retryAfterText(view?.retry_after ?? null) : ''}` : null;
  // While waiting, a network error is not final: polling continues until the code expires.
  const waitingOffline = pending && isConnectivityError(code);
  // Without a session there is nothing to play offline: activating is what needs the service.
  const needsConnection = !pending && isConnectivityError(code);

  async function copySerial() {
    if (!serial) return;
    try { await navigator.clipboard.writeText(serial); setCopy('copied'); }
    catch { setCopy('failed'); }
  }

  return <Shell>
    <section className="activation-section" aria-labelledby="activation-title">
      <h1 id="activation-title">Activa Panita Client</h1>
      <p>Necesitas una cuenta de Panitacraft activada para usar el cliente. Es independiente de tu cuenta de Minecraft.</p>
      {activation.notice && <p role="status">{activation.notice}</p>}
      {activation.initializationError && <div role="alert">
        <p>{messageForError(activation.initializationError)}</p>
        <button type="button" onClick={activation.retryInitialization}>Reintentar</button>
      </div>}
      {!activation.initializationError && !view && <p role="status">Cargando…</p>}
      {shown && !waitingOffline && !needsConnection && <p role="alert" className="account-error">{shown}</p>}
      {needsConnection && <p role="alert" className="account-error">No pudimos conectar con Panitacraft. Activar el cliente necesita conexión: revisa tu red e inténtalo de nuevo. Cuando ya estés activado, podrás jugar sin conexión las instancias que tengas instaladas.</p>}
      {waitingOffline && <p role="status">No pudimos conectar con el servicio. Seguimos esperando tu confirmación mientras el código siga vigente.</p>}
      {view && !pending && <form className="activation-form" noValidate onSubmit={event => { event.preventDefault(); void activation.start(identifier); }}>
        <label htmlFor="member-identifier">Tu nombre de Minecraft o de Discord</label>
        <input id="member-identifier" value={identifier} onChange={event => setIdentifier(event.target.value)} maxLength={100} required autoComplete="off" />
        <button type="submit" disabled={starting || identifier.trim().length === 0}>{starting ? 'Pidiendo tu código…' : 'Obtener código'}</button>
      </form>}
      {pending && serial && <div className="serial-box" role="status">
        <p>Tu código de activación:</p>
        <p className="serial"><code aria-label={`Código ${serial.split('').join(' ')}`}>{serial}</code></p>
        <div className="account-buttons">
          <button type="button" onClick={() => void copySerial()}>Copiar código</button>
          {copy === 'copied' && <span role="status">Código copiado.</span>}
          {copy === 'failed' && <span role="alert">No pudimos copiarlo. Selecciónalo y cópialo a mano.</span>}
        </div>
        <p>Escribe <code>/confirm {serial}</code> en el servidor de Discord de Panitacraft, o envía el código al bot por mensaje directo.</p>
        <p><strong>Confirma solo un código que generaste aquí.</strong> Si alguien más te pide confirmar un código, no lo hagas.</p>
        <p>El código vence en {countdownText(remainingSeconds(view?.expires_at ?? null, now))}.</p>
        <button type="button" disabled={activation.busy === 'cancel_activation'} onClick={() => void activation.cancel()}>Cancelar</button>
      </div>}
    </section>
  </Shell>;
}

function Onboarding() {
  const activation = useActivation();
  const user = activation?.view?.user ?? null;
  return <Shell>
    <section aria-labelledby="onboarding-title">
      <h1 id="onboarding-title">Elige cómo jugar</h1>
      <p>Tu cuenta de Panitacraft está activada. Ahora añade la cuenta de Minecraft con la que vas a jugar: inicia sesión con Microsoft si tienes Minecraft Java original, o usa un nombre sin conexión si no lo tienes.</p>
    </section>
    <Accounts suggestedName={user ? offlineNameSuggestion(user) : ''} />
  </Shell>;
}

/** The member's Panitacraft account inside the launcher, with the session controls. */
export function ActivationPanel() {
  const activation = useActivation();
  const [confirming, setConfirming] = useState(false);
  const view = activation?.view;
  if (!activation || !view?.user) return null;
  const user = view.user;
  const hint = registrationHint(user);
  return <section className="activation-panel" aria-labelledby="activation-panel-heading">
    <h2 id="activation-panel-heading">Tu cuenta de Panitacraft</h2>
    {view.state === 'active_offline' && <div className="catalog-notice" role="status">
      <p>No pudimos verificar tu sesión de Panitacraft. Puedes jugar las instancias que ya tienes instaladas; instalar, actualizar o reparar necesita conexión.</p>
      <button type="button" disabled={activation.busy === 'refresh_activation'} onClick={() => void activation.refresh()}>Reintentar</button>
    </div>}
    {!view.persisted && <div role="alert" className="account-error">
      <p>Tu sesión de Panitacraft no se ha guardado. Reintenta guardarla antes de cerrar el cliente.</p>
      <button type="button" onClick={() => void activation.retrySave()}>Reintentar guardado</button>
    </div>}
    {(activation.error ?? (view.error === 'api_key_rejected' ? view.error : null)) && <p role="alert" className="account-error">{messageForError(activation.error ?? view.error)}</p>}
    {activation.notice && <p role="status">{activation.notice}</p>}
    <p><strong>{memberName(user)}</strong> <span>{user.discord_name}</span></p>
    {hint && <p>{hint}</p>}
    {confirming
      ? <span role="group" aria-label="Confirmar cierre de sesión">
          <span>¿Cerrar la sesión de Panitacraft en este equipo? Tus cuentas de Minecraft se conservan.</span>
          <button type="button" disabled={activation.busy === 'sign_out_activation'} onClick={() => { setConfirming(false); void activation.signOut(); }}>Cerrar sesión</button>
          <button type="button" onClick={() => setConfirming(false)}>Conservar</button>
        </span>
      : <button type="button" onClick={() => setConfirming(true)}>Cerrar sesión de Panitacraft</button>}
  </section>;
}
