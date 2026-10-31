import { createContext, useContext, useEffect, useReducer, useRef, useState, type ReactNode } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { WindowControls } from './components/window-controls';
import { WindowDragRegion } from './components/window-drag-region';
import { TooltipProvider } from './components/ui/tooltip';
import { ArrowRight, Check, Copy, Info, LogOut, Ellipsis, RefreshCw, X } from 'lucide-react';
import { TransientNotice, useTransientNotice } from './components/transient-notice';
import logoBackground from './assets/logo_background.svg';
import { Button } from './components/ui/button';
import { Hint } from './components/hint';
import { AccountDialog } from './components/account-dialog';
import { ProfileArtwork } from './components/profile-artwork';
import { DropdownMenu, DropdownMenuTrigger, DropdownMenuContent, DropdownMenuItem } from './components/ui/dropdown-menu';
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
      <WindowDragRegion><span className="setup-brand"><img src={logoBackground} alt="" aria-hidden="true" draggable={false} /><span className="toolbar-brand">PANITA <span>CLIENT</span></span></span></WindowDragRegion>
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
  const toast = useTransientNotice();
  useEffect(() => { if (activation?.notice) toast.show(activation.notice); }, [activation?.notice, toast.show]);
  const view = activation?.view ?? null;
  const pending = view?.state === 'pending';
  const now = useNow(pending);
  const [lastSerial, setLastSerial] = useState<string | null>(null);
  useEffect(() => { if (view?.serial) { setLastSerial(view.serial); toast.clear(); } }, [view?.serial, toast.clear]);
  if (!activation) return null;
  const serial = view?.serial ?? lastSerial;
  const starting = activation.busy === 'start_activation' || view?.state === 'creating';
  const code = view?.error ?? activation.error;
  const shown = code ? `${messageForError(code)}${code === 'activation_rate_limited' ? retryAfterText(view?.retry_after ?? null) : ''}` : null;
  // While waiting, a network error is not final: polling continues until the code expires.
  const waitingOffline = pending && isConnectivityError(code);
  // Without a session there is nothing to play offline: activating is what needs the service.
  const needsConnection = !pending && isConnectivityError(code);

  async function copySerial() {
    if (!serial) return;
    try { await navigator.clipboard.writeText(serial); toast.show('Código copiado.'); }
    catch { toast.show('No pudimos copiar el código. Selecciónalo y cópialo a mano.', 'error'); }
  }

  const expired = !pending && !!lastSerial && code === 'activation_expired';
  const help = 'Te damos un código que confirmas desde Discord: envíaselo a Panita Bot por mensaje directo o usa /confirm en el servidor. No necesitas contraseña.';
  return <Shell>
    <section className="activation-section login-card" aria-labelledby="activation-title">
      <h1 id="activation-title" className="account-title"><img src={logoWhite} alt="" />Acceder a Panita Client</h1>
      <p className="login-subtitle">{pending || expired ? 'Introduce este código' : 'Vincula tu cuenta de Panitacraft'} <Hint text={help}><button className="info-button" aria-label="Cómo vincular tu cuenta"><Info aria-hidden="true" /></button></Hint></p>
      <TransientNotice notice={toast.notice} />
      {activation.initializationError && <div role="alert"><p>{messageForError(activation.initializationError)}</p><Button onClick={activation.retryInitialization}><RefreshCw aria-hidden="true" />Reintentar</Button></div>}
      {!activation.initializationError && !view && <p role="status">Cargando…</p>}
      {shown && !expired && !waitingOffline && !needsConnection && <p role="alert" className="account-error">{shown}</p>}
      {needsConnection && <p role="alert">No pudimos conectar con Panitacraft. Revisa tu conexión e inténtalo de nuevo.</p>}
      {waitingOffline && <p role="status">No pudimos conectar con el servicio. Seguimos esperando tu confirmación mientras el código siga vigente.</p>}
      {view && !pending && !expired && <form className="activation-form" noValidate onSubmit={event => { event.preventDefault(); void activation.start(identifier); }}>
        <label htmlFor="member-identifier">IGN o usuario de Discord</label>
        <div className="identifier-input"><ProfileArtwork username={identifier.trim()} /><input id="member-identifier" placeholder="Ej. Aerosmith_" value={identifier} onChange={event => setIdentifier(event.target.value)} maxLength={100} required autoComplete="off" /></div>
        <Button type="submit" disabled={starting || identifier.trim().length === 0}>{starting ? 'Pidiendo tu código…' : 'Continuar'}<ArrowRight aria-hidden="true" /></Button>
      </form>}
      {(pending || expired) && serial && <div className={`serial-box ${expired ? 'is-expired' : ''}`}>
        <p className="member-greeting">Hola, <ProfileArtwork username={identifier.trim()} /><strong>{identifier}</strong></p>
        <div className="serial-groups" role="status" aria-label={`Código ${serial}`}><code>{serial}</code></div>
        <Button variant="outline" disabled={expired} onClick={() => void copySerial()}><Copy aria-hidden="true" />Copiar código</Button>
        {pending && <>
          <p className="serial-instructions">Envía el código a <strong>Panita Bot</strong> por mensaje directo.<br />En el servidor, usa <code>/confirm {serial}</code></p>
          <p className="serial-expiry">Vence en {countdownText(remainingSeconds(view?.expires_at ?? null, now))}.</p>
        </>}
        <p>¿No eres tú? <button className="text-action" disabled={!!activation.busy} onClick={() => { void activation.cancel().then(() => setLastSerial(null)); }}>Cambiar</button></p>
        {expired && <><p role="alert">El código venció. Genera otro y vuelve a intentarlo.</p><Button disabled={starting} onClick={() => void activation.start(identifier)}>Generar otro código<RefreshCw aria-hidden="true" /></Button></>}
      </div>}
    </section>
  </Shell>;
}

