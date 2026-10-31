import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { ChevronDown, CircleArrowUp, Download, Play, FolderOpen, FileText, Database, RefreshCw, Settings2, Trash2, Wrench, X, LoaderCircle, Square, ShieldCheck, TriangleAlert } from 'lucide-react';
import { InstanceProgress } from './components/instance-progress';
import { AccountDialog } from './components/account-dialog';
import { TransientNotice, useTransientNotice } from './components/transient-notice';
import { Button } from './components/ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from './components/ui/dropdown-menu';
import { messageForError } from './errors';
import { acceptPlaySnapshot, playActive, type PlaySnapshot } from './play-state';
import { useLauncherUpdate } from './update';
import { blocksPlay } from './update-state';
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

export function PlayNotice({ names = {} }: { names?: Record<string, string> }) {
  const play = useContext(PlayContext);
  const toast = useTransientNotice();
  const seen = useRef(new Set<string>());
  useEffect(() => {
    for (const entry of play?.snapshot?.instances ?? []) {
      const key = entry.requestId + ':' + entry.phase;
      if (seen.current.has(key)) continue;
      const message = entry.phase === 'installed' ? (entry.operation === 'update' ? 'Actualización completada.' : entry.operation === 'repair' ? 'Reparación completada.' : 'Instalación completada.')
        : entry.phase === 'uninstalled' ? (entry.eraseRequested ? 'Instancia eliminada.' : 'Instancia desinstalada. Tus datos personales se conservan.')
        : entry.phase === 'cancelled' ? 'Operación cancelada.' : null;
      if (message) { seen.current.add(key); toast.show((names[entry.instanceId] ? names[entry.instanceId] + ': ' : '') + message); }
    }
  }, [play?.snapshot, toast.show, names]);
  return <><TransientNotice notice={toast.notice} />
    {play?.error && <p role="alert">{play.error}</p>}
    {play?.snapshot?.closeBlocked && <p className="catalog-notice" role="alert">No pudimos cerrar esta ventana. Cancela la instalación o el inicio pendiente y vuelve a intentarlo. Puedes minimizar esta ventana.</p>}
  </>;
}

/** Progress of running operations floats over every screen, so players keep using the launcher; it closes by itself when work ends. */
export function OperationDock({ names = {} }: { names?: Record<string, string> }) {
  const play = useContext(PlayContext);
  const entries = (play?.snapshot?.instances ?? []).filter(entry => playActive(entry.phase) && entry.phase !== 'running' && entry.phase !== 'stopping' && entry.phase !== 'warning');
  if (entries.length === 0) return null;
  return <div className="operation-dock" aria-label="Operaciones en curso">
    {entries.map(entry => <InstanceProgress key={entry.requestId} entry={entry} busy={false} title={names[entry.instanceId]}
      onCancel={() => void play?.act('cancel_play', { instanceId: entry.instanceId, requestId: entry.requestId })} />)}
  </div>;
}

export function usePlaySnapshot() {
  return useContext(PlayContext)?.snapshot ?? null;
}

