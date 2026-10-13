import { useEffect, useRef, useState } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { messageForError } from './errors';

type Status = 'offline' | 'waiting_for_browser' | 'authenticating' | 'ready' | 'sign_in_required' | 'error';
type AccountView = {
  metadata: { account_id: string; kind: 'microsoft' | 'offline'; username: string; profile_id: string; oauth_client_id: string | null };
  status: Status; error: string | null; persisted: boolean;
};
type AccountsView = { revision: number; accounts: AccountView[]; selected_account_id: string | null; login_pending: boolean; login_error: string | null };
const statusText: Record<Status, string> = {
  offline: 'Cuenta sin conexión', waiting_for_browser: 'Esperando al navegador',
  authenticating: 'Verificando sesión…', ready: 'Sesión disponible',
  sign_in_required: 'Sesión pendiente', error: 'No se pudo verificar la sesión',
};

export function Accounts() {
  const [view, setView] = useState<AccountsView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [username, setUsername] = useState('');
  const [busy, setBusy] = useState<string | null>(null);
  const sequence = useRef(0);
  const desktop = isTauri();

  function accept(next: AccountsView) {
    setView(previous => previous && previous.revision > next.revision ? previous : next);
  }

  useEffect(() => {
    if (!desktop) return;
    let active = true;
    let stop: (() => void) | undefined;
    void (async () => {
      const unlisten = await listen<AccountsView>('accounts-changed', event => { if (active) accept(event.payload); });
      if (!active) { unlisten(); return; }
      stop = unlisten;
      await invoke('initialize');
      const initial = await invoke<AccountsView>('account_snapshot');
      if (active) accept(initial);
    })().catch(reason => { if (active) setError(messageForError(reason)); });
    return () => { active = false; stop?.(); sequence.current += 1; };
  }, [desktop]);

  async function act(command: string, args?: Record<string, unknown>, success?: string) {
    const current = ++sequence.current;
    setBusy(command); setError(null); setNotice(null);
    try {
      const outcome = await invoke<unknown>(command, args);
      if (current !== sequence.current) return;
      if (outcome === 'pending') setError('La sesión se cerró en este cliente, pero la eliminación del archivo quedó pendiente. Se reintentará al reiniciar.');
      else if (outcome === 'not_durable') setError('La sesión se cerró en este cliente, pero no pudimos eliminar los datos guardados. La cuenta podría reaparecer al reiniciar.');
      else if (success) setNotice(success);
      const next = await invoke<AccountsView>('account_snapshot');
      if (current === sequence.current) accept(next);
    } catch (reason) {
      if (current === sequence.current) setError(messageForError(reason));
    } finally {
      if (current === sequence.current) setBusy(null);
    }
  }

  return <section className="accounts-section" aria-labelledby="accounts-heading">
    <h2 id="accounts-heading">Tus cuentas</h2>
    {!desktop && <p>La gestión de cuentas está disponible en la aplicación de escritorio.</p>}
    {error && <p role="alert" className="account-error">{error}</p>}
    {notice && <p role="status">{notice}</p>}
    {view?.login_error && <p role="alert" className="account-error">{messageForError(view.login_error)}</p>}
    {desktop && !view && !error && <p role="status">Cargando cuentas…</p>}
    {desktop && !view && error && <button type="button" onClick={() => void act('initialize')}>Reintentar</button>}
    {view && <>
      {view.login_pending && <div className="login-progress" role="status">
        <p>Completa el inicio de sesión en tu navegador y permite abrir Panita Client. Cuando vuelvas aquí, puedes cerrar la pestaña del navegador. Puedes cancelar o seleccionar otra cuenta.</p>
        <button type="button" onClick={() => void act('cancel_microsoft_login')}>Cancelar inicio de sesión</button>
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
            {!account.persisted && <p>La sesión actual no se ha guardado. Reintenta guardarla antes de cerrar el cliente.</p>}
            <div className="account-buttons">
              <button type="button" aria-pressed={selected} disabled={selected} onClick={() => void act('select_account', { accountId: id })}>Seleccionar</button>
              {account.metadata.kind === 'microsoft' && <button type="button" disabled={view.login_pending} onClick={() => void act('start_microsoft_login', { accountId: id })}>Volver a iniciar sesión</button>}
              {selected && account.metadata.kind === 'microsoft' && <button type="button" disabled={view.login_pending || busy === 'renew_account'} onClick={() => void act('renew_account')}>Reintentar sesión</button>}
              {!account.persisted && <button type="button" onClick={() => void act('retry_account_save', { accountId: id }, 'La cuenta se guardó correctamente.')}>Reintentar guardado</button>}
              {selected && account.metadata.kind === 'microsoft' && <button type="button" disabled={view.login_pending || account.status !== 'ready' || busy === 'validate_account'} onClick={() => void act('validate_account', undefined, 'Minecraft autorizó la sesión. Esta comprobación también se realizará al jugar.')}>Comprobar sesión</button>}
              <button type="button" onClick={() => void act('remove_account', { accountId: id })}>Cerrar sesión y quitar cuenta</button>
            </div>
          </li>;
        })}
      </ul>
    </>}
  </section>;
}
