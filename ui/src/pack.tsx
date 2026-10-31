import { useEffect, useRef, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { messageForError } from './errors';
import { packApi } from './pack-api';
import { playActive } from './play-state';
import { usePlaySnapshot } from './play';
import { AccountDialog } from './components/account-dialog';
import { Button } from './components/ui/button';
import { Database, Download, FolderOpen, ShieldCheck, Trash2, TriangleAlert, X } from 'lucide-react';
import { TransientNotice, useTransientNotice } from './components/transient-notice';
import {
  cacheRetainedReason, clearSummary, conflictReasonText, conflictSummary, folderHint, importErrorText, manualHeading,
  reasonText, skippedSummary, formatBytes, type CacheReport, type PackFile, type PackList,
} from './pack-state';

const packImportError = (cause: unknown) => importErrorText(cause) ?? messageForError(cause);

/** Progress, manual files and kept configuration for one modpack instance. */
export function PackPanel({ instanceId, awaiting, onChanged }: { instanceId: string; awaiting: boolean; onChanged: () => void }) {
  const entry = usePlaySnapshot()?.instances.find(candidate => candidate.instanceId === instanceId);
  const [files, setFiles] = useState<PackFile[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const installing = (entry?.operation === 'install' || entry?.operation === 'update' || entry?.operation === 'repair') && playActive(entry.phase);

  useEffect(() => {
    if (!awaiting || !isTauri()) { setFiles([]); return; }
    let active = true;
    packApi.files(instanceId).then(list => { if (active) setFiles(list.files); })
      .catch(cause => { if (active) setError(packImportError(cause)); });
    return () => { active = false; };
  }, [instanceId, awaiting, entry?.requestId, entry?.phase]);

  async function run(work: () => Promise<PackList | boolean>) {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setError(null);
    try {
      const result = await work();
      if (typeof result !== 'boolean' && !result.cancelled) {
        setFiles(result.files);
        if (result.complete || result.files.length !== files.length) onChanged();
      }
    } catch (cause) {
      setError(packImportError(cause));
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }

  const skipped = entry?.packSkipped ?? [];
  // An update that stopped on conflicts explains them in the update controls.
  const conflicts = entry?.operation === 'update' ? [] : entry?.packConflicts ?? [];
  if (!installing && !awaiting && skipped.length === 0 && conflicts.length === 0) return null;

  return <div className="pack-panel">
    {!installing && conflicts.length > 0 && <div className="catalog-notice" role="alert">
      <p>{conflictSummary(conflicts.map(conflict => conflict.path))}</p>
      <details><summary>Archivos en conflicto</summary><ul>{conflicts.map(conflict => <li key={conflict.path}><code>{conflict.path}</code>: {conflictReasonText(conflict.reason)}</li>)}</ul></details>
      <p>Muévelos o elimínalos y vuelve a intentar la instalación. Nunca los reemplazamos por nuestra cuenta.</p>
    </div>}
    {!installing && skipped.length > 0 && <div role="status">
      <p>{skippedSummary(skipped)}</p>
      <details><summary>Archivos conservados</summary><ul>{skipped.map(path => <li key={path}><code>{path}</code></li>)}</ul></details>
    </div>}
    {awaiting && !installing && <fieldset className="catalog-notice">
      <legend>{manualHeading(files.length)}</legend>
      <p>Sin ellos no puedes jugar. Los demás archivos ya están instalados.</p>
      <ul>
        {files.map(file => <li key={file.path}>
          <strong>{file.name}</strong> (<code>{file.fileName}</code>)
          <p>{reasonText(file.reason)} {folderHint(file)}</p>
          <div className="account-buttons">
            <button type="button" disabled={busy || !isTauri()} onClick={() => void run(() => packApi.openSource(instanceId, file.path))}>Abrir CurseForge</button>
            <button type="button" disabled={busy || !isTauri()} onClick={() => void run(() => packApi.openFolder(instanceId, file.path))}>Abrir carpeta</button>
            <button type="button" disabled={busy || !isTauri()} onClick={() => void run(() => packApi.importFile(instanceId, file.path))}>Seleccionar archivo…</button>
          </div>
        </li>)}
      </ul>
      <button type="button" disabled={busy || !isTauri()} onClick={() => void run(() => packApi.verify(instanceId))}>Verificar</button>
    </fieldset>}
    {error && <p className="account-error" role="alert">{error}</p>}
  </div>;
}

/** Size of the disposable download cache and the explicit action that clears it. */
export function CachePanel({ openRequest = 0 }: { openRequest?: number }) {
  const panel = useRef<HTMLElement>(null);
  useEffect(() => { if (openRequest) { panel.current?.scrollIntoView?.({ block: 'center' }); panel.current?.focus(); } }, [openRequest]);
  const [size, setSize] = useState<number | null>(null);
  const [question, setQuestion] = useState(false);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const [report, setReport] = useState<CacheReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const toast = useTransientNotice();

  async function refresh() {
    try { setSize(await packApi.cacheSize()); } catch (cause) { setError(packImportError(cause)); }
  }
  useEffect(() => { if (isTauri()) void refresh(); }, []);

  async function clear() {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true); setError(null); setQuestion(false); setReport(null);
    toast.show('Limpiando la caché de modpacks…', 'progress');
    try {
      const cleared = await packApi.clearCache();
      await refresh();
      // Partial results need their explanation on screen; a clean result is just confirmed.
      if (cleared.retainedEntries > 0) { toast.clear(); setReport(cleared); }
      else toast.show(`Caché de modpacks limpia (${formatBytes(cleared.removedBytes)} liberados).`);
    } catch (cause) {
      toast.clear();
      setError(packImportError(cause));
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }

  return <section ref={panel} tabIndex={-1} className="settings-section" aria-labelledby="cache-heading">
    <h2 id="cache-heading">Caché de modpacks</h2>
    <p>{size === null ? 'Calculando tamaño…' : `Tamaño actual: ${new Intl.NumberFormat('es', { maximumFractionDigits: 1 }).format(size / 1048576)} MB`}</p>
    <p>Guarda archivos compartidos para acelerar futuras descargas. Es completamente seguro vaciarla: tus instancias, mundos y ajustes no se verán afectados.</p>
    <button type="button" disabled={busy || !isTauri() || (size === 0 || size === null)} onClick={() => setQuestion(true)}>Limpiar caché</button>
    {question && <AccountDialog className="update-dialog" titleClassName="dialog-title" onCancel={() => setQuestion(false)}
      title={<><Database aria-hidden="true" />Limpiar la caché de modpacks</>}>
      <ul className="update-notes">
        <li><FolderOpen aria-hidden="true" />Eliminaremos la carpeta <code>content</code> de tus datos de aplicación ({size === null ? 'tamaño desconocido' : formatBytes(size)}).</li>
        <li><ShieldCheck aria-hidden="true" />No tocamos tus instancias, mundos, cuentas ni los archivos de Minecraft y Java.</li>
        <li><Download aria-hidden="true" />Los archivos se descargarán de nuevo la próxima vez que instales o repares un modpack.</li>
        <li className="is-warning"><TriangleAlert aria-hidden="true" />Esta acción no se puede deshacer.</li>
      </ul>
      <div className="account-buttons">
        <Button variant="destructive" disabled={busy} onClick={() => void clear()}><Trash2 aria-hidden="true" />Confirmar limpieza</Button>
        <Button variant="secondary" disabled={busy} onClick={() => setQuestion(false)}><X aria-hidden="true" />Volver</Button>
      </div>
    </AccountDialog>}
    {report && <div role="status">
      <p>{clearSummary(report)}</p>
      {report.details.length > 0 && <details><summary>Por qué se conservaron algunos archivos</summary>
        <ul>{report.details.map(detail => <li key={`${detail.directory}:${detail.reason}`}><code>{detail.directory}</code>: {cacheRetainedReason(detail.reason)}</li>)}</ul>
      </details>}
    </div>}
    {error && <p role="alert">{error}</p>}
    <TransientNotice notice={toast.notice} />
  </section>;
}
