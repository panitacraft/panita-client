import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { messageForError } from './errors';
import { acceptPlaySnapshot, playActive, type PlayPhase, type PlaySnapshot } from './play-state';
import type { UpdateTarget } from './library-state';
import { invokePlay, type PlayArguments, type PlayCommand } from './play-api';
import { nativeRetentionReason, type NativeCleanupReport } from './native-cleanup-state';

type PlayContextValue = {
  snapshot: PlaySnapshot | null;
  error: string | null;
  act: <C extends PlayCommand>(command: C, args: PlayArguments[C]) => Promise<void>;
};
const PlayContext = createContext<PlayContextValue | null>(null);
const phaseText: Record<PlayPhase, string> = {
  preparing: 'Comprobando la instancia y sus archivos…',
  installing: 'Preparando Minecraft y Java. Descargando y comprobando archivos…',
  installed: 'Instalación completa. Ya puedes jugar.',
  uninstalled: 'Desinstalación completa. Conservamos tus mundos, ajustes y archivos personales.',
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

  async function act<C extends PlayCommand>(command: C, args: PlayArguments[C]) {
    setError(null);
    try {
      await invokePlay(command, args);
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
    {play?.snapshot?.closeBlocked && <p className="catalog-notice" role="alert">No pudimos cerrar esta ventana. Cancela la instalación o el inicio pendiente y vuelve a intentarlo. Puedes minimizar esta ventana.</p>}
  </>;
}

export function usePlaySnapshot() {
  return useContext(PlayContext)?.snapshot ?? null;
}

export function PlayControls({ instanceId, allowed, installAllowed = false, updateTarget = null, repairAllowed = false, uninstallAllowed = false, cleanupAllowed = false }: { instanceId: string; allowed: boolean; installAllowed?: boolean; updateTarget?: UpdateTarget | null; repairAllowed?: boolean; uninstallAllowed?: boolean; cleanupAllowed?: boolean }) {
  const play = useContext(PlayContext);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const [forceQuestion, setForceQuestion] = useState<string | null>(null);
  const [updateQuestion, setUpdateQuestion] = useState<(UpdateTarget & { backupWorlds: boolean }) | null>(null);
  const [uninstallQuestion, setUninstallQuestion] = useState(false);
  const [logError, setLogError] = useState<string | null>(null);
  const [cleanupText, setCleanupText] = useState<string | null>(null);
  const [cleanupError, setCleanupError] = useState<string | null>(null);
  const [cleanupDetails, setCleanupDetails] = useState<NativeCleanupReport['details']>([]);
  const entry = play?.snapshot?.instances.find(entry => entry.instanceId === instanceId);
  const active = entry && playActive(entry.phase);
  const running = entry && (entry.phase === 'running' || entry.phase === 'stopping');

  async function act<C extends PlayCommand>(command: C, args: Omit<PlayArguments[C], 'instanceId'>) {
    if (!play || busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    try { await play.act(command, { instanceId, ...args } as PlayArguments[C]); }
    finally { busyRef.current = false; setBusy(false); }
  }

  async function cleanNatives() {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setCleanupError(null);
    setCleanupDetails([]);
    setCleanupText('Comprobando los archivos nativos antiguos…');
    try {
      const report = await invoke<NativeCleanupReport>('clean_native_cache', { instanceId });
      setCleanupText(`Se eliminaron ${report.cleanedGenerations} generaciones antiguas. Se conservaron ${report.retainedGenerations + report.untrackedGenerations} antiguas por seguridad. La generación activa se conserva.`);
      setCleanupDetails(report.details);
    } catch (cause) {
      setCleanupText(null);
      setCleanupError(messageForError(cause));
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }

  return <div className="play-controls">
    {entry?.operation === 'play' && (running || entry.phase === 'exited') && <button type="button" disabled={busy || !isTauri()} onClick={() => {
      setLogError(null);
      void invoke('open_game_log', { instanceId, requestId: entry.requestId }).catch(cause => setLogError(messageForError(cause)));
    }}>Ver registro de Minecraft</button>}
    {logError && <p role="alert">{logError}</p>}
    {entry?.consoleError && <p role="alert">Minecraft se inició, pero no pudimos abrir su consola automáticamente. Puedes volver a intentarlo con «Ver registro de Minecraft». {messageForError(entry.consoleError)}</p>}
    {entry && <p role="status">{entry.phase === 'exited' && entry.exitSuccess === false
      ? `Minecraft se cerró de forma inesperada.${entry.exitCode !== null ? ` Código: ${entry.exitCode}.` : ''}`
      : entry.operation === 'install' && entry.phase === 'failed' ? 'No pudimos completar la instalación.'
      : entry.operation === 'install' && entry.phase === 'cancelled' ? 'Instalación cancelada. Puedes reintentarla.'
      : entry.operation === 'update' && entry.phase === 'installing' ? 'Preparando la actualización. La versión instalada se conserva hasta completar la preparación.'
      : entry.operation === 'update' && entry.phase === 'installed' ? 'Actualización completa. Ya puedes jugar.'
      : entry.operation === 'update' && entry.phase === 'failed' ? 'No pudimos completar la actualización.'
      : entry.operation === 'update' && entry.phase === 'cancelled' ? 'Actualización cancelada.'
      : entry.operation === 'repair' && entry.phase === 'installing' ? 'Reparando los archivos de la versión registrada…'
      : entry.operation === 'repair' && entry.phase === 'installed' ? 'Reparación completa. Ya puedes jugar.'
      : entry.operation === 'repair' && entry.phase === 'failed' ? 'No pudimos completar la reparación.'
      : entry.operation === 'repair' && entry.phase === 'cancelled' ? 'Reparación cancelada.'
      : entry.operation === 'uninstall' && entry.phase === 'installing' ? 'Desinstalando y conservando tus datos…'
      : entry.operation === 'uninstall' && entry.phase === 'failed' ? 'No pudimos completar la desinstalación. Conservamos los archivos restantes para recuperar la operación.'
      : entry.operation === 'uninstall' && entry.phase === 'cancelled' ? 'Desinstalación cancelada.'
      : phaseText[entry.phase]}</p>}
    {entry && entry.operation !== 'play' && entry.operation !== 'uninstall' && active && <div>
      <progress aria-label={entry.operation === 'update' ? 'Actualización de Minecraft y Java' : entry.operation === 'repair' ? 'Reparación de Minecraft y Java' : 'Instalación de Minecraft y Java'} />
      <p>Archivos recibidos: {entry.receivedFiles}. Archivos reutilizados: {entry.reusedFiles}.</p>
      <p>La instalación termina cuando se comprueban todos los archivos y Java.</p>
      {entry.operation === 'update' && entry.backupRequested && <p>La copia de mundos debe completarse antes de cambiar la versión.</p>}
    </div>}
    {entry && (entry.operation === 'repair' || entry.operation === 'uninstall') && !active && entry.retainedFiles > 0 && <p role="status">Conservamos {entry.retainedFiles} archivos nativos modificados o que no pudimos comprobar.</p>}
    {entry?.operation === 'update' && entry.phase === 'installed' && <p>{entry.backupOperation
      ? `Copia de mundos guardada en la carpeta de esta instancia: world-backups/${entry.backupOperation}/worlds.zip.`
      : entry.backupRequested ? 'No había mundos locales para copiar.' : 'Actualizaste sin crear una copia de mundos.'}</p>}
    {entry?.error && entry.error !== 'operation_cancelled' && <p className="account-error" role="alert">{messageForError(entry.error)}</p>}
    {entry?.phase === 'warning' && <div className="catalog-notice" role="alert">
      <p>No pudimos comprobar si esta instancia sigue habilitada. Usaremos la instalación local verificada. Si usas una cuenta Microsoft, necesitas conexión para verificarla.</p>
      <div className="account-buttons">
        <button type="button" disabled={busy || !entry.confirmationId} onClick={() => { if (entry.confirmationId) void act('confirm_play', { requestId: entry.requestId, confirmationId: entry.confirmationId }); }}>Jugar de todos modos</button>
        <button type="button" disabled={busy} onClick={() => void act('cancel_play', { requestId: entry.requestId })}>Cancelar</button>
      </div>
    </div>}
    {active && entry && !running && entry.phase !== 'warning' && <button type="button" disabled={busy || entry.phase === 'cancelling'} onClick={() => void act('cancel_play', { requestId: entry.requestId })}>{entry.operation === 'update' ? 'Cancelar actualización' : entry.operation === 'install' ? 'Cancelar instalación' : entry.operation === 'repair' ? 'Cancelar reparación' : entry.operation === 'uninstall' ? 'Cancelar desinstalación' : 'Cancelar inicio'}</button>}
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
    {!active && installAllowed && entry?.phase !== 'installed' && <button type="button" disabled={busy || !isTauri() || !play?.snapshot} onClick={() => void act('begin_install', { requestId: crypto.randomUUID() })}>{entry?.operation === 'install' ? 'Reintentar instalación' : 'Instalar'}</button>}
    {!active && !installAllowed && <button type="button" disabled={busy || !allowed || entry?.phase === 'uninstalled' || !isTauri() || !play?.snapshot} onClick={() => void act('begin_play', { requestId: crypto.randomUUID() })}>{allowed && entry?.phase !== 'uninstalled' ? 'Jugar' : 'No disponible para jugar'}</button>}
    {!active && updateTarget && <button type="button" disabled={busy || !isTauri() || !play?.snapshot} onClick={() => setUpdateQuestion({ ...updateTarget, backupWorlds: true })}>Actualizar…</button>}
    {!active && repairAllowed && <button type="button" disabled={busy || !isTauri() || !play?.snapshot} onClick={() => void act('begin_repair', { requestId: crypto.randomUUID() })}>Reparar archivos</button>}
    {!active && (cleanupAllowed || entry?.phase === 'uninstalled') && <button type="button" disabled={busy || !isTauri() || !play?.snapshot} onClick={() => void cleanNatives()}>Limpiar nativos antiguos</button>}
    {cleanupText && <p role="status">{cleanupText}</p>}
    {cleanupDetails.length > 0 && <details><summary>Por qué se conservaron estas carpetas</summary>
      <ul>{cleanupDetails.map(detail => <li key={detail.directory}><code>{detail.directory}</code>: {nativeRetentionReason(detail.reason)}</li>)}</ul>
    </details>}
    {cleanupError && <p role="alert">{cleanupError}</p>}
    {!active && uninstallAllowed && <button type="button" disabled={busy || !isTauri() || !play?.snapshot} onClick={() => setUninstallQuestion(true)}>Desinstalar…</button>}
    {!active && uninstallQuestion && <fieldset className="catalog-notice">
      <legend>Desinstalar conservando mis datos</legend>
      <p>Quitaremos esta instalación de las disponibles para jugar y eliminaremos solo sus archivos nativos verificados. Conservaremos tus mundos, ajustes, capturas, copias de mundos y archivos personales. Los archivos compartidos de Minecraft y Java también se conservan.</p>
      <p>Puedes reinstalar esta instancia cuando esté habilitada en el catálogo. Esta acción no elimina todos sus archivos ni libera el espacio de las cachés compartidas.</p>
      <div className="account-buttons">
        <button type="button" disabled={busy} onClick={() => { setUninstallQuestion(false); void act('begin_uninstall', { requestId: crypto.randomUUID() }); }}>Confirmar desinstalación</button>
        <button type="button" disabled={busy} onClick={() => setUninstallQuestion(false)}>Volver</button>
      </div>
    </fieldset>}
    {!active && updateQuestion && <fieldset className="catalog-notice">
      <legend>Actualizar de Minecraft {updateQuestion.fromVersion} a {updateQuestion.toVersion}</legend>
      <p>Una versión nueva puede modificar tus mundos cuando los abras. Volver a los archivos de una versión anterior no recupera los cambios en tus mundos.</p>
      <label><input type="radio" name={`${instanceId}-backup`} checked={updateQuestion.backupWorlds} onChange={() => setUpdateQuestion({ ...updateQuestion, backupWorlds: true })} /> Crear una copia de mis mundos antes de actualizar (recomendado)</label>
      <label><input type="radio" name={`${instanceId}-backup`} checked={!updateQuestion.backupWorlds} onChange={() => setUpdateQuestion({ ...updateQuestion, backupWorlds: false })} /> Actualizar sin copia de mundos</label>
      <div className="account-buttons">
        <button type="button" disabled={busy} onClick={() => {
          const choice = updateQuestion;
          setUpdateQuestion(null);
          void act('begin_update', { requestId: crypto.randomUUID(), update: { from_version: choice.fromVersion, to_version: choice.toVersion, backup_worlds: choice.backupWorlds } });
        }}>Confirmar actualización</button>
        <button type="button" disabled={busy} onClick={() => setUpdateQuestion(null)}>Conservar versión instalada</button>
      </div>
    </fieldset>}
  </div>;
}
