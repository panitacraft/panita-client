import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { RefreshCw } from 'lucide-react';
import { Button } from './components/ui/button';
import { messageForError } from './errors';
import { invokeUpdate } from './update-api';
import { channelLocked, updateNotice, type LauncherUpdateView, type UpdateAction, type UpdateChannel } from './update-state';

export type LauncherUpdate = {
  view: LauncherUpdateView | null;
  busy: boolean;
  error: string | null;
  check: () => Promise<void>;
  setChannel: (channel: UpdateChannel) => Promise<void>;
  perform: (action: UpdateAction) => Promise<void>;
};
const UpdateContext = createContext<LauncherUpdate | null>(null);

/** Absent outside the provider, so consumers must tolerate `null`. */
export function useLauncherUpdate() { return useContext(UpdateContext); }

export function LauncherUpdateProvider({ children }: { children: ReactNode }) {
  const desktop = isTauri();
  const [view, setView] = useState<LauncherUpdateView | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const busyRef = useRef(false);

  useEffect(() => {
    if (!desktop) return;
    let active = true;
    let stop: (() => void) | undefined;
    let heard = false;
    void (async () => {
      const unlisten = await listen<LauncherUpdateView>('launcher-update-changed', event => { heard = true; if (active) setView(event.payload); });
      if (!active) { unlisten(); return; }
      stop = unlisten;
      const initial = await invokeUpdate('launcher_update_status', undefined);
      // An event is newer than this read, so it wins.
      if (active && !heard) setView(initial);
    })().catch(() => {});
    // The backend throttles this recheck, so regaining focus never causes a request storm.
    const onFocus = () => { void invokeUpdate('check_launcher_update', { force: false }).then(next => { if (active) setView(next); }).catch(() => {}); };
    window.addEventListener('focus', onFocus);
    return () => { active = false; stop?.(); window.removeEventListener('focus', onFocus); };
  }, [desktop]);

  const act = useCallback(async (work: () => Promise<LauncherUpdateView | void>) => {
    if (busyRef.current) return;
    busyRef.current = true; setBusy(true); setError(null);
    try { const next = await work(); if (next) setView(next); }
    catch (reason) { setError(messageForError(reason)); }
    finally { busyRef.current = false; setBusy(false); }
  }, []);

  const value: LauncherUpdate = {
    view, busy, error,
    check: () => act(() => invokeUpdate('check_launcher_update', { force: true })),
    setChannel: channel => act(() => invokeUpdate('set_update_channel', { channel })),
    perform: action => {
      switch (action) {
        case 'apply': return act(() => invokeUpdate('apply_launcher_update', undefined));
        case 'cancel': return act(() => invokeUpdate('cancel_launcher_update_apply', undefined));
        case 'retry': return act(() => invokeUpdate('check_launcher_update', { force: true }));
        case 'release_page': return act(() => invokeUpdate('open_launcher_release_page', undefined));
      }
    },
  };
  return <UpdateContext.Provider value={value}>{children}</UpdateContext.Provider>;
}

/** The launcher-wide update notice, shown above the library and the utility pages. */
export function LauncherUpdateNotice() {
  const update = useLauncherUpdate();
  const notice = updateNotice(update?.view ?? null, messageForError);
  if (!update || (!notice && !update.error)) return null;
  const action = notice?.action;
  return <>
    {notice && <div className="catalog-notice" role={notice.tone === 'alert' ? 'alert' : 'status'}>
      <p>{notice.text}</p>
      {action && notice.actionLabel && <Button variant="secondary" disabled={update.busy} onClick={() => void update.perform(action)}>{notice.actionLabel}</Button>}
    </div>}
    {update.error && <p role="alert">{update.error}</p>}
  </>;
}

export function UpdatesPanel() {
  const update = useLauncherUpdate();
  const view = update?.view ?? null;
  const locked = channelLocked(view) || update?.busy === true;
  return <section className="settings-section" aria-labelledby="updates-heading">
    <h2 id="updates-heading" className="sr-only">Actualizaciones</h2>
    <p>Versión instalada: <strong>{view?.currentVersion ? `v${view.currentVersion}` : 'desconocida'}</strong></p>
    <label className="inherit-toggle"><span>Recibir versiones beta</span>
      <input role="switch" aria-label="Recibir versiones beta" type="checkbox" checked={view?.channel === 'beta'} disabled={locked}
        onChange={event => { if (update) void update.setChannel(event.target.checked ? 'beta' : 'stable'); }} />
    </label>
    <p>{view?.channel === 'beta'
      ? 'Recibes versiones beta antes que el resto. Pueden tener errores. Al activarse una versión estable más nueva, pasarás a ella.'
      : 'Recibes solo versiones estables. Al volver desde beta no se instala una versión anterior; seguirás con la tuya hasta que salga una más nueva.'}</p>
    <Button variant="secondary" disabled={!view || update?.busy} onClick={() => void update?.check()}><RefreshCw aria-hidden="true" />Buscar actualizaciones</Button>
    {update?.error && <p role="alert">{update.error}</p>}
  </section>;
}
