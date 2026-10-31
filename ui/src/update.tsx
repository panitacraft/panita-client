import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { CircleArrowUp, ExternalLink, LoaderCircle, RefreshCw, RotateCw } from 'lucide-react';
import { Button } from './components/ui/button';
import { Hint } from './components/hint';
import { messageForError } from './errors';
import { useMotion } from './motion';
import { invokeUpdate } from './update-api';
import { badgeModel, channelLocked, type BadgeIcon, type LauncherUpdateView, type UpdateAction, type UpdateChannel } from './update-state';

const HIGHLIGHT_MS = 3500;

export type LauncherUpdate = {
  view: LauncherUpdateView | null;
  busy: boolean;
  error: string | null;
  /** True for a few seconds after the player tries to play while an update is required. */
  highlighted: boolean;
  highlight: () => void;
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
  const [highlighted, setHighlighted] = useState(false);
  const busyRef = useRef(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);

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

  const highlight = useCallback(() => {
    clearTimeout(timer.current);
    setHighlighted(true);
    timer.current = setTimeout(() => setHighlighted(false), HIGHLIGHT_MS);
  }, []);

  const value: LauncherUpdate = {
    view, busy, error, highlighted, highlight,
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

function BadgeIconView({ icon }: { icon: BadgeIcon }) {
  switch (icon) {
    case 'update': return <CircleArrowUp aria-hidden="true" />;
    case 'retry': return <RotateCw aria-hidden="true" />;
    case 'external': return <ExternalLink aria-hidden="true" />;
    case 'busy': return <LoaderCircle className="spin" aria-hidden="true" />;
  }
}

/**
 * The sidebar version. It keeps its box in every state: plain text normally, a button
 * when an update can be taken, and a red retry icon beside a smaller version after a
 * failed check. Everything else about an update is explained in its tooltip.
 */
export function VersionBadge({ version }: { version: string }) {
  const update = useLauncherUpdate();
  const model = badgeModel(update?.view ?? null, messageForError);
  const label = `Panita Client ${version}`;
  if (!update || model.kind === 'version') {
    return <div className="rail-version-slot"><span className="rail-version" title={label}>v{version}</span></div>;
  }
  if (model.kind === 'retry') {
    return <div className="rail-version-slot has-retry">
      <span className="rail-version" title={label}>v{version}</span>
      <Hint text={model.tooltip} side="right">
        <button type="button" className="rail-retry" aria-label="Reintentar la búsqueda de actualizaciones" disabled={update.busy} onClick={() => void update.perform('retry')}><RotateCw aria-hidden="true" /></button>
      </Hint>
    </div>;
  }
  const action = model.action;
  return <div className="rail-version-slot">
    <Hint text={model.tooltip} side="right">
      <button type="button" className={`rail-update ${model.error ? 'is-error' : ''} ${update.highlighted ? 'is-highlighted' : ''}`}
        aria-label={`${model.label}: ${model.tooltip}`} aria-disabled={!action || update.busy}
        onClick={() => { if (action && !update.busy) void update.perform(action); }}>
        <BadgeIconView icon={model.icon} /><span>{model.label}</span>
      </button>
    </Hint>
  </div>;
}

export function GeneralPanel() {
  const update = useLauncherUpdate();
  const [motion, setMotion] = useMotion();
  const view = update?.view ?? null;
  const locked = channelLocked(view) || update?.busy === true;
  return <section className="settings-section" aria-labelledby="updates-heading">
    <h2 id="updates-heading" className="sr-only">General</h2>
    <label className="inherit-toggle setting-row">
      <span className="setting-text">
        <strong>Efectos de movimiento</strong>
        <small>Animaciones al cambiar de pantalla, cargar imágenes y pasar el cursor. Desactívalas si prefieres una interfaz más estática.</small>
      </span>
      <input role="switch" aria-label="Efectos de movimiento" type="checkbox" checked={motion} onChange={event => setMotion(event.target.checked)} />
    </label>
    <p>Versión instalada: <strong>{view?.currentVersion ? `v${view.currentVersion}` : 'desconocida'}</strong></p>
    <label className="inherit-toggle setting-row">
      <span className="setting-text">
        <strong>Recibir versiones beta</strong>
        <small>{view?.channel === 'beta'
          ? 'Recibes versiones beta antes que el resto y pueden tener errores. Cuando salga una versión estable más nueva, pasarás a ella.'
          : 'Recibes solo versiones estables. Si vuelves desde beta, no se instala una versión anterior: conservas la tuya hasta que salga una más nueva.'}</small>
      </span>
      <input role="switch" aria-label="Recibir versiones beta" type="checkbox" checked={view?.channel === 'beta'} disabled={locked}
        onChange={event => { if (update) void update.setChannel(event.target.checked ? 'beta' : 'stable'); }} />
    </label>
    <Button variant="secondary" disabled={!view || update?.busy} onClick={() => void update?.check()}><RefreshCw aria-hidden="true" />Buscar actualizaciones</Button>
    {update?.error && <p role="alert">{update.error}</p>}
  </section>;
}
