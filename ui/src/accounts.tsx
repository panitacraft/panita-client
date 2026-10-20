import { useEffect, useReducer, useRef, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { messageForError } from './errors';
import { invokeAccount, type AccountArguments } from './account-api';
import { accountFeedback, initialAccountFeedback, type AccountStatus, type AccountsView } from './account-state';

const statusText: Record<AccountStatus, string> = {
  offline: 'Cuenta sin conexión', waiting_for_browser: 'Esperando al navegador',
  authenticating: 'Verificando sesión…', ready: 'Sesión disponible',
  sign_in_required: 'Sesión pendiente', error: 'No se pudo verificar la sesión',
};

export function Accounts() {
  const [{ view, error, notice, busy, blocked, cleanupWarning }, dispatch] = useReducer(accountFeedback, initialAccountFeedback);
  const [attempt, setAttempt] = useState(0);
  const [username, setUsername] = useState('');
  const [confirming, setConfirming] = useState<string | null>(null);
  const loginStarting = useRef(false);
  const inFlight = useRef(new Set<string>());
  const sequence = useRef(0);
  const desktop = isTauri();

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
      if (command === 'add_offline_account') setUsername('');
      const next = await invokeAccount('account_snapshot', undefined);
      if (current === sequence.current) accept(next);
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
    } finally {
      inFlight.current.delete(key);
      if (command === 'start_microsoft_login') loginStarting.current = false;
      if (current === sequence.current) dispatch({ type: 'finish', operation: current, error: resultError, notice: resultNotice, selectedAccountId });
    }
  }

  return <section className="accounts-section" aria-labelledby="accounts-heading">
    <h2 id="accounts-heading">Tus cuentas</h2>
    {!desktop && <p>La gestión de cuentas está disponible en la aplicación de escritorio.</p>}
    {error && <p role="alert" className="account-error">{error}</p>}
    {cleanupWarning && <p role="alert" className="account-error">{cleanupWarning}</p>}
    {notice && <p role="status">{notice}</p>}
    {view?.login_error && <p role="alert" className="account-error">{messageForError(view.login_error)}</p>}
    {desktop && !view && !error && <p role="status">Cargando cuentas…</p>}
    {desktop && !view && error && !blocked && <button type="button" onClick={() => {
      dispatch({ type: 'start', operation: ++sequence.current, command: 'initialize' });
      setAttempt(previous => previous + 1);
    }}>Reintentar</button>}
    {view && <>
      {view.login_pending && <div className="login-progress" role="status">
        <p>Completa el inicio de sesión en tu navegador y permite abrir Panita Client. Cuando vuelvas aquí, puedes cerrar la pestaña del navegador. Puedes cancelar o seleccionar otra cuenta.</p>
        <button type="button" onClick={() => void act('cancel_microsoft_login', undefined)}>Cancelar inicio de sesión</button>
      </div>}
      <div className="account-actions">
        <button type="button" disabled={view.login_pending || busy === 'start_microsoft_login'} onClick={() => void act('start_microsoft_login', { accountId: null })}>Iniciar sesión con Microsoft</button>
        <form noValidate onSubmit={event => { event.preventDefault(); void act('add_offline_account', { username }); }}>
          <label htmlFor="offline-name">Nombre para jugar sin conexión</label>
          <input id="offline-name" value={username} onChange={event => setUsername(event.target.value)} minLength={3} maxLength={16} pattern="[A-Za-z0-9_]{3,16}" required autoComplete="off" />
          <button type="submit" disabled={busy === 'add_offline_account'}>Añadir cuenta sin conexión</button>
        </form>
      </div>
      {view.accounts.length === 0 && <p>No has añadido ninguna cuenta.</p>}
      <ul className="account-list">
        {view.accounts.map(account => {
          const id = account.metadata.account_id;
          const selected = id === view.selected_account_id;
          return <li key={id} className={selected ? 'selected-account' : undefined}>
            <div><strong>{account.metadata.username}</strong> <span>{account.metadata.kind === 'microsoft' ? 'Microsoft' : 'Sin conexión'}</span></div>
            <p>{selected ? 'Cuenta seleccionada · ' : ''}{statusText[account.status]}</p>
            {account.error && <p role="alert" className="account-error">{messageForError(account.error)}</p>}
            {!account.persisted && <p>Esta cuenta no se ha guardado. Reintenta guardarla antes de cerrar el cliente.</p>}
            <div className="account-buttons">
              <button type="button" aria-pressed={selected} disabled={selected} onClick={() => void act('select_account', { accountId: id })}>Seleccionar</button>
              {account.metadata.kind === 'microsoft' && <button type="button" disabled={view.login_pending} onClick={() => void act('start_microsoft_login', { accountId: id })}>Volver a iniciar sesión</button>}
              {selected && account.metadata.kind === 'microsoft' && <button type="button" disabled={view.login_pending || busy === 'renew_account'} onClick={() => void act('renew_account', undefined)}>Reintentar sesión</button>}
              {!account.persisted && <button type="button" onClick={() => void act('retry_account_save', { accountId: id }, 'La cuenta se guardó correctamente.')}>Reintentar guardado</button>}
              {selected && account.metadata.kind === 'microsoft' && <button type="button" disabled={view.login_pending || account.status !== 'ready' || busy === 'validate_account'} onClick={() => void act('validate_account', undefined, 'Minecraft autorizó la sesión. Esta comprobación también se realizará al jugar.')}>Comprobar sesión</button>}
              {confirming === id
                ? <span role="group" aria-label="Confirmar quitar cuenta">
                    <span>¿Quitar esta cuenta de este equipo?</span>
                    <button type="button" onClick={() => { setConfirming(null); void act('remove_account', { accountId: id }); }}>Quitar cuenta</button>
                    <button type="button" onClick={() => setConfirming(null)}>Conservar</button>
                  </span>
                : <button type="button" onClick={() => setConfirming(id)}>Cerrar sesión y quitar cuenta</button>}
            </div>
          </li>;
        })}
      </ul>
    </>}
  </section>;
}
