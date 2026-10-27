import { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { usePlaySnapshot } from './play';
import { messageForError } from './errors';
import { previousLogPage, unacknowledgedCrashes, type GameLogSnapshot } from './game-output-state';
import { formattedGameLines, logPrefix, readableGameLines } from './game-log-format';
import { startGameLogFeed, type GameLogNotice } from './game-log-feed';

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
  const [viewStart, setViewStart] = useState(0);
  const [wrap, setWrap] = useState(true);
  const [colors, setColors] = useState(true);
  const [query, setQuery] = useState('');
  const [searching, setSearching] = useState(false);
  const [searchNotice, setSearchNotice] = useState<string | null>(null);
  const [match, setMatch] = useState<number | null>(null);
  const [retry, setRetry] = useState(0);
  const searchGeneration = useRef(0);
  const list = useRef<HTMLDivElement>(null);
  const end = useRef<HTMLDivElement>(null);
  useEffect(() => () => { searchGeneration.current++; }, [instanceId, requestId]);
  useEffect(() => startGameLogFeed({
    offset, follow,
    load: cursor => invoke<GameLogSnapshot>('game_log_snapshot', { instanceId, requestId, offset: cursor }),
    subscribe: changed => listen<GameLogNotice>('game-log-changed', event => {
      const notice = event.payload;
      if (notice.instanceId === instanceId && notice.requestId === requestId) changed(notice);
    }),
    receive: next => { setSnapshot(next); setError(null); },
    fail: cause => setError(messageForError(cause)),
  }), [instanceId, requestId, offset, follow, retry]);
  const lines = useMemo(() => formattedGameLines((snapshot?.lines ?? []).filter(line => (line.sequence ?? 0) >= viewStart)), [snapshot?.lines, viewStart]);
  const selected = match === null ? undefined : lines.find(line => (line.sequence ?? -1) >= match)?.sequence;
  useEffect(() => {
    if (follow) end.current?.scrollIntoView({ block: 'end' });
    else if (selected !== undefined) list.current?.querySelector('[data-sequence="' + selected + '"]')?.scrollIntoView({ block: 'center' });
  }, [snapshot, follow, selected]);

  function navigate(cursor: number | null) {
    setFollow(cursor === null); setOffset(cursor);
    list.current?.scrollTo({ top: 0 });
  }
  function clearView() {
    searchGeneration.current++; setSearching(false); setMatch(null); setSearchNotice(null);
    const start = snapshot?.total ?? 0;
    setViewStart(start); setSnapshot(null); setOffset(start); setFollow(true); setRetry(value => value + 1);
  }
  async function find(backwards: boolean) {
    if (!query.trim() || searching) return;
    const generation = ++searchGeneration.current;
    setSearching(true); setSearchNotice(null);
    let start = match === null ? (backwards ? snapshot?.total ?? 0 : 0) : backwards ? match : match + 1;
    try {
      while (generation === searchGeneration.current) {
        const result = await invoke<{ found: number | null; next: number; total: number }>('find_game_log', {
          instanceId, requestId, query, start, backwards,
        });
        if (generation !== searchGeneration.current) return;
        if (result.found !== null) {
          setMatch(result.found); setViewStart(0); navigate(result.found); return;
        }
        if ((backwards && result.next === 0) || (!backwards && result.next >= result.total)) {
          setSearchNotice('No hay más coincidencias en esa dirección.'); return;
        }
        if (result.next === start) throw new Error('Console search did not advance');
        start = result.next;
        await new Promise(resolve => setTimeout(resolve, 0));
      }
    } catch (cause) { if (generation === searchGeneration.current) setSearchNotice(messageForError(cause)); }
    finally { if (generation === searchGeneration.current) setSearching(false); }
  }
  return <main><section className="game-log-section">
    <h1>Registro de Minecraft</h1>
    <p>Historial sin límite de entradas, solo en memoria. Las entradas nuevas llegan por eventos; cada lectura y página son limitadas para mantener la ventana utilizable.</p>
    <p>Se ocultan datos sensibles y se omiten líneas demasiado largas. No se guarda ni exporta este registro. Las sesiones largas consumen más memoria; el historial termina al cerrar el cliente o reemplazar esta ejecución.</p>
    <p>Instancia: {instanceId}</p>
    <div className="game-log-tools">
      <label><input type="checkbox" checked={follow} onChange={event => {
        setFollow(event.target.checked); setOffset(event.target.checked ? null : snapshot?.offset ?? viewStart);
      }} /> Seguir entradas nuevas</label>
      <label><input type="checkbox" checked={wrap} onChange={event => setWrap(event.target.checked)} /> Ajustar líneas</label>
      <label><input type="checkbox" checked={colors} onChange={event => setColors(event.target.checked)} /> Colorear niveles</label>
      <button type="button" disabled={!snapshot} onClick={clearView}>Limpiar vista</button>
      <button type="button" disabled={viewStart === 0} onClick={() => { setViewStart(0); navigate(null); }}>Restaurar historial</button>
    </div>
    <p>Limpiar la vista conserva el historial en memoria y permite restaurarlo.</p>
    <div className="game-log-tools" role="search" aria-label="Buscar en el historial de Minecraft">
      <label>Buscar en todo el historial <input value={query} maxLength={256} onChange={event => {
        searchGeneration.current++; setSearching(false); setQuery(event.target.value); setMatch(null); setSearchNotice(null);
      }} onKeyDown={event => { if (event.key === 'Enter') { event.preventDefault(); void find(event.shiftKey); } }} /></label>
      <button type="button" disabled={!query.trim() || searching} onClick={() => void find(true)}>Coincidencia anterior</button>
      <button type="button" disabled={!query.trim() || searching} onClick={() => void find(false)}>Coincidencia siguiente</button>
      {searching && <button type="button" onClick={() => { searchGeneration.current++; setSearching(false); }}>Cancelar búsqueda</button>}
    </div>
    {searching && <p role="status">Buscando en el historial conservado…</p>}
    {searchNotice && <p role="status">{searchNotice}</p>}
    {error && <p role="alert">{error} <button type="button" onClick={() => setRetry(value => value + 1)}>Reintentar lectura</button></p>}
    {!snapshot && !error && <p role="status">Cargando registro…</p>}
    {snapshot && <>
      <p role="status">{snapshot.running ? 'Minecraft está en ejecución.' : 'Minecraft se cerró.' + (snapshot.exitCode !== null ? ' Código de salida: ' + snapshot.exitCode + '.' : '')}</p>
      <nav aria-label="Páginas del registro de Minecraft">
        <button type="button" disabled={snapshot.offset <= viewStart} onClick={() => { setMatch(null); navigate(viewStart); }}>Inicio</button>
        <button type="button" disabled={snapshot.offset <= viewStart} onClick={() => { setMatch(null); navigate(Math.max(viewStart, previousLogPage(snapshot))); }}>Anterior</button>
        <button type="button" disabled={snapshot.nextOffset >= snapshot.total} onClick={() => { setMatch(null); navigate(snapshot.nextOffset); }}>Siguiente</button>
        <button type="button" onClick={() => { setMatch(null); navigate(null); }}>Últimas entradas</button>
      </nav>
      <p>{snapshot.nextOffset > snapshot.offset ? 'Eventos ' + (snapshot.offset + 1) + '–' + snapshot.nextOffset + ' de ' + snapshot.total : 'Sin entradas en esta página. Total registrado: ' + snapshot.total}. El tamaño de página no limita el historial.</p>
      <div ref={list} className={'game-log-lines ' + (wrap ? 'log-wrap ' : 'log-nowrap ') + (colors ? 'log-colored' : '')} tabIndex={0} aria-label="Entradas del registro de Minecraft">
        {lines.length === 0 && <p>Todavía no hay mensajes visibles en esta página.</p>}
        {lines.map((line, index) => {
          const level = line.metadata?.level?.toLowerCase() ?? (line.stream === 'stderr' ? 'error' : 'normal');
          const tone = ['fatal', 'error', 'warn', 'info', 'debug', 'trace'].includes(level) ? level : 'normal';
          return <pre key={line.sequence ?? index} data-sequence={line.sequence} className={'log-' + tone + (line.sequence === selected ? ' log-match' : '')}>
            <span className="log-metadata">{logPrefix(line.metadata) || '[' + line.stream + ']'} </span>{line.text}
          </pre>;
        })}
        <div ref={end} />
      </div>
    </>}
  </section></main>;
}
