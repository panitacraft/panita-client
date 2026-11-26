import { useEffect, useState } from 'react';
import { CircleAlert, CircleCheck, RefreshCw, Activity, FolderOpen, Trash2 } from 'lucide-react';
import { isTauri } from '@tauri-apps/api/core';
import { clearOldLogs, loadDiagnostics, openLogsFolder, type ClearedLogs } from './diagnostic-api';
import { diagnosticText, visibleDiagnosticRecords, type DiagnosticSnapshot } from './diagnostic-state';
import { messageForError } from './errors';

function formatTime(at: number): string {
  return new Date(at).toLocaleString('es', { dateStyle: 'short', timeStyle: 'medium', hour12: false });
}

export function clearedLogsMessage({ removed, failed }: ClearedLogs): string {
  if (removed === 0 && failed === 0) return 'No hay registros anteriores que borrar.';
  const done = removed === 1 ? 'Se borró 1 registro anterior.' : `Se borraron ${removed} registros anteriores.`;
  if (failed === 0) return done;
  const left = failed === 1 ? 'No pudimos borrar 1 archivo; puede estar en uso.' : `No pudimos borrar ${failed} archivos; pueden estar en uso.`;
  return removed === 0 ? left : `${done} ${left}`;
}

function DiagnosticViewer() {
  const [snapshot, setSnapshot] = useState<DiagnosticSnapshot | null>(null);
  const [errorsOnly, setErrorsOnly] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState(false);
  const [folderError, setFolderError] = useState(false);
  const [confirmClear, setConfirmClear] = useState(false);
  const [clearing, setClearing] = useState(false);
  const [clearResult, setClearResult] = useState<{ text: string; failed: boolean } | null>(null);
  async function clearPrevious() {
    setClearing(true); setClearResult(null);
    try {
      const result = await clearOldLogs();
      setClearResult({ text: clearedLogsMessage(result), failed: result.failed > 0 });
    } catch { setClearResult({ text: 'No pudimos borrar los registros anteriores.', failed: true }); }
    finally { setClearing(false); setConfirmClear(false); }
  }
  useEffect(() => {
    let active = true;
    setBusy(true);
    setError(false);
    loadDiagnostics().then(next => { if (active) setSnapshot(next); })
      .catch(() => { if (active) setError(true); })
      .finally(() => { if (active) setBusy(false); });
    return () => { active = false; };
  }, [attempt]);
  const records = snapshot ? visibleDiagnosticRecords(snapshot) : [];
  const hasProblem = (record: typeof records[number]) => !!record.error || record.phase === 'failed' || record.phase === 'exited_with_error';
  const visible = errorsOnly ? records.filter(hasProblem) : records;
  return <div aria-busy={busy}>
    <p>Actividad de esta sesión, del evento más reciente al más antiguo. Se conserva hasta cerrar el cliente.</p>
    <p>Cada sesión también se guarda en un archivo de texto para soporte. La sesión actual es latest.log; al abrir otra sesión recibe su propio nombre. Los archivos se eliminan solos después de 48 horas y no incluyen contraseñas ni tokens.</p>
    {folderError && <p role="alert">No pudimos abrir la carpeta de registros.</p>}
    <div className="diagnostic-toolbar"><span><Activity aria-hidden="true" />{snapshot?.records.length ?? 0} {snapshot?.records.length === 1 ? 'evento' : 'eventos'}</span><label><input type="checkbox" checked={errorsOnly} onChange={event => setErrorsOnly(event.target.checked)} />Solo problemas</label>
    <div className="diagnostic-actions"><button type="button" disabled={!isTauri()} onClick={() => { setFolderError(false); openLogsFolder().catch(() => setFolderError(true)); }}><FolderOpen aria-hidden="true" />Abrir carpeta de registros</button>
    <button type="button" disabled={clearing || !isTauri()} onClick={() => { setClearResult(null); setConfirmClear(true); }}><Trash2 aria-hidden="true" />Borrar registros anteriores</button>
    <button type="button" disabled={busy || !isTauri()} onClick={() => setAttempt(value => value + 1)}><RefreshCw aria-hidden="true" />Actualizar registro</button></div></div>
    {confirmClear && <div role="group" aria-label="Confirmar borrado de registros anteriores" className="diagnostic-confirm">
      <p>Se borrarán los archivos de registro de sesiones anteriores. El registro de esta sesión (latest.log) se conserva. Esta acción no se puede deshacer.</p>
      <div className="diagnostic-actions"><button type="button" disabled={clearing} onClick={() => setConfirmClear(false)}>Cancelar</button>
      <button type="button" disabled={clearing} onClick={() => void clearPrevious()}><Trash2 aria-hidden="true" />Borrar</button></div>
    </div>}
    {clearResult && <p role={clearResult.failed ? 'alert' : 'status'}>{clearResult.text}</p>}
    {busy && <p role="status">Cargando registro…</p>}
    {error && <p role="alert">No pudimos actualizar el registro. Puedes reintentar; conservamos la última lectura.</p>}
    {snapshot && !snapshot.records.length && <p role="status">{isTauri() ? 'Todavía no hay eventos disponibles.' : 'El registro está disponible en la aplicación de escritorio.'}</p>}
    {snapshot && errorsOnly && !visible.length && <p role="status">No hay problemas registrados en esta sesión.</p>}
    {visible.length > 0 && <ol className="diagnostic-list" tabIndex={0} aria-label="Eventos recientes del cliente">
      {visible.map((record, index) => {
        const problem = hasProblem(record);
        return <li key={index} className={problem ? 'is-error' : ''}>
          <span className="timeline-dot" aria-hidden="true" />
          <div className="timeline-body">
            <p className="diagnostic-event"><span>{diagnosticText(record)}</span>{problem ? <CircleAlert aria-hidden="true" /> : <CircleCheck aria-hidden="true" />}</p>
            {record.error && <p>{messageForError(record.error)}</p>}
            <p className="timeline-meta">{record.at > 0 && <time dateTime={new Date(record.at).toISOString()}>Fecha {formatTime(record.at)}</time>}
              <span>Sesión {record.processSessionId}</span>
              {record.operationId && <span>Operación {record.operationId}</span>}
              {record.error && <span>Categoría <code>{record.error}</code></span>}</p>
          </div>
        </li>;
      })}
    </ol>}
  </div>;
}

export function DiagnosticsPanel() {
  return <section className="diagnostics-section" aria-labelledby="diagnostics-heading">
    <h2 id="diagnostics-heading">Registro del cliente</h2>
    <DiagnosticViewer />
  </section>;
}
