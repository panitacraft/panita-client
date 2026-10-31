import { useEffect, useState } from 'react';
import { CircleAlert, CircleCheck, RefreshCw, Activity } from 'lucide-react';
import { isTauri } from '@tauri-apps/api/core';
import { loadDiagnostics } from './diagnostic-api';
import { diagnosticText, visibleDiagnosticRecords, type DiagnosticSnapshot } from './diagnostic-state';
import { messageForError } from './errors';

function DiagnosticViewer() {
  const [snapshot, setSnapshot] = useState<DiagnosticSnapshot | null>(null);
  const [errorsOnly, setErrorsOnly] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState(false);
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
    <div className="diagnostic-toolbar"><span><Activity aria-hidden="true" />{snapshot?.records.length ?? 0} {snapshot?.records.length === 1 ? 'evento' : 'eventos'}</span><label><input type="checkbox" checked={errorsOnly} onChange={event => setErrorsOnly(event.target.checked)} />Solo problemas</label>
    <button type="button" disabled={busy || !isTauri()} onClick={() => setAttempt(value => value + 1)}><RefreshCw aria-hidden="true" />Actualizar registro</button></div>
    {busy && <p role="status">Cargando registro…</p>}
    {error && <p role="alert">No pudimos actualizar el registro. Puedes reintentar; conservamos la última lectura.</p>}
    {snapshot && !snapshot.records.length && <p role="status">{isTauri() ? 'Todavía no hay eventos disponibles.' : 'El registro está disponible en la aplicación de escritorio.'}</p>}
    {snapshot && errorsOnly && !visible.length && <p role="status">No hay problemas registrados en esta sesión.</p>}
    {visible.length > 0 && <ol className="diagnostic-list" tabIndex={0} aria-label="Eventos recientes del cliente">
      {visible.map((record, index) => <li key={index}>
        <p className={`diagnostic-event ${hasProblem(record) ? 'is-error' : ''}`}>{record.error || record.phase === 'failed' || record.phase === 'exited_with_error' ? <CircleAlert aria-hidden="true" /> : <CircleCheck aria-hidden="true" />}{diagnosticText(record)}</p>
        {record.error && <p>{messageForError(record.error)}</p>}
        <details><summary>Detalles para soporte</summary>
          <dl><dt>Sesión del cliente</dt><dd>{record.processSessionId}</dd>
            {record.operationId && <><dt>Operación</dt><dd>{record.operationId}</dd></>}
          </dl>
          {record.error && <p>Categoría: <code>{record.error}</code></p>}
        </details>
      </li>)}
    </ol>}
  </div>;
}

export function DiagnosticsPanel() {
  return <section className="diagnostics-section" aria-labelledby="diagnostics-heading">
    <h2 id="diagnostics-heading">Registro del cliente</h2>
    <DiagnosticViewer />
  </section>;
}
