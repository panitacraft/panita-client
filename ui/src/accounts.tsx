import { useEffect, useReducer, useRef, useState } from 'react';
import { Check, ChevronDown, Copy, Ellipsis, LogIn, LogOut, Plus, RefreshCw, Save, ShieldCheck, UserRound, X } from 'lucide-react';
import { TransientNotice, useTransientNotice } from './components/transient-notice';
import { Button } from './components/ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from './components/ui/dropdown-menu';
import { AccountDialog } from './components/account-dialog';
import { MicrosoftIcon, ProfileArtwork } from './components/profile-artwork';
import minecraft from './assets/brands/minecraft.svg';
import { isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { messageForError } from './errors';
import { invokeAccount, type AccountArguments } from './account-api';
import { accountFeedback, initialAccountFeedback, type AccountStatus, type AccountsView } from './account-state';

const statusText: Record<AccountStatus, string> = {
  offline: 'No premium', waiting_for_browser: 'Esperando al navegador',
  authenticating: 'Verificando sesión…', ready: 'Sesión verificada',
  sign_in_required: 'Sesión pendiente', error: 'No se pudo verificar la sesión',
};

// `suggestedName` only pre-fills the offline name during onboarding; it is never a requirement.
export function Accounts({ suggestedName = '', onSnapshot, onboarding = false }: { onboarding?: boolean; suggestedName?: string; onSnapshot?: (view: AccountsView | null) => void }) {
  const [{ view, error, notice, busy, blocked, cleanupWarning }, dispatch] = useReducer(accountFeedback, initialAccountFeedback);
  const [attempt, setAttempt] = useState(0);
  const [username, setUsername] = useState(suggestedName);
  const [confirming, setConfirming] = useState<string | null>(null);
  const [dialog, setDialog] = useState<'offline' | 'microsoft' | null>(null);
  const toast = useTransientNotice();
  useEffect(() => { if (notice) toast.show(notice); }, [notice, toast.show]);
  const wasPending = useRef(false);
  useEffect(() => {
    if (view?.login_pending) { wasPending.current = true; setDialog('microsoft'); }
    else if (wasPending.current) {
      wasPending.current = false;
      if (!view?.login_error) setDialog(null);
    }
  }, [view?.login_pending, view?.login_error]);
  const loginStarting = useRef(false);
  const inFlight = useRef(new Set<string>());
  const sequence = useRef(0);
  const desktop = isTauri();

  useEffect(() => { onSnapshot?.(view); }, [view, onSnapshot]);

  function accept(next: AccountsView) {
    dispatch({ type: 'snapshot', view: next });
  }

  useEffect(() => {
    if (!desktop) return;
    let active = true;
    let stop: (() => void) | undefined;
    void (async () => {
      const unlisten = await listen<AccountsView>('accounts-changed', event => {
        if (!active) return;
        accept(event.payload);
      });
      if (!active) { unlisten(); return; }
      stop = unlisten;
      await invokeAccount('initialize', undefined);
      const initial = await invokeAccount('account_snapshot', undefined);
      if (active) accept(initial);
    })().catch(reason => {
      if (!active) return;
      dispatch({ type: 'initialization_failed', error: messageForError(reason), blocked: reason === 'accounts_file_invalid' });
    });
    return () => { active = false; stop?.(); sequence.current += 1; };
  }, [desktop, attempt]);

  async function act<C extends keyof AccountArguments>(command: C, args: AccountArguments[C], success?: string) {
    const selectedAccountId = command === 'renew_account' || command === 'validate_account' ? view?.selected_account_id : undefined;
    const key = JSON.stringify([command, args, selectedAccountId]);
    if (inFlight.current.has(key)) return;
    if (command === 'start_microsoft_login') {
      // A second click before the first reply must not report the first login as busy.
      if (loginStarting.current || view?.login_pending) return;
      loginStarting.current = true;
    }
    inFlight.current.add(key);
    const current = ++sequence.current;
    dispatch({ type: 'start', operation: current, command });
    let resultError: string | null = null;
    let resultNotice: string | null = null;
    try {
      const outcome = await invokeAccount(command, args);
      if (command === 'remove_account') {
        if (outcome === 'pending') dispatch({ type: 'cleanup_warning', warning: 'La sesión se cerró en este cliente, pero la eliminación del archivo quedó pendiente. Se reintentará al reiniciar.' });
        else if (outcome === 'not_durable') dispatch({ type: 'cleanup_warning', warning: 'La sesión se cerró en este cliente, pero no pudimos eliminar los datos guardados. La cuenta podría reaparecer al reiniciar.' });
      }
      if (current !== sequence.current) return;
      if (success) resultNotice = success;
      if (command === 'add_offline_account') { setUsername(''); setDialog(null); }
      const next = await invokeAccount('account_snapshot', undefined);
      if (current === sequence.current) accept(next);
      return true;
    } catch (reason) {
      let shownInRow = false;
      if (command === 'renew_account' || command === 'validate_account') {
        // Refresh even on failure: preflight errors do not always update a row.
        try {
          const next = await invokeAccount('account_snapshot', undefined);
          if (current === sequence.current) accept(next);
          shownInRow = next.accounts.some(account => account.metadata.account_id === selectedAccountId && account.error === reason);
        } catch { /* Preserve the original actionable error if refresh also fails. */ }
      }
      if (!shownInRow) resultError ??= messageForError(reason);
      return false;
    } finally {
      inFlight.current.delete(key);
      if (command === 'start_microsoft_login') loginStarting.current = false;
      if (current === sequence.current) dispatch({ type: 'finish', operation: current, error: resultError, notice: resultNotice, selectedAccountId });
    }
  }

  function startLogin(accountId: string | null = null) {
    toast.clear(); setDialog('microsoft');
    void act('start_microsoft_login', { accountId });
  }
  async function cancelDialog() {
    if (loginStarting.current) return;
    if (dialog === 'microsoft' && view?.login_pending) {
      if (!await act('cancel_microsoft_login', undefined)) return;
    }
    setDialog(null); toast.clear();
  }
  async function copyLink() {
    try {
      const url = await invokeAccount('microsoft_login_link', undefined);
      await navigator.clipboard.writeText(url);
      toast.show('Enlace copiado.');
    } catch { toast.show('No pudimos copiar el enlace. Inténtalo de nuevo.', 'error'); }
  }
  const unavailable = !view || view.login_pending || busy === 'start_microsoft_login' || blocked;
  const choices = <>
    <Button disabled={unavailable} onClick={() => startLogin()}><MicrosoftIcon />Añadir cuenta Microsoft</Button>
    <Button disabled={!view || blocked} onClick={() => { setUsername(suggestedName); setDialog('offline'); }}><UserRound aria-hidden="true" />Añadir cuenta No premium</Button>
  </>;
  const feedback = <>
    {error && <p role="alert" className="account-error">{error}</p>}
    {view?.login_error && <p role="alert" className="account-error">{messageForError(view.login_error)}</p>}
  </>;
  return <section className={`accounts-section ${onboarding ? 'account-onboarding' : ''}`} aria-labelledby="accounts-heading">
    {onboarding ? <>
      <h1 id="accounts-heading" className="account-title"><img src={minecraft} alt="" />Acceder a Minecraft</h1>
      <p>Selecciona el tipo de cuenta</p><div className="account-choices">{choices}</div>
    </> : <div className="account-section-heading">
      <h2 id="accounts-heading" className="account-title"><img src={minecraft} alt="" />Perfiles de Minecraft</h2>
      <DropdownMenu><DropdownMenuTrigger asChild><Button disabled={!view || blocked}><Plus aria-hidden="true" />Añadir<ChevronDown aria-hidden="true" /></Button></DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem disabled={unavailable} onSelect={() => startLogin()}><MicrosoftIcon />Añadir cuenta Microsoft</DropdownMenuItem>
          <DropdownMenuItem onSelect={() => { setUsername(''); setDialog('offline'); }}><UserRound />Añadir cuenta No premium</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>}
    {!desktop && <p>La gestión de cuentas está disponible en la aplicación de escritorio.</p>}
    {!dialog && feedback}
    {cleanupWarning && <p role="alert" className="account-error">{cleanupWarning}</p>}
    {!dialog && !confirming && <TransientNotice notice={toast.notice} />}
    {desktop && !view && !error && <p role="status">Cargando cuentas…</p>}
    {desktop && !view && error && !blocked && <Button onClick={() => { dispatch({ type: 'start', operation: ++sequence.current, command: 'initialize' }); setAttempt(previous => previous + 1); }}>Reintentar</Button>}
    {view && !onboarding && <ul className="account-list profile-list">
      {view.accounts.length === 0 && <li>No has añadido ninguna cuenta.</li>}
      {view.accounts.map(account => {
        const id = account.metadata.account_id;
        const selected = id === view.selected_account_id;
        const microsoft = account.metadata.kind === 'microsoft';
        return <li key={id} className="profile-row">
          <ProfileArtwork username={microsoft ? account.metadata.username : 'MHF_Steve'} bust />
          <div className="profile-info"><div className="profile-name"><strong>{account.metadata.username}</strong>{selected && <span className="active-badge"><Check aria-hidden="true" />Activa</span>}</div>
            <span className="profile-type">{microsoft ? <MicrosoftIcon /> : <UserRound aria-hidden="true" />}{microsoft ? 'Microsoft' : 'No premium'}</span>
            {microsoft && account.status !== 'ready' && <p className="profile-status">{statusText[account.status]}</p>}
            {account.error && <p role="alert">{messageForError(account.error)}</p>}
            {!account.persisted && <p role="alert">Esta cuenta no se ha guardado. Reintenta guardarla antes de cerrar el cliente.</p>}
          </div>
          <DropdownMenu><DropdownMenuTrigger asChild><Button variant="ghost" size="icon" aria-label={`Opciones de ${account.metadata.username}`}><Ellipsis aria-hidden="true" /></Button></DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem disabled={selected} onSelect={() => void act('select_account', { accountId: id })}><Check />Seleccionar</DropdownMenuItem>
              {microsoft && <DropdownMenuItem disabled={view.login_pending} onSelect={() => startLogin(id)}><LogIn />Volver a iniciar sesión</DropdownMenuItem>}
              {selected && microsoft && <DropdownMenuItem disabled={view.login_pending || busy === 'renew_account'} onSelect={() => void act('renew_account', undefined, 'Sesión actualizada.')}><RefreshCw />Reintentar sesión</DropdownMenuItem>}
              {selected && microsoft && <DropdownMenuItem disabled={view.login_pending || account.status !== 'ready' || busy === 'validate_account'} onSelect={() => void act('validate_account', undefined, 'Sesión de Minecraft verificada.')}><ShieldCheck />Comprobar sesión</DropdownMenuItem>}
              {!account.persisted && <DropdownMenuItem onSelect={() => void act('retry_account_save', { accountId: id }, 'Cuenta guardada.')}><Save />Reintentar guardado</DropdownMenuItem>}
              <DropdownMenuSeparator /><DropdownMenuItem className="text-destructive" onSelect={() => setConfirming(id)}><LogOut />Quitar cuenta</DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </li>;
      })}
    </ul>}
    {dialog === 'offline' && <AccountDialog title="Nueva cuenta No premium" onCancel={() => { if (busy !== 'add_offline_account') setDialog(null); }}>
      <TransientNotice notice={toast.notice} />
      <form noValidate onSubmit={event => { event.preventDefault(); void act('add_offline_account', { username }); }}>
        <label htmlFor="offline-name">Nombre de usuario</label>
        <input id="offline-name" placeholder="Ej. Steve" value={username} onChange={event => setUsername(event.target.value)} minLength={3} maxLength={16} pattern="[A-Za-z0-9_]{3,16}" required autoComplete="off" autoFocus />
        {feedback}<div className="account-buttons"><Button type="submit" disabled={busy === 'add_offline_account' || !username.trim()}><Check aria-hidden="true" />Confirmar</Button><Button type="button" variant="outline" disabled={busy === 'add_offline_account'} onClick={() => setDialog(null)}><X aria-hidden="true" />Cancelar</Button></div>
      </form>
    </AccountDialog>}
    {dialog === 'microsoft' && <AccountDialog title="Añadir cuenta Microsoft" onCancel={() => void cancelDialog()}>
      <p>¿Problemas con el navegador? <button className="text-action" disabled={!view?.login_pending || busy === 'cancel_microsoft_login'} onClick={() => void copyLink()}><Copy aria-hidden="true" />Copiar enlace</button></p>
      <p>Completa el inicio de sesión en tu navegador para volver a Panita Client.</p>
      {feedback}<TransientNotice notice={toast.notice} />
      <div className="account-buttons"><Button variant="outline" disabled={busy === 'start_microsoft_login' || busy === 'cancel_microsoft_login'} onClick={() => void cancelDialog()}><X aria-hidden="true" />Cancelar</Button></div>
    </AccountDialog>}
    {confirming && <AccountDialog title="Quitar cuenta" onCancel={() => setConfirming(null)}>
      <TransientNotice notice={toast.notice} />
      <p>¿Quitar esta cuenta de este equipo? Se eliminará su sesión guardada.</p>
      <div className="account-buttons"><Button variant="destructive" onClick={() => { const id = confirming; setConfirming(null); void act('remove_account', { accountId: id }); }}><LogOut aria-hidden="true" />Quitar cuenta</Button><Button variant="outline" onClick={() => setConfirming(null)}><X aria-hidden="true" />Cancelar</Button></div>
    </AccountDialog>}
  </section>;
}
