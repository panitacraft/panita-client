import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { ArrowRight, Check, ChevronDown, CircleArrowUp, Download, List, Play, FolderOpen, FileText, Database, RefreshCw, Settings2, Trash2, Wrench, X, LoaderCircle, Square, TriangleAlert } from 'lucide-react';
import { InstanceProgress } from './components/instance-progress';
import { AccountDialog } from './components/account-dialog';
import { VariantDialog } from './components/variant-dialog';
import { TransientNotice, useTransientNotice } from './components/transient-notice';
import { Button } from './components/ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from './components/ui/dropdown-menu';
import { messageForError } from './errors';
import { acceptPlaySnapshot, playActive, type PlaySnapshot } from './play-state';
import { useLauncherUpdate } from './update';
import { blocksPlay } from './update-state';
import type { UpdateTarget } from './library-state';
import { invokePlay, type PlayArguments, type PlayCommand, type UpdateArguments } from './play-api';
import { previewOptions, withdrawnVariantText, type Variant, type VariantChange, type VariantOption } from './variant-state';
import { nativeCleanupSummary, type NativeCleanupReport } from './native-cleanup-state';
import { ConflictsDialog, CompletionDialog, DetailsDialog, ManualFilesDialog, NativeCleanupSection } from './components/notice-details';
import { completionDetails, completionNote, entryNotice, type CompletionDetails } from './notice-state';
import { conflictsResolvable, versionLabels, type VersionNames } from './pack-state';

type PlayContextValue = {
  snapshot: PlaySnapshot | null;
  error: string | null;
  /** The last update asked for an instance: what a conflict or a missing file asks to repeat. */
  lastUpdate: (instanceId: string) => UpdateArguments | null;
  act: <C extends PlayCommand>(command: C, args: PlayArguments[C]) => Promise<void>;
};
const PlayContext = createContext<PlayContextValue | null>(null);
export function PlayProvider({ children }: { children: ReactNode }) {
  const updates = useRef(new Map<string, UpdateArguments>());
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
    if (command === 'begin_update') updates.current.set(args.instanceId, (args as PlayArguments['begin_update']).update);
    try {
      await invokePlay(command, args);
      const next = await invoke<PlaySnapshot>('play_snapshot');
      setSnapshot(previous => acceptPlaySnapshot(previous, next));
    } catch (cause) { setError(messageForError(cause)); }
  }
  return <PlayContext.Provider value={{ snapshot, error, act, lastUpdate: id => updates.current.get(id) ?? null }}>{children}</PlayContext.Provider>;
}

