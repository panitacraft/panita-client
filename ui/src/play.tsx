import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { messageForError } from './errors';
import { acceptPlaySnapshot, playActive, type PlayPhase, type PlaySnapshot } from './play-state';

type PlayContextValue = {
  snapshot: PlaySnapshot | null;
  error: string | null;
  act: (command: string, args: Record<string, unknown>) => Promise<void>;
};
const PlayContext = createContext<PlayContextValue | null>(null);
const phaseText: Record<PlayPhase, string> = {
  preparing: 'Comprobando la instancia y sus archivos…',
  warning: 'Necesitamos tu confirmación para continuar.',
  starting: 'Verificando la cuenta e iniciando Minecraft…',
  running: 'Minecraft está en ejecución.',
  stopping: 'Esperando a que Minecraft se cierre…',
  cancelling: 'Cancelando la preparación…',
  exited: 'Minecraft se cerró.',
  failed: 'No pudimos iniciar Minecraft.',
  cancelled: 'Inicio cancelado.',
};

export function PlayProvider({ children }: { children: ReactNode }) {
  const [snapshot, setSnapshot] = useState<PlaySnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!isTauri()) return;
    let active = true;
    let stop: (() => void) | undefined;
    void (async () => {
      const unlisten = await listen<PlaySnapshot>('play-changed', event => {
        if (active) setSnapshot(previous => acceptPlaySnapshot(previous, event.payload));
      });
      if (!active) { unlisten(); return; }
      stop = unlisten;
      await invoke<void>('initialize');
      const initial = await invoke<PlaySnapshot>('play_snapshot');
      if (active) setSnapshot(previous => acceptPlaySnapshot(previous, initial));
    })().catch(cause => { if (active) setError(messageForError(cause)); });
    return () => { active = false; stop?.(); };
  }, []);

  async function act(command: string, args: Record<string, unknown>) {
    setError(null);
    try {
      await invoke<void>(command, args);
      const next = await invoke<PlaySnapshot>('play_snapshot');
      setSnapshot(previous => acceptPlaySnapshot(previous, next));
    } catch (cause) { setError(messageForError(cause)); }
  }
  return <PlayContext.Provider value={{ snapshot, error, act }}>{children}</PlayContext.Provider>;
}

export function PlayNotice() {
  const play = useContext(PlayContext);
  return <>
    {play?.error && <p role="alert">{play.error}</p>}
    {play?.snapshot?.closeBlocked && <p className="catalog-notice" role="alert">No pudimos cerrar esta ventana. Si hay un inicio pendiente, cancélalo y vuelve a intentarlo. Puedes minimizar esta ventana.</p>}
  </>;
}

export function PlayControls({ instanceId, allowed }: { instanceId: string; allowed: boolean }) {
  const play = useContext(PlayContext);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const [forceQuestion, setForceQuestion] = useState<string | null>(null);
  const entry = play?.snapshot?.instances.find(entry => entry.instanceId === instanceId);
  const active = entry && playActive(entry.phase);
  const running = entry && (entry.phase === 'running' || entry.phase === 'stopping');

  async function act(command: string, args: Record<string, unknown>) {
    if (!play || busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    try { await play.act(command, { instanceId, ...args }); }
    finally { busyRef.current = false; setBusy(false); }
  }

  return <div className="play-controls">
    {entry && <p role="status">{entry.phase === 'exited' && entry.exitSuccess === false
      ? `Minecraft se cerró de forma inesperada.${entry.exitCode !== null ? ` Código: ${entry.exitCode}.` : ''}`
      : phaseText[entry.phase]}</p>}
    {entry?.error && entry.error !== 'operation_cancelled' && <p className="account-error" role="alert">{messageForError(entry.error)}</p>}
    {entry?.phase === 'warning' && <div className="catalog-notice" role="alert">
      <p>No pudimos comprobar si esta instancia sigue habilitada. Usaremos la instalación local verificada. Si usas una cuenta Microsoft, necesitas conexión para verificarla.</p>
      <div className="account-buttons">
        <button type="button" disabled={busy || !entry.confirmationId} onClick={() => void act('confirm_play', { requestId: entry.requestId, confirmationId: entry.confirmationId })}>Jugar de todos modos</button>
        <button type="button" disabled={busy} onClick={() => void act('cancel_play', { requestId: entry.requestId })}>Cancelar</button>
      </div>
    </div>}
    {active && !running && entry?.phase !== 'warning' && <button type="button" disabled={busy || entry?.phase === 'cancelling'} onClick={() => void act('cancel_play', { requestId: entry?.requestId })}>Cancelar inicio</button>}
    {running && entry && <>
      <p>Cierra Minecraft desde su menú para guardar la partida.</p>
      <p>Si cierras esta ventana, Panita seguirá en segundo plano hasta que cierres los juegos. Abre Panita de nuevo para volver a esta pantalla.</p>
      <div className="account-buttons">
        {play?.snapshot?.gracefulStopSupported && entry.phase !== 'stopping' && <button type="button" disabled={busy} onClick={() => void act('stop_game', { requestId: entry.requestId, force: false })}>Solicitar cierre</button>}
        <button type="button" disabled={busy} onClick={() => setForceQuestion(entry.requestId)}>Cerrar forzosamente…</button>
      </div>
      {forceQuestion === entry.requestId && <div className="catalog-notice" role="alert">
        <p>Al cerrar forzosamente puedes perder cambios sin guardar. Es preferible salir desde Minecraft.</p>
        <div className="account-buttons">
          <button type="button" disabled={busy} onClick={() => { setForceQuestion(null); void act('stop_game', { requestId: entry.requestId, force: true }); }}>Confirmar cierre forzoso</button>
          <button type="button" disabled={busy} onClick={() => setForceQuestion(null)}>Volver</button>
        </div>
      </div>}
    </>}
    {!active && <button type="button" disabled={busy || !allowed || !isTauri() || !play?.snapshot} onClick={() => void act('begin_play', { requestId: crypto.randomUUID() })}>{allowed ? 'Jugar' : 'No disponible para jugar'}</button>}
  </div>;
}