export function PlayControls({ instanceId, allowed, installAllowed = false, updateTarget = null, repairAllowed = false, uninstallAllowed = false, cleanupAllowed = false, pack = false, onSettings, onPackCache, removalPending = false, optionsAllowed = false }: { instanceId: string; allowed: boolean; installAllowed?: boolean; updateTarget?: UpdateTarget | null; repairAllowed?: boolean; uninstallAllowed?: boolean; cleanupAllowed?: boolean; pack?: boolean; onSettings?: () => void; onPackCache?: () => void; removalPending?: boolean; optionsAllowed?: boolean }) {
  const play = useContext(PlayContext);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const [forceQuestion, setForceQuestion] = useState<string | null>(null);
  const [updateQuestion, setUpdateQuestion] = useState<(UpdateTarget & { backupWorlds: boolean }) | null>(null);
  const [uninstallQuestion, setUninstallQuestion] = useState(false);
  const [deleteInstance, setDeleteInstance] = useState(false);
  const [eraseConfirmed, setEraseConfirmed] = useState(false);
  const [instanceFolder, setInstanceFolder] = useState<string | null>(null);
  const uninstallDialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    let current = true;
    if (uninstallQuestion) {
      if (!uninstallDialog.current?.open) uninstallDialog.current?.showModal();
      if (isTauri()) void invoke<string>('instance_folder_path', { instanceId }).then(path => { if (current) setInstanceFolder(path); }).catch(cause => { if (current) setLogError(messageForError(cause)); });
    } else uninstallDialog.current?.close();
    return () => { current = false; };
  }, [uninstallQuestion, instanceId]);
  function askUninstall() { setLogError(null); setDeleteInstance(false); setEraseConfirmed(false); setInstanceFolder(null); setUninstallQuestion(true); }
  const [names, setNames] = useState<VersionNames | null>(null);
  const [lastBackup, setLastBackup] = useState(true);
  const [logError, setLogError] = useState<string | null>(null);
  const [cleanupText, setCleanupText] = useState<string | null>(null);
  const toast = useTransientNotice();
  const [cleanupError, setCleanupError] = useState<string | null>(null);
  const [cleanupDetails, setCleanupDetails] = useState<NativeCleanupReport['details']>([]);
  const entry = play?.snapshot?.instances.find(entry => entry.instanceId === instanceId);
  const [optionsOpen, setOptionsOpen] = useState(false);
  const active = entry && playActive(entry.phase);
  // A required launcher update replaces Play; installing and updating instances stay available.
  const launcherUpdate = useLauncherUpdate();
  const launcherRequired = !active && blocksPlay(launcherUpdate?.view ?? null);
  const running = entry && (entry.phase === 'running' || entry.phase === 'stopping');
  const optionsVisible = optionsAllowed && entry?.phase !== 'uninstalled';
  const primaryDisabled = active ? !running || entry.phase === 'stopping'
    : updateTarget ? busy
    : (installAllowed && entry?.phase !== 'installed') || entry?.phase === 'uninstalled'
      ? busy || !installAllowed || !isTauri() || !play?.snapshot
      : busy || !allowed || !isTauri() || !play?.snapshot;
  const optionsEnabled = optionsVisible && !primaryDisabled;
  useEffect(() => { if (!optionsEnabled) setOptionsOpen(false); }, [optionsEnabled]);

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
    setCleanupText(null);
    toast.show('Eliminando las carpetas nativas antiguas…', 'progress');
    try {
      const report = await invoke<NativeCleanupReport>('clean_native_cache', { instanceId });
      if (report.retainedGenerations > 0) { toast.clear(); setCleanupText(nativeCleanupSummary(report)); }
      else { setCleanupText(null); toast.show('Caché nativa limpia.'); }
      setCleanupDetails(report.details);
    } catch (cause) {
      setCleanupText(null);
      toast.clear(); setCleanupError(messageForError(cause));
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }

  const updateDialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const dialog = updateDialog.current;
    if (!dialog) return;
    if (updateQuestion && !active) {
      if (!dialog.open) dialog.showModal();
    } else if (dialog.open) dialog.close();
  }, [updateQuestion, active]);

  function askUpdate() {
    if (!updateTarget) return;
    setUpdateQuestion({ ...updateTarget, backupWorlds: true });
    if (updateTarget.pack && isTauri()) {
      setNames(null);
      void invoke<VersionNames>('pack_version_names', { instanceId }).then(setNames).catch(() => setNames(null));
    }
  }

  return <div className="play-controls">
    <TransientNotice notice={toast.notice} />
    {logError && <p role="alert">{logError}</p>}
    {entry?.consoleError && <p role="alert">Minecraft se inició, pero no pudimos abrir su consola automáticamente. Puedes volver a intentarlo con «Ver registro». {messageForError(entry.consoleError)}</p>}
    {entry && (entry.operation === 'repair' || entry.operation === 'uninstall') && !active && entry.retainedFiles > 0 && <p role="status">Conservamos {entry.retainedFiles} archivos nativos modificados o que no pudimos comprobar.</p>}
    {entry?.nativeCleanup && !active && entry.nativeCleanup.retainedGenerations > 0 && <div role="status">
      <p>{nativeCleanupSummary(entry.nativeCleanup)}</p>
      {entry.nativeCleanup.retainedGenerations > 0 && <p>Cierra los programas que usen esas carpetas y vuelve a intentar «Limpiar nativos antiguos».</p>}
      {entry.nativeCleanup.details.length > 0 && <details><summary>Detalle de la limpieza automática</summary>
        <ul>{entry.nativeCleanup.details.map(detail => <li key={detail.directory}><code>{detail.directory}</code>: {nativeRetentionReason(detail.reason)}</li>)}</ul>
      </details>}
    </div>}
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
    {forceQuestion && entry?.requestId === forceQuestion && <AccountDialog title="Cerrar Minecraft" onCancel={() => setForceQuestion(null)}>
      <p>Se cerrará el juego inmediatamente. Puedes perder cambios sin guardar.</p>
      <div className="account-buttons"><Button variant="destructive" disabled={busy} onClick={() => { setForceQuestion(null); void act('stop_game', { requestId: entry.requestId, force: true }); }}><Square aria-hidden="true" />Confirmar cierre forzoso</Button><Button variant="outline" onClick={() => setForceQuestion(null)}><X aria-hidden="true" />Cancelar</Button></div>
    </AccountDialog>}
    <div className="instance-actions">
    <div className="split-button" role="group" aria-label="Controles de la instancia">
    {active ? <Button className={`primary-play h-12 text-xl ${running && entry.phase !== 'stopping' ? 'bg-danger text-foreground hover:bg-danger-hover' : 'busy-play'}`} size="lg" aria-disabled={!running || entry.phase === 'stopping'} onClick={() => { if (running && entry && entry.phase !== 'stopping') setForceQuestion(entry.requestId); }}>
      {running && entry.phase !== 'stopping' ? <Square aria-hidden="true" /> : <LoaderCircle className="spin" aria-hidden="true" />}{running ? entry.phase === 'stopping' ? 'Cerrando…' : 'Cerrar forzosamente…' : 'Preparando…'}
    </Button> : updateTarget ? <Button className="primary-play h-12 text-xl" size="lg" disabled={busy} onClick={askUpdate}><RefreshCw aria-hidden="true" />Actualizar…</Button>
      : (installAllowed && entry?.phase !== 'installed') || entry?.phase === 'uninstalled' ? <Button className="primary-play h-12 text-xl" size="lg" disabled={busy || !installAllowed || !isTauri() || !play?.snapshot} onClick={() => void act('begin_install', { requestId: crypto.randomUUID() })}><Download aria-hidden="true" />{entry?.phase === 'failed' ? 'Reintentar instalación' : 'Instalar'}</Button>
      : launcherRequired ? <Button className="primary-play h-12 text-xl" size="lg" aria-disabled="true" title="Actualiza Panita Client para poder jugar. El botón de versión de la barra lateral inicia la actualización." onClick={() => launcherUpdate?.highlight()}><CircleArrowUp aria-hidden="true" />Actualización requerida</Button>
      : <Button className="primary-play h-12 text-xl" size="lg" disabled={busy || !allowed || !isTauri() || !play?.snapshot} onClick={() => void act('begin_play', { requestId: crypto.randomUUID() })}><Play aria-hidden="true" />{allowed ? 'Jugar' : 'No disponible para jugar'}</Button>}
    {optionsVisible && <>
    <DropdownMenu modal={false} open={optionsEnabled && optionsOpen} onOpenChange={open => setOptionsOpen(optionsEnabled && open)}>
      <DropdownMenuTrigger asChild><Button className={`split-trigger h-12 w-12 rounded-l-none p-0 ${running && entry?.phase !== 'stopping' ? 'bg-danger text-foreground hover:bg-danger-hover' : ''}`} disabled={!optionsEnabled} aria-label="Opciones de instancia" title={optionsEnabled ? 'Opciones de instancia' : 'Opciones no disponibles en este momento'}><ChevronDown aria-hidden="true" /></Button></DropdownMenuTrigger>
      <DropdownMenuContent className="instance-menu" align="end" sideOffset={8}>
        {onSettings && <DropdownMenuItem onSelect={onSettings}><Settings2 aria-hidden="true" />Ajustes de la instancia</DropdownMenuItem>}
        <DropdownMenuItem disabled={busy || !isTauri()} onSelect={() => void invoke('open_instance_folder', { instanceId, folder: 'instance' }).catch(cause => setLogError(messageForError(cause)))}><FolderOpen aria-hidden="true" />Abrir carpeta</DropdownMenuItem>
        <DropdownMenuItem disabled={busy || !isTauri() || entry?.operation !== 'play' || !(running || entry?.phase === 'exited')} onSelect={() => { if (entry) void invoke('open_game_log', { instanceId, requestId: entry.requestId }).catch(cause => setLogError(messageForError(cause))); }}><FileText aria-hidden="true" />Ver registro</DropdownMenuItem>
        {pack && onPackCache && <DropdownMenuItem onSelect={onPackCache}><Database aria-hidden="true" />Limpiar caché de modpacks…</DropdownMenuItem>}
        {running && entry && <>
          {play?.snapshot?.gracefulStopSupported && entry.phase !== 'stopping' && <DropdownMenuItem disabled={busy} onSelect={() => void act('stop_game', { requestId: entry.requestId, force: false })}><Square aria-hidden="true" />Solicitar cierre</DropdownMenuItem>}
        </>}
        {!active && repairAllowed && <DropdownMenuItem disabled={busy || !isTauri() || !play?.snapshot} onSelect={() => void act('begin_repair', { requestId: crypto.randomUUID() })}><Wrench aria-hidden="true" />Reparar archivos</DropdownMenuItem>}
        {!active && (cleanupAllowed || entry?.phase === 'uninstalled') && <DropdownMenuItem disabled={busy || !isTauri() || !play?.snapshot} onSelect={() => void cleanNatives()}><RefreshCw aria-hidden="true" />Limpiar nativos antiguos</DropdownMenuItem>}
        {!active && uninstallAllowed && <><DropdownMenuSeparator /><DropdownMenuItem className="text-destructive" disabled={busy} onSelect={askUninstall}><Trash2 aria-hidden="true" />Desinstalar…</DropdownMenuItem></>}
        {!onSettings && !repairAllowed && !cleanupAllowed && !uninstallAllowed && !updateTarget && <DropdownMenuItem disabled>No hay opciones disponibles</DropdownMenuItem>}
      </DropdownMenuContent>
    </DropdownMenu>
    </>}
    </div>
    </div>
    {cleanupText && <p role="status">{cleanupText}</p>}
    {cleanupDetails.length > 0 && <details><summary>Por qué se conservaron estas carpetas</summary>
      <ul>{cleanupDetails.map(detail => <li key={detail.directory}><code>{detail.directory}</code>: {nativeRetentionReason(detail.reason)}</li>)}</ul>
    </details>}
    {cleanupError && <p role="alert">{cleanupError}</p>}
    <dialog ref={uninstallDialog} className="update-dialog" aria-labelledby={instanceId + '-uninstall-title'} onCancel={() => setUninstallQuestion(false)} onClose={() => { if (!uninstallDialog.current?.open) setUninstallQuestion(false); }}>
    {uninstallQuestion && <>
      <h2 id={instanceId + '-uninstall-title'}>Desinstalar instancia</h2>
      {!deleteInstance && <>
      <p>La instancia dejará de estar lista para jugar. Conservaremos tus mundos, ajustes, capturas, mods añadidos y copias de seguridad.</p>
      <p>{pack ? "Se quitarán los archivos del modpack sin cambios y sus carpetas nativas." : "Se limpiarán sus carpetas nativas."} Puedes reinstalarla después.</p>
      </>}
      <label className="backup-choice"><input type="checkbox" checked={deleteInstance} onChange={event => { setDeleteInstance(event.target.checked); setEraseConfirmed(false); }} /><span>Eliminar también la carpeta de la instancia</span></label>
      {removalPending && <p role="alert">Hay una eliminación pendiente. Marca la opción de eliminar la carpeta para reintentarla.</p>}
      {deleteInstance && <div className="erase-warning">
        {logError && <p role="alert">No pudimos verificar la carpeta. Cierra este diálogo y vuelve a intentarlo. {logError}</p>}
        <p>Se eliminará permanentemente esta carpeta y todo su contenido:</p>
        <p><code>{instanceFolder ?? (isTauri() ? 'Consultando carpeta…' : 'Carpeta de esta instancia · vista previa')}</code></p>
        <p>Se borrarán mundos, capturas, ajustes, mods y copias de seguridad. No se puede deshacer. Tus cuentas, otras instancias y cachés se conservan.</p>
        <label className="backup-choice"><input type="checkbox" checked={eraseConfirmed} onChange={event => setEraseConfirmed(event.target.checked)} /><span>Entiendo que perderé todos los datos de esta instancia</span></label>
      </div>}
      <div className="account-buttons">
        <Button variant="destructive" disabled={busy || !isTauri() || !play?.snapshot || (removalPending && !deleteInstance) || (deleteInstance && (!eraseConfirmed || !instanceFolder))} onClick={() => { setUninstallQuestion(false); void act('begin_uninstall', { requestId: crypto.randomUUID(), deleteInstance }); }}><Trash2 aria-hidden="true" />{deleteInstance ? 'Eliminar instancia' : 'Desinstalar'}</Button>
        <Button variant="secondary" disabled={busy} onClick={() => setUninstallQuestion(false)}><X aria-hidden="true" />Cancelar</Button>
      </div>
    </>}
    </dialog>
    <dialog ref={updateDialog} className="update-dialog" aria-labelledby={instanceId + '-update-title'} aria-describedby={instanceId + '-update-warning'} onCancel={() => setUpdateQuestion(null)} onClose={() => { if (!updateDialog.current?.open) setUpdateQuestion(null); }}>
    {updateQuestion && <>
      <h2 id={instanceId + '-update-title'} className="dialog-title"><CircleArrowUp aria-hidden="true" />{updateQuestion.pack ? versionTitle(names, updateQuestion.fromVersion, updateQuestion.toVersion) : `Actualizar Minecraft (${updateQuestion.fromVersion} → ${updateQuestion.toVersion})`}</h2>
      <ul className="update-notes">
        {updateQuestion.pack && <li><ShieldCheck aria-hidden="true" />Actualizaremos solo los archivos del modpack que no hayas cambiado.</li>}
        {updateQuestion.pack && <li><ShieldCheck aria-hidden="true" />Tus mundos, mods añadidos y ajustes editados se conservan.</li>}
        <li id={instanceId + '-update-warning'} className="is-warning"><TriangleAlert aria-hidden="true" />Abrir mundos con otra versión puede cambiarlos. Volver de versión no deshace esos cambios.</li>
      </ul>
      <label className="backup-choice"><input type="checkbox" checked={updateQuestion.backupWorlds} onChange={event => setUpdateQuestion({ ...updateQuestion, backupWorlds: event.target.checked })} /><span>Crear copia de mis mundos (recomendado)</span></label>
      <div className="account-buttons">
        <Button disabled={busy || !isTauri() || !play?.snapshot} onClick={() => {
          const choice = updateQuestion;
          setUpdateQuestion(null);
          setLastBackup(choice.backupWorlds);
          void act('begin_update', { requestId: crypto.randomUUID(), update: { from_version: choice.fromVersion, to_version: choice.toVersion, backup_worlds: choice.backupWorlds } });
        }}><RefreshCw aria-hidden="true" />Actualizar</Button>
        <Button variant="secondary" disabled={busy} onClick={() => setUpdateQuestion(null)}><X aria-hidden="true" />Cancelar</Button>
      </div>
    </>}
    </dialog>
  </div>;
}
