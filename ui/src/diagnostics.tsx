import { useEffect, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { loadDiagnostics } from './diagnostic-api';
import { diagnosticText, visibleDiagnosticRecords, type DiagnosticSnapshot } from './diagnostic-state';
import { messageForError } from './errors';

function DiagnosticViewer() {
  const [snapshot, setSnapshot] = useState<DiagnosticSnapshot | null>(null);
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
  return <div aria-busy={busy}>
    <p>Se muestran hasta 256 eventos de esta ejecución, del más reciente al más antiguo. El registro se borra al cerrar el cliente.</p>
    <button type="button" disabled={busy || !isTauri()} onClick={() => setAttempt(value => value + 1)}>Actualizar registro</button>
    {busy && <p role="status">Cargando registro…</p>}
    {error && <p role="alert">No pudimos actualizar el registro. Puedes reintentar; conservamos la última lectura.</p>}
    {snapshot && !snapshot.records.length && <p role="status">{isTauri() ? 'Todavía no hay eventos disponibles.' : 'El registro está disponible en la aplicación de escritorio.'}</p>}
    {snapshot && snapshot.records.length > 0 && <ol className="diagnostic-list" tabIndex={0} aria-label="Eventos recientes del cliente">
      {visibleDiagnosticRecords(snapshot).map((record, index) => <li key={index}>
        <p>{diagnosticText(record)}</p>
        {record.error && <p>{messageForError(record.error)}</p>}
        <details><summary>Identificadores para soporte</summary>
          <dl><dt>Sesión del cliente</dt><dd>{record.processSessionId}</dd>
            {record.operationId && <><dt>Operación</dt><dd>{record.operationId}</dd></>}
          </dl>
          <p>La sesión agrupa los eventos de una ejecución del cliente; la operación relaciona los eventos de una misma acción. Al reportar un problema, comparte el evento, su error y estos identificadores. No son códigos de acceso ni permiten consultar un informe en línea.</p>
        </details>
      </li>)}
    </ol>}
  </div>;
}

export function DiagnosticsPanel() {
  const [open, setOpen] = useState(false);
  return <section className="diagnostics-section" aria-labelledby="diagnostics-heading">
    <h2 id="diagnostics-heading">Registro del cliente</h2>
    <details onToggle={event => setOpen(event.currentTarget.open)}>
      <summary>Ver eventos recientes</summary>
      {open && <DiagnosticViewer />}
    </details>
  </section>;
}
