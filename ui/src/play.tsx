import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { ChevronDown, Download, Play, RefreshCw, Settings2, Trash2, Wrench } from 'lucide-react';
import { Button } from './components/ui/button';
import { messageForError } from './errors';
import { acceptPlaySnapshot, playActive, type PlayPhase, type PlaySnapshot } from './play-state';
import type { UpdateTarget } from './library-state';
import { invokePlay, type PlayArguments, type PlayCommand } from './play-api';
import { nativeCleanupSummary, nativeRetentionReason, type NativeCleanupReport } from './native-cleanup-state';
import {
  backedUpSummary, conflictReasonText, conflictsResolvable, folderHint, manualHeading, reasonText, releasedSummary,
  skippedSummary, updateConflictHeading, versionTitle, type VersionNames,
} from './pack-state';

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
  awaiting_files: 'Instalamos casi todo. Faltan archivos que debes descargar tú mismo para poder jugar.',
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

export function PlayControls({ instanceId, allowed, installAllowed = false, updateTarget = null, repairAllowed = false, uninstallAllowed = false, cleanupAllowed = false, pack = false, onSettings }: { instanceId: string; allowed: boolean; installAllowed?: boolean; updateTarget?: UpdateTarget | null; repairAllowed?: boolean; uninstallAllowed?: boolean; cleanupAllowed?: boolean; pack?: boolean; onSettings?: () => void }) {
  const play = useContext(PlayContext);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const [forceQuestion, setForceQuestion] = useState<string | null>(null);
  const [updateQuestion, setUpdateQuestion] = useState<(UpdateTarget & { backupWorlds: boolean }) | null>(null);
  const [uninstallQuestion, setUninstallQuestion] = useState(false);
  const [names, setNames] = useState<VersionNames | null>(null);
  const [lastBackup, setLastBackup] = useState(true);
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
    setCleanupText('Eliminando las carpetas nativas antiguas…');
    try {
      const report = await invoke<NativeCleanupReport>('clean_native_cache', { instanceId });
      setCleanupText(nativeCleanupSummary(report));
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
      : entry.operation === 'update' && entry.phase === 'installing' ? (pack ? 'Actualizando el modpack. Si algo falla, volvemos a la versión instalada.' : 'Preparando la actualización. La versión instalada se conserva hasta completar la preparación.')
      : entry.operation === 'update' && entry.phase === 'installed' ? 'Actualización completa. Ya puedes jugar.'
      : entry.operation === 'update' && entry.phase === 'failed' ? 'No pudimos completar la actualización.'
      : entry.operation === 'update' && entry.phase === 'cancelled' ? 'Actualización cancelada.'
      : entry.operation === 'repair' && entry.phase === 'installing' ? (pack ? 'Reparando los archivos de la versión instalada del modpack…' : 'Reparando los archivos de la versión registrada…')
      : entry.operation === 'repair' && entry.phase === 'installed' ? 'Reparación completa. Ya puedes jugar.'
      : entry.operation === 'repair' && entry.phase === 'failed' ? 'No pudimos completar la reparación.'
      : entry.operation === 'repair' && entry.phase === 'cancelled' ? 'Reparación cancelada.'
      : entry.operation === 'uninstall' && entry.phase === 'installing' ? (pack ? 'Quitando los archivos del modpack que no modificaste…' : 'Desinstalando y conservando tus datos…')
      : entry.operation === 'uninstall' && entry.phase === 'failed' ? 'No pudimos completar la desinstalación. Conservamos los archivos restantes para recuperar la operación.'
      : entry.operation === 'uninstall' && entry.phase === 'cancelled' ? 'Desinstalación cancelada.'
      : entry.operation === 'uninstall' && entry.phase === 'uninstalled' && (entry.nativeCleanup?.retainedGenerations ?? 0) > 0 ? 'La instancia está desinstalada. La limpieza de nativos sigue pendiente.'
      : phaseText[entry.phase]}</p>}
    {entry && entry.operation !== 'play' && entry.operation !== 'uninstall' && active && <div>
      <progress aria-label={entry.operation === 'update' ? 'Actualización de Minecraft y Java' : entry.operation === 'repair' ? 'Reparación de Minecraft y Java' : 'Instalación de Minecraft y Java'} />
      <p>Archivos recibidos: {entry.receivedFiles}. Archivos reutilizados: {entry.reusedFiles}.</p>
      <p>La instalación termina cuando se comprueban todos los archivos y Java.</p>
      {entry.operation === 'update' && entry.backupRequested && <p>La copia de mundos debe completarse antes de cambiar la versión.</p>}
    </div>}
    {entry && (entry.operation === 'repair' || entry.operation === 'uninstall') && !active && entry.retainedFiles > 0 && <p role="status">Conservamos {entry.retainedFiles} archivos nativos modificados o que no pudimos comprobar.</p>}
    {entry?.nativeCleanup && !active && <div role="status">
      <p>{nativeCleanupSummary(entry.nativeCleanup)}</p>
      {entry.nativeCleanup.retainedGenerations > 0 && <p>Cierra los programas que usen esas carpetas y vuelve a intentar «Limpiar nativos antiguos».</p>}
      {entry.nativeCleanup.details.length > 0 && <details><summary>Detalle de la limpieza automática</summary>
        <ul>{entry.nativeCleanup.details.map(detail => <li key={detail.directory}><code>{detail.directory}</code>: {nativeRetentionReason(detail.reason)}</li>)}</ul>
      </details>}
    </div>}
    {entry?.operation === 'update' && entry.phase === 'installed' && <p>{entry.backupOperation
      ? `Copia de mundos guardada en la carpeta de esta instancia: world-backups/${entry.backupOperation}/worlds.zip.`
      : entry.backupRequested ? 'No había mundos locales para copiar.' : 'Actualizaste sin crear una copia de mundos.'}</p>}
    {entry?.error && entry.error !== 'operation_cancelled' && <p className="account-error" role="alert">{messageForError(entry.error)}</p>}
    {!active && entry?.operation === 'update' && entry.error === 'pack_conflict' && (entry.packConflicts?.length ?? 0) > 0 && updateTarget && <fieldset className="catalog-notice">
      <legend>{updateConflictHeading(entry.packConflicts?.length ?? 0)}</legend>
      <p>No cambiamos nada: tu instalación sigue como estaba.</p>
      <ul>{(entry.packConflicts ?? []).map(conflict => <li key={conflict.path}><code>{conflict.path}</code>: {conflictReasonText(conflict.reason)}</li>)}</ul>
      {conflictsResolvable(entry.packConflicts ?? [])
        ? <>
          <p>Si continúas, movemos tus archivos a la carpeta <code>pack-backups</code> de esta instancia (no los borramos) e instalamos los del modpack.</p>
          <div className="account-buttons">
            <button type="button" disabled={busy || !isTauri()} onClick={() => void act('begin_update', { requestId: crypto.randomUUID(), update: { from_version: updateTarget.fromVersion, to_version: updateTarget.toVersion, backup_worlds: lastBackup, move_aside: true } })}>Continuar y guardar mis archivos</button>
          </div>
        </>
        : <p>Mueve esa carpeta o enlace fuera de la instancia y vuelve a actualizar.</p>}
    </fieldset>}
    {!active && entry?.operation === 'update' && entry.error === 'manual_files_required' && (entry.packAwaiting?.length ?? 0) > 0 && updateTarget && <fieldset className="catalog-notice">
      <legend>{manualHeading(entry.packAwaiting?.length ?? 0)}</legend>
      <p>No cambiamos nada. Descarga estos archivos, guárdalos en la carpeta indicada y vuelve a pulsar «Actualizar…».</p>
      <ul>{(entry.packAwaiting ?? []).map(file => <li key={file.path}>
        <strong>{file.name}</strong> (<code>{file.fileName}</code>)
        <p>{reasonText(file.reason)} {folderHint(file)}</p>
        <div className="account-buttons">
          <button type="button" disabled={busy || !isTauri()} onClick={() => void invoke('open_curseforge_project', { projectId: file.projectId }).catch(cause => setLogError(messageForError(cause)))}>Abrir CurseForge</button>
          <button type="button" disabled={busy || !isTauri()} onClick={() => void invoke('open_instance_folder', { instanceId, folder: file.folder }).catch(cause => setLogError(messageForError(cause)))}>Abrir carpeta</button>
        </div>
      </li>)}</ul>
    </fieldset>}
    {!active && (entry?.operation === 'update' || entry?.operation === 'repair') && entry.phase === 'installed' && ((entry.packSkipped?.length ?? 0) > 0 || (entry.packReleased?.length ?? 0) > 0 || (entry.packBackedUp?.length ?? 0) > 0) && <div role="status">
      {(entry.packSkipped?.length ?? 0) > 0 && <p>{skippedSummary(entry.packSkipped ?? [])}</p>}
      {(entry.packReleased?.length ?? 0) > 0 && <p>{releasedSummary(entry.packReleased ?? [])}</p>}
      {(entry.packBackedUp?.length ?? 0) > 0 && <>
        <p>{backedUpSummary(entry.packBackedUp ?? [])}</p>
        <button type="button" disabled={busy || !isTauri()} onClick={() => void invoke('open_instance_folder', { instanceId, folder: 'pack-backups' }).catch(cause => setLogError(messageForError(cause)))}>Abrir copias de archivos</button>
      </>}
    </div>}
    {entry?.phase === 'warning' && <div className="catalog-notice" role="alert">
      <p>No pudimos comprobar si esta instancia sigue habilitada. Usaremos la instalación local verificada. Si usas una cuenta Microsoft, necesitas conexión para verificarla.</p>
      {pack && <p>Tampoco podemos comprobar si hay una versión más nueva del modpack, así que esta instalación puede estar desactualizada.</p>}
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
    <div className="instance-actions">
    {!active && installAllowed && entry?.phase !== 'installed' && <Button className="primary-play" size="lg" disabled={busy || !isTauri() || !play?.snapshot} onClick={() => void act('begin_install', { requestId: crypto.randomUUID() })}><Download aria-hidden="true" />{entry?.operation === 'install' ? 'Reintentar instalación' : 'Instalar'}</Button>}
    {!active && !installAllowed && !updateTarget?.pack && <Button className="primary-play" size="lg" disabled={busy || !allowed || entry?.phase === 'uninstalled' || !isTauri() || !play?.snapshot} onClick={() => void act('begin_play', { requestId: crypto.randomUUID() })}><Play aria-hidden="true" />{allowed && entry?.phase !== 'uninstalled' ? 'Jugar' : 'No disponible para jugar'}</Button>}
    {!active && updateTarget?.pack && <p role="status">Hay una versión nueva de este modpack. Debes actualizarla antes de jugar.</p>}
    {!active && updateTarget && <Button className={updateTarget.pack ? 'primary-play' : undefined} variant={updateTarget.pack ? 'default' : 'secondary'} size="lg" disabled={busy || !isTauri() || !play?.snapshot} onClick={() => {
      setUpdateQuestion({ ...updateTarget, backupWorlds: true });
      if (updateTarget.pack) {
        setNames(null);
        void invoke<VersionNames>('pack_version_names', { instanceId }).then(setNames).catch(() => setNames(null));
      }
    }}><RefreshCw aria-hidden="true" />Actualizar…</Button>}
    <details className="instance-options"><summary><Settings2 aria-hidden="true" />Opciones de instancia<ChevronDown aria-hidden="true" /></summary><div className="options-list">
      {onSettings && <Button variant="ghost" onClick={onSettings}><Settings2 aria-hidden="true" />Ajustes de la instancia</Button>}
      {!active && repairAllowed && <Button variant="ghost" disabled={busy || !isTauri() || !play?.snapshot} onClick={() => void act('begin_repair', { requestId: crypto.randomUUID() })}><Wrench aria-hidden="true" />Reparar archivos</Button>}
      {!active && (cleanupAllowed || entry?.phase === 'uninstalled') && <Button variant="ghost" disabled={busy || !isTauri() || !play?.snapshot} onClick={() => void cleanNatives()}><RefreshCw aria-hidden="true" />Limpiar nativos antiguos</Button>}
      {!active && uninstallAllowed && <Button variant="ghost" className="text-destructive" disabled={busy || !isTauri() || !play?.snapshot} onClick={() => setUninstallQuestion(true)}><Trash2 aria-hidden="true" />Desinstalar…</Button>}
    </div></details>
    </div>
    {cleanupText && <p role="status">{cleanupText}</p>}
    {cleanupDetails.length > 0 && <details><summary>Por qué se conservaron estas carpetas</summary>
      <ul>{cleanupDetails.map(detail => <li key={detail.directory}><code>{detail.directory}</code>: {nativeRetentionReason(detail.reason)}</li>)}</ul>
    </details>}
    {cleanupError && <p role="alert">{cleanupError}</p>}
    {!active && uninstallQuestion && <fieldset className="catalog-notice">
      <legend>Desinstalar conservando mis datos</legend>
      {pack
        ? <p>Quitaremos esta instalación de las disponibles para jugar y eliminaremos solo los mods, packs de recursos y shaders del modpack que no hayas modificado. Conservaremos tus mundos, capturas, tu configuración, los archivos que modificaste o añadiste tú y tus copias de mundos. También eliminaremos las carpetas nativas de esta instancia. Los archivos compartidos de Minecraft y Java también se conservan.</p>
        : <p>Quitaremos esta instalación de las disponibles para jugar y eliminaremos todas sus carpetas nativas, incluidos archivos antiguos, modificados o añadidos allí. Conservaremos tus mundos, ajustes, capturas y copias de mundos en la carpeta de Minecraft. Los archivos compartidos de Minecraft y Java también se conservan.</p>}
      <p>Puedes reinstalar esta instancia cuando esté habilitada en el catálogo. Esta acción no elimina todos sus archivos ni libera el espacio de las cachés compartidas.</p>
      <div className="account-buttons">
        <button type="button" disabled={busy} onClick={() => { setUninstallQuestion(false); void act('begin_uninstall', { requestId: crypto.randomUUID() }); }}>Confirmar desinstalación</button>
        <button type="button" disabled={busy} onClick={() => setUninstallQuestion(false)}>Volver</button>
      </div>
    </fieldset>}
    {!active && updateQuestion && <fieldset className="catalog-notice">
      <legend>{updateQuestion.pack ? versionTitle(names, updateQuestion.fromVersion, updateQuestion.toVersion) : `Actualizar de Minecraft ${updateQuestion.fromVersion} a ${updateQuestion.toVersion}`}</legend>
      {updateQuestion.pack && <p>Cambiaremos solo los archivos del modpack que no hayas modificado. Conservamos los mods que añadiste, tu configuración editada y tus mundos. Si algún archivo tuyo estorba, te lo mostraremos antes de cambiar nada.</p>}
      <p>Una versión nueva puede modificar tus mundos cuando los abras. Volver a los archivos de una versión anterior no recupera los cambios en tus mundos.</p>
      <label><input type="radio" name={`${instanceId}-backup`} checked={updateQuestion.backupWorlds} onChange={() => setUpdateQuestion({ ...updateQuestion, backupWorlds: true })} /> Crear una copia de mis mundos antes de actualizar (recomendado)</label>
      <label><input type="radio" name={`${instanceId}-backup`} checked={!updateQuestion.backupWorlds} onChange={() => setUpdateQuestion({ ...updateQuestion, backupWorlds: false })} /> Actualizar sin copia de mundos</label>
      <div className="account-buttons">
        <button type="button" disabled={busy} onClick={() => {
          const choice = updateQuestion;
          setUpdateQuestion(null);
          setLastBackup(choice.backupWorlds);
          void act('begin_update', { requestId: crypto.randomUUID(), update: { from_version: choice.fromVersion, to_version: choice.toVersion, backup_worlds: choice.backupWorlds } });
        }}>Confirmar actualización</button>
        <button type="button" disabled={busy} onClick={() => setUpdateQuestion(null)}>Conservar versión instalada</button>
      </div>
    </fieldset>}
  </div>;
}