function Onboarding() {
  const activation = useActivation();
  const user = activation?.view?.user ?? null;
  return <Shell><Accounts onboarding suggestedName={user ? offlineNameSuggestion(user) : ''} /></Shell>;
}

/** The member's Discord avatar, falling back to the Panita logo when absent or failing to load. */
function PanitaAvatar({ url }: { url: string | null }) {
  const [failed, setFailed] = useState<string | null>(null);
  if (!url || url === failed) return <img className="panita-profile-logo" src={logoWhite} alt="" />;
  return <img className="panita-profile-avatar" src={url} alt="" aria-hidden="true" referrerPolicy="no-referrer" decoding="async" onError={() => setFailed(url)} />;
}

/** The member's Panitacraft account inside the launcher, with the session controls. */
export function ActivationPanel() {
  const activation = useActivation();
  const toast = useTransientNotice();
  useEffect(() => { if (activation?.notice) toast.show(activation.notice); }, [activation?.notice, toast.show]);
  const [confirming, setConfirming] = useState(false);
  const view = activation?.view;
  if (!activation || !view?.user) return null;
  const user = view.user;
  const hint = registrationHint(user);
  return <section className="activation-panel" aria-labelledby="activation-panel-heading">
    <h2 id="activation-panel-heading" className="account-title"><img src={logoWhite} alt="" />Cuenta de Panitacraft</h2>
    {view.state === 'active_offline' && <div className="catalog-notice" role="status">
      <p>No pudimos verificar tu sesión de Panitacraft. Puedes jugar las instancias que ya tienes instaladas; instalar, actualizar o reparar necesita conexión.</p>
      <Button disabled={activation.busy === 'refresh_activation'} onClick={() => void activation.refresh()}><RefreshCw aria-hidden="true" />Reintentar</Button>
    </div>}
    {!view.persisted && <div role="alert" className="account-error">
      <p>Tu sesión de Panitacraft no se ha guardado. Reintenta guardarla antes de cerrar el cliente.</p>
      <Button onClick={() => void activation.retrySave()}><RefreshCw aria-hidden="true" />Reintentar guardado</Button>
    </div>}
    {(activation.error ?? (view.error === 'api_key_rejected' ? view.error : null)) && <p role="alert" className="account-error">{messageForError(activation.error ?? view.error)}</p>}
    <TransientNotice notice={toast.notice} />
    <div className="profile-row panita-profile">
      <PanitaAvatar url={user.avatar_url} />
      <div className="profile-info"><div className="profile-name"><strong>{memberName(user)}</strong><span className="active-badge"><Check aria-hidden="true" />Activa</span></div><span className="profile-type">{user.discord_name}</span>{hint && <p>{hint}</p>}</div>
      <DropdownMenu><DropdownMenuTrigger asChild><Button variant="ghost" size="icon" aria-label="Opciones de Panitacraft"><Ellipsis aria-hidden="true" /></Button></DropdownMenuTrigger><DropdownMenuContent align="end"><DropdownMenuItem onSelect={() => setConfirming(true)}><LogOut />Cerrar sesión</DropdownMenuItem></DropdownMenuContent></DropdownMenu>
    </div>
    {confirming && <AccountDialog title="Cerrar sesión de Panitacraft" onCancel={() => setConfirming(false)}>
      <p>¿Cerrar la sesión de Panitacraft en este equipo? Tus perfiles de Minecraft se conservan.</p>
      <div className="account-buttons"><Button variant="destructive" disabled={activation.busy === 'sign_out_activation'} onClick={() => { setConfirming(false); void activation.signOut(); }}><LogOut aria-hidden="true" />Cerrar sesión</Button><Button variant="outline" onClick={() => setConfirming(false)}><X aria-hidden="true" />Cancelar</Button></div>
    </AccountDialog>}
  </section>;
}