export function PlayNotice({ names = {} }: { names?: Record<string, string> }) {
  const play = useContext(PlayContext);
  const toast = useTransientNotice();
  const seen = useRef(new Set<string>());
  // What a finished operation kept or left behind, told by a toast and read in a modal.
  const [completion, setCompletion] = useState<{ instanceId: string; title: string; details: CompletionDetails } | null>(null);
  const [completionError, setCompletionError] = useState<string | null>(null);
  useEffect(() => {
    for (const entry of play?.snapshot?.instances ?? []) {
      const key = entry.requestId + ':' + entry.phase;
      if (seen.current.has(key)) continue;
      const message = entry.phase === 'installed' ? (entry.operation === 'update' ? 'Actualización completada.' : entry.operation === 'repair' ? 'Reparación completada.' : 'Instalación completada.')
        : entry.phase === 'uninstalled' ? (entry.eraseRequested ? 'Instancia eliminada.' : 'Instancia desinstalada. Tus datos personales se conservan.')
        : entry.phase === 'cancelled' ? 'Operación cancelada.' : null;
      if (!message) continue;
      seen.current.add(key);
      const prefix = names[entry.instanceId] ? names[entry.instanceId] + ': ' : '';
      const details = completionDetails(entry);
      if (!details) { toast.show(prefix + message); continue; }
      toast.show(`${prefix}${message} ${completionNote(details)}`, 'success', {
        label: 'Ver detalles',
        onClick: () => { setCompletionError(null); setCompletion({ instanceId: entry.instanceId, title: prefix + message.replace(/\.$/, ''), details }); },
      });
    }
  }, [play?.snapshot, toast.show, names]);
  return <><TransientNotice notice={toast.notice} />
    {completion && <CompletionDialog title={completion.title} details={completion.details} busy={!isTauri()} error={completionError} onClose={() => setCompletion(null)}
      onOpenBackups={() => { setCompletionError(null); void invoke('open_instance_folder', { instanceId: completion.instanceId, folder: 'pack-backups' }).catch(cause => setCompletionError(messageForError(cause))); }} />}
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

/** The play snapshot and the way to start operations, for screens outside the instance controls. */
export function usePlayControl() {
  return useContext(PlayContext);
}

export function PlayControls({ instanceId, allowed, installAllowed = false, updateTarget = null, repairAllowed = false, uninstallAllowed = false, cleanupAllowed = false, pack = false, onSettings, onPackCache, removalPending = false, optionsAllowed = false, variants = ['standard'], withdrawnVariant = null, automaticSwitch = null }: { instanceId: string; allowed: boolean; installAllowed?: boolean; updateTarget?: UpdateTarget | null; repairAllowed?: boolean; uninstallAllowed?: boolean; cleanupAllowed?: boolean; pack?: boolean; onSettings?: () => void; onPackCache?: () => void; removalPending?: boolean; optionsAllowed?: boolean; variants?: Variant[]; withdrawnVariant?: Variant | null; automaticSwitch?: (VariantChange & { to: Variant }) | null }) {
  const play = useContext(PlayContext);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const [variantDialog, setVariantDialog] = useState<{ options: VariantOption[] | null; error: string | null } | null>(null);
  const [switchNotice, setSwitchNotice] = useState<Variant | null>(null);
  const switched = useRef<string | null>(null);
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
  const toast = useTransientNotice();
  const [cleanupError, setCleanupError] = useState<string | null>(null);
  // The list behind a toast, read in a modal: the player's files in the way, files to
  // download by hand, or native folders a manual cleanup could not remove.
  const [details, setDetails] = useState<'conflicts' | 'manual' | 'natives' | null>(null);
  const [nativeReport, setNativeReport] = useState<NativeCleanupReport | null>(null);
  const announced = useRef(new Set<string>());
  const entry = play?.snapshot?.instances.find(entry => entry.instanceId === instanceId);
  const [optionsOpen, setOptionsOpen] = useState(false);
  const active = entry && playActive(entry.phase);
  // A finished operation that stopped before changing anything is told with a toast, not a block in the page.
  const notice = entry ? entryNotice(entry) : null;
  const noticeKey = entry && notice ? `${entry.requestId}:${entry.phase}:${entry.error}` : null;
  useEffect(() => {
    if (!notice || !noticeKey || announced.current.has(noticeKey)) return;
    announced.current.add(noticeKey);
    const kind = notice.details;
    toast.show(notice.message, 'error', kind ? { label: 'Ver detalles', onClick: () => setDetails(kind) } : undefined);
  }, [noticeKey]);
  // A modal about one operation closes with it: a new request must not reopen it by accident.
  useEffect(() => { if (details === 'conflicts' || details === 'manual') { if (notice?.details !== details) setDetails(null); } }, [details, notice?.details]);
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
  // A disabled instance cannot be played, but its menu must stay reachable so the player can still uninstall it.
  const uninstallReachable = !active && uninstallAllowed && !busy && isTauri() && !!play?.snapshot;
  const optionsEnabled = optionsVisible && (!primaryDisabled || uninstallReachable);
  useEffect(() => { if (!optionsEnabled) setOptionsOpen(false); }, [optionsEnabled]);

  async function act<C extends PlayCommand>(command: C, args: Omit<PlayArguments[C], 'instanceId'>) {
    if (!play || busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    try { await play.act(command, { instanceId, ...args } as PlayArguments[C]); }
    finally { busyRef.current = false; setBusy(false); }
  }

  const choice = variants.length > 1;
  // What a conflict or a missing manual file asks the player to repeat.
  const retry: UpdateArguments | null = updateTarget
    ? { from_version: updateTarget.fromVersion, to_version: updateTarget.toVersion, backup_worlds: lastBackup }
    : play?.lastUpdate(instanceId) ?? null;

  function openVariants() {
    setLogError(null);
    if (!isTauri()) { setVariantDialog({ options: previewOptions(variants), error: null }); return; }
    setVariantDialog({ options: null, error: null });
    void invoke<VariantOption[]>('instance_variants', { instanceId }).then(options => {
      // Nothing to choose between: install the one variant that exists.
      if (options.length === 0) {
        setVariantDialog(null);
        void act('begin_install', { requestId: crypto.randomUUID() });
        return;
      }
      setVariantDialog(current => current && { ...current, options });
    }).catch(cause => setVariantDialog(current => current && { ...current, error: messageForError(cause) }));
  }

  function confirmVariant(variant: Variant) {
    setVariantDialog(null);
    void act('begin_install', { requestId: crypto.randomUUID(), variant });
  }

  // An installed variant the instance withdrew moves to Standard without a question: it is
  // not a version change, and nothing the player owns is touched. One attempt per variant,
  // so a failure waits for the player instead of retrying in a loop.
  const switchKey = automaticSwitch ? `${automaticSwitch.pin}:${automaticSwitch.installed}` : null;
  useEffect(() => {
    if (!automaticSwitch || !switchKey || active || busyRef.current || !isTauri() || !play?.snapshot || switched.current === switchKey) return;
    switched.current = switchKey;
    setSwitchNotice(automaticSwitch.installed);
    const update: UpdateArguments = { from_version: automaticSwitch.pin, to_version: automaticSwitch.pin, backup_worlds: false, from_variant: automaticSwitch.installed, to_variant: automaticSwitch.to };
    void act('begin_update', { requestId: crypto.randomUUID(), update });
  }, [switchKey, active, play?.snapshot]);

  async function cleanNatives() {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setCleanupError(null);
    toast.show('Eliminando las carpetas nativas antiguas…', 'progress');
    try {
      const report = await invoke<NativeCleanupReport>('clean_native_cache', { instanceId });
      if (report.retainedGenerations > 0) {
        setNativeReport(report);
        toast.show(nativeCleanupSummary(report), 'error', { label: 'Ver detalles', onClick: () => setDetails('natives') });
      } else toast.show('Caché nativa limpia.');
    } catch (cause) {
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
    if (updateTarget.pack) {
      // The catalog already names the versions of a custom pack, so the prompt shows them at once; the notes
      // (`changelog` undefined) and any name only the backend knows (a Modrinth version number) arrive with its
      // answer, which waits on the network. A failure or the browser preview leaves the names and no notes.
      const seeded: VersionNames = { installed: updateTarget.labels?.installed ?? null, available: updateTarget.labels?.available ?? null, changelog: undefined };
      setNames(seeded);
      if (isTauri()) {
        void invoke<VersionNames>('pack_version_names', { instanceId })
          .then(answer => setNames({ installed: answer.installed ?? seeded.installed, available: answer.available ?? seeded.available, changelog: answer.changelog ?? null }))
          .catch(() => setNames({ ...seeded, changelog: null }));
      } else setNames({ ...seeded, changelog: null });
    }
  }

  // Both prompts name the version being left and the one being installed; a pack uses readable names once they are known.
  const updateLabels = updateQuestion
    ? updateQuestion.pack ? versionLabels(names, updateQuestion.fromVersion, updateQuestion.toVersion) : { installed: updateQuestion.fromVersion, available: updateQuestion.toVersion }
    : null;

  return <div className="play-controls">
    <TransientNotice notice={toast.notice} />
    {logError && <p role="alert">{logError}</p>}
    {entry?.consoleError && <p role="alert">Minecraft se inició, pero no pudimos abrir su consola automáticamente. Puedes volver a intentarlo con «Ver registro». {messageForError(entry.consoleError)}</p>}
    {entry && (entry.operation === 'repair' || entry.operation === 'uninstall') && !active && entry.retainedFiles > 0 && <p role="status">Conservamos {entry.retainedFiles} archivos nativos modificados o que no pudimos comprobar.</p>}
    {switchNotice && <p role="status">{withdrawnVariantText(switchNotice)}</p>}
    {entry?.error && entry.error !== 'operation_cancelled' && !notice && <p className="account-error" role="alert">{messageForError(entry.error)}</p>}
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
    {active ? <Button className={`primary-play h-12 text-xl ${running && entry.phase !== 'stopping' ? 'border-danger bg-danger text-foreground hover:border-danger-hover hover:bg-danger-hover focus-visible:ring-danger-hover' : 'busy-play'}`} size="lg" aria-disabled={!running || entry.phase === 'stopping'} onClick={() => { if (running && entry && entry.phase !== 'stopping') setForceQuestion(entry.requestId); }}>
      {running && entry.phase !== 'stopping' ? <Square aria-hidden="true" /> : <LoaderCircle className="spin" aria-hidden="true" />}{running ? entry.phase === 'stopping' ? 'Cerrando…' : 'Cerrar forzosamente…' : 'Preparando…'}
    </Button> : updateTarget ? <Button className="primary-play h-12 text-xl" size="lg" disabled={busy} onClick={askUpdate}><RefreshCw aria-hidden="true" />Actualizar…</Button>
      : (installAllowed && entry?.phase !== 'installed') || entry?.phase === 'uninstalled' ? <Button className="primary-play h-12 text-xl" size="lg" disabled={busy || !installAllowed || !isTauri() || !play?.snapshot} onClick={() => choice ? openVariants() : void act('begin_install', { requestId: crypto.randomUUID() })}><Download aria-hidden="true" />{entry?.phase === 'failed' ? 'Reintentar instalación' : 'Instalar'}</Button>
      : launcherRequired ? <Button className="primary-play h-12 text-xl" size="lg" aria-disabled="true" title="Actualiza Panita Client para poder jugar. El botón de versión de la barra lateral inicia la actualización." onClick={() => launcherUpdate?.highlight()}><CircleArrowUp aria-hidden="true" />Actualización requerida</Button>
      : <Button className="primary-play h-12 text-xl" size="lg" disabled={busy || !allowed || !isTauri() || !play?.snapshot} onClick={() => void act('begin_play', { requestId: crypto.randomUUID() })}><Play aria-hidden="true" />{allowed ? 'Jugar' : 'No disponible para jugar'}</Button>}
    {optionsVisible && <>
    <DropdownMenu modal={false} open={optionsEnabled && optionsOpen} onOpenChange={open => setOptionsOpen(optionsEnabled && open)}>
      <DropdownMenuTrigger asChild><Button className={`split-trigger h-12 w-12 rounded-l-none p-0 ${running && entry?.phase !== 'stopping' ? 'border-danger bg-danger text-foreground hover:border-danger-hover hover:bg-danger-hover focus-visible:ring-danger-hover' : ''}`} disabled={!optionsEnabled} aria-label="Opciones de instancia" title={optionsEnabled ? 'Opciones de instancia' : 'Opciones no disponibles en este momento'}><ChevronDown aria-hidden="true" /></Button></DropdownMenuTrigger>
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
    {cleanupError && <p role="alert">{cleanupError}</p>}
    {details === 'conflicts' && entry?.packConflicts && notice?.details === 'conflicts' && <ConflictsDialog conflicts={entry.packConflicts} operation={entry.operation}
      resolvable={conflictsResolvable(entry.packConflicts)} canContinue={entry.operation === 'update' && !!retry && conflictsResolvable(entry.packConflicts)}
      busy={busy || !isTauri()} onClose={() => setDetails(null)}
      onContinue={() => { if (!retry) return; setDetails(null); void act('begin_update', { requestId: crypto.randomUUID(), update: { ...retry, move_aside: true } }); }} />}
    {details === 'manual' && entry?.packAwaiting && notice?.details === 'manual' && <ManualFilesDialog files={entry.packAwaiting} busy={busy || !isTauri()} onClose={() => setDetails(null)}
      onOpenSource={file => void invoke('open_curseforge_project', { projectId: file.projectId }).catch(cause => setLogError(messageForError(cause)))}
      onOpenFolder={file => void invoke('open_instance_folder', { instanceId, folder: file.folder }).catch(cause => setLogError(messageForError(cause)))} />}
    {details === 'natives' && nativeReport && <DetailsDialog title="Carpetas nativas antiguas" onClose={() => setDetails(null)}><NativeCleanupSection report={nativeReport} /></DetailsDialog>}
    {variantDialog && <VariantDialog variants={variants} options={variantDialog.options} error={variantDialog.error} busy={busy} onConfirm={confirmVariant} onCancel={() => setVariantDialog(null)} />}
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
    <dialog ref={updateDialog} className="update-dialog update-modal" aria-labelledby={instanceId + '-update-title'} onCancel={() => setUpdateQuestion(null)} onClose={() => { if (!updateDialog.current?.open) setUpdateQuestion(null); }}>
    {updateQuestion && updateLabels && <>
      <div className="update-head">
        <h2 id={instanceId + '-update-title'} className="dialog-title"><CircleArrowUp aria-hidden="true" />{updateQuestion.pack ? 'Actualizar modpack' : 'Actualizar Minecraft'}</h2>
        <span className="update-versions"><span className="version-chip">{updateLabels.installed}</span><ArrowRight aria-hidden="true" /><span className="sr-only">a</span><span className="version-chip is-new">{updateLabels.available}</span></span>
        <button type="button" className="update-close" aria-label="Cerrar" onClick={() => setUpdateQuestion(null)}><X aria-hidden="true" /></button>
      </div>
      {updateQuestion.pack && <div className="update-card">
        <h3 id={instanceId + '-notes-title'}><List aria-hidden="true" />Notas de la versión</h3>
        {names?.changelog === undefined ? <p className="update-card-empty" role="status">Buscando las novedades…</p>
          : names?.changelog ? <pre className="changelog" tabIndex={0} aria-labelledby={instanceId + '-notes-title'}>{names?.changelog}</pre>
          : <p className="update-card-empty">El autor no ha publicado notas para esta versión.</p>}
      </div>}
      <ul className="update-checks">
        <li><Check aria-hidden="true" />{updateQuestion.pack ? 'Solo se reemplazan los archivos del modpack que no hayas modificado.' : `Se instalan los archivos de Minecraft ${updateLabels.available}.`}</li>
        <li><Check aria-hidden="true" />{updateQuestion.pack ? 'Tus mundos, mods añadidos y ajustes personalizados se conservan.' : 'Tus mundos y ajustes se conservan.'}</li>
      </ul>
      {updateQuestion.pack && withdrawnVariant && <p className="update-callout"><TriangleAlert aria-hidden="true" /><span>{withdrawnVariantText(withdrawnVariant)}</span></p>}
      <div className="update-backup">
        <label htmlFor={instanceId + '-update-backup'}><span>Copia de seguridad de mis mundos</span><span className="update-chip">Recomendado</span></label>
        <input id={instanceId + '-update-backup'} className="update-switch" role="switch" type="checkbox" checked={updateQuestion.backupWorlds} onChange={event => setUpdateQuestion({ ...updateQuestion, backupWorlds: event.target.checked })} />
      </div>
      <div className="update-foot">
        <Button variant="outline" disabled={busy} onClick={() => setUpdateQuestion(null)}>Cancelar</Button>
        <Button disabled={busy || !isTauri() || !play?.snapshot} onClick={() => {
          const choice = updateQuestion;
          setUpdateQuestion(null);
          setLastBackup(choice.backupWorlds);
          void act('begin_update', { requestId: crypto.randomUUID(), update: { from_version: choice.fromVersion, to_version: choice.toVersion, backup_worlds: choice.backupWorlds } });
        }}><RefreshCw aria-hidden="true" />Actualizar a {updateLabels.available}</Button>
      </div>
    </>}
    </dialog>
  </div>;
}
