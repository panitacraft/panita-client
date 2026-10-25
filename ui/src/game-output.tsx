import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { usePlaySnapshot } from './play';
import { messageForError } from './errors';
import { previousLogPage, readableGameLines, unacknowledgedCrashes, type GameLogSnapshot } from './game-output-state';

export function CrashNotifications({ names }: { names: Record<string, string> }) {
  const snapshot = usePlaySnapshot();
  const [dismissed, setDismissed] = useState(new Set<string>());
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [summary, setSummary] = useState<string | null>(null);
  const dialog = useRef<HTMLDialogElement>(null);
  const crash = unacknowledgedCrashes(snapshot, dismissed)[0];
  const key = crash?.key;
  useEffect(() => {
    let active = true;
    setNotice(null);
    setSummary(null);
    if (key && !dialog.current?.open) dialog.current?.showModal();
    if (!key) dialog.current?.close();
    if (crash) void invoke<GameLogSnapshot>('game_log_snapshot', { instanceId: crash.instanceId, requestId: crash.requestId }).then(output => {
      const readable = readableGameLines(output.lines);
      const errors = readable.filter(line => line.stream === 'stderr');
      const lines = errors.length ? errors : readable;
      if (active) setSummary(lines.slice(-3).map(line => line.text).join('\n').slice(-4096) || null);
    }).catch(() => { /* The dialog remains usable without a console summary. */ });
    return () => { active = false; };
  }, [key]);
  async function open(command: 'open_crash_reports' | 'open_game_log') {
    if (!crash || busy) return;
    setBusy(true); setNotice(null);
    try {
      const result = await invoke<boolean | 'pending' | void>(command, { instanceId: crash.instanceId, requestId: crash.requestId });
      if (result === false) setNotice('Minecraft no creó una carpeta de informes de fallos. Puedes revisar el registro del juego.');
      if (result === 'pending') setNotice('Solicitamos abrir la carpeta de informes. El explorador está tardando en responder; puedes seguir usando Panita.');
    } catch (error) { setNotice(messageForError(error)); }
    finally { setBusy(false); }
  }
  return <dialog ref={dialog} aria-labelledby="crash-title" onClose={() => { if (key) setDismissed(previous => new Set([...previous, key])); }}>
    <h2 id="crash-title">Minecraft se cerró de forma inesperada</h2>
    {crash && <p>{names[crash.instanceId] ?? 'La instancia de Minecraft'} terminó con un error.{crash.exitCode !== null ? ` Código de salida: ${crash.exitCode}.` : ''}</p>}
    {summary && <><p>Última salida del juego:</p><pre className="crash-summary">{summary}</pre></>}
    <p>Revisa el registro o los informes de fallos para obtener más información. Algunos errores no generan un informe.</p>
    {notice && <p role="status">{notice}</p>}
    <div className="account-buttons">
      <button type="button" disabled={busy} onClick={() => void open('open_crash_reports')}>Abrir carpeta de informes</button>
      <button type="button" disabled={busy} onClick={() => void open('open_game_log')}>Ver registro de Minecraft</button>
      <button type="button" onClick={() => dialog.current?.close()}>Cerrar</button>
    </div>
  </dialog>;
}

export function GameLogWindow({ instanceId, requestId }: { instanceId: string; requestId: string }) {
  const [snapshot, setSnapshot] = useState<GameLogSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [follow, setFollow] = useState(true);
  const [offset, setOffset] = useState<number | null>(null);
  const end = useRef<HTMLDivElement>(null);
  useEffect(() => {
    let active = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    async function refresh() {
      try {
        const next = await invoke<GameLogSnapshot>('game_log_snapshot', { instanceId, requestId, offset });
        if (active) { setSnapshot(next); setError(null); }
        if (active && next.running) timer = setTimeout(() => void refresh(), 1000);
      } catch (cause) { if (active) setError(messageForError(cause)); }
    }
    void refresh();
    return () => { active = false; clearTimeout(timer); };
  }, [instanceId, requestId, offset]);
  useEffect(() => { if (follow) end.current?.scrollIntoView({ block: 'end' }); }, [snapshot, follow]);
  return <main><section className="game-log-section">
    <h1>Registro de Minecraft</h1>
    <p>Historial de esta ejecución sin límite de entradas, conservado solo en memoria. Se muestra por páginas sin descartar entradas anteriores por cantidad. Se ocultan datos sensibles y se omiten líneas demasiado largas. Este visor no guarda ni exporta el registro.</p>
    <p>Las sesiones largas consumen más memoria. El historial deja de estar disponible al cerrar el cliente o reemplazar esta ejecución por otra.</p>
    <p>Instancia: {instanceId}</p>
    <label><input type="checkbox" checked={follow} onChange={event => {
      setFollow(event.target.checked);
      setOffset(event.target.checked ? null : snapshot?.offset ?? 0);
    }} /> Seguir las entradas nuevas</label>
    {error && <p role="alert">{error} Cierra esta ventana y abre el registro desde la ejecución actual.</p>}
    {!snapshot && !error && <p role="status">Cargando registro…</p>}
    {snapshot && <>
      <p role="status">{snapshot.running ? 'Minecraft está en ejecución.' : `Minecraft se cerró.${snapshot.exitCode !== null ? ` Código de salida: ${snapshot.exitCode}.` : ''}`}</p>
      <nav aria-label="Páginas del registro de Minecraft">
        <button type="button" disabled={snapshot.offset === 0} onClick={() => { setFollow(false); setOffset(0); }}>Inicio</button>
        <button type="button" disabled={snapshot.offset === 0} onClick={() => { setFollow(false); setOffset(previousLogPage(snapshot)); }}>Anterior</button>
        <button type="button" disabled={snapshot.nextOffset >= snapshot.total} onClick={() => { setFollow(false); setOffset(snapshot.nextOffset); }}>Siguiente</button>
        <button type="button" onClick={() => { setFollow(true); setOffset(null); }}>Últimas entradas</button>
      </nav>
      <p>Eventos {snapshot.total === 0 ? 0 : snapshot.offset + 1}–{snapshot.nextOffset} de {snapshot.total}. El tamaño de página no limita el historial conservado.</p>
      <div className="game-log-lines" tabIndex={0} aria-label="Entradas del registro de Minecraft">
        {snapshot.lines.length === 0 && <p>Todavía no hay entradas disponibles.</p>}
        {readableGameLines(snapshot.lines).map((line, index) => <pre key={index}>[{line.stream}] {line.text}</pre>)}
        <div ref={end} />
      </div>
    </>}
  </section></main>;
}
