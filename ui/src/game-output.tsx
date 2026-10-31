import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { usePlaySnapshot } from './play';
import { messageForError } from './errors';
import { ArrowDownToLine, ChevronUp, ChevronDown, FileText, FolderOpen, RotateCcw, Search, Trash2, TriangleAlert, X } from 'lucide-react';
import { Button } from './components/ui/button';
import { unacknowledgedCrashes, type GameLogSnapshot } from './game-output-state';
import { formattedGameLines, displayLogTime, readableGameLines } from './game-log-format';
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
  return <dialog ref={dialog} className="crash-dialog" aria-labelledby="crash-title" onClose={() => { if (key) setDismissed(previous => new Set([...previous, key])); }}>
    <h2 id="crash-title"><TriangleAlert aria-hidden="true" />Minecraft se cerró de forma inesperada</h2>
    {crash && <p>{names[crash.instanceId] ?? 'La instancia de Minecraft'} terminó con un error.{crash.exitCode !== null ? ` Código de salida: ${crash.exitCode}.` : ''}</p>}
    {summary && <details className="crash-details"><summary>Ver últimos mensajes</summary><pre className="crash-summary">{summary}</pre></details>}
    <p>Revisa el registro para conocer la causa. Puede que Minecraft no haya creado un informe.</p>
    {notice && <p role="status">{notice}</p>}
    <div className="account-buttons">
      <Button variant="outline" disabled={busy} onClick={() => void open('open_crash_reports')}><FolderOpen aria-hidden="true" />Informes de fallos</Button>
      <Button disabled={busy} onClick={() => void open('open_game_log')}><FileText aria-hidden="true" />Ver registro</Button>
      <Button variant="ghost" onClick={() => dialog.current?.close()}><X aria-hidden="true" />Cerrar</Button>
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
  const anchor = useRef<{ sequence: string; top: number } | null>(null);
  const scrollingRead = useRef(false);
  const [levels, setLevels] = useState(new Set(['normal', 'info', 'warn', 'error', 'fatal', 'notice']));
  useEffect(() => () => { searchGeneration.current++; }, [instanceId, requestId]);
  useEffect(() => startGameLogFeed({
    offset, follow,
    load: cursor => invoke<GameLogSnapshot>('game_log_snapshot', { instanceId, requestId, offset: cursor }),
    subscribe: changed => listen<GameLogNotice>('game-log-changed', event => {
      const notice = event.payload;
      if (notice.instanceId === instanceId && notice.requestId === requestId) changed(notice);
    }),
    receive: next => { setSnapshot(next); setError(null); scrollingRead.current = false; },
    fail: cause => { setError(messageForError(cause)); scrollingRead.current = false; },
  }), [instanceId, requestId, offset, follow, retry]);
  const lines = useMemo(() => formattedGameLines((snapshot?.lines ?? []).filter(line => (line.sequence ?? 0) >= viewStart)).filter(line => {
    const level = line.metadata?.level?.toLowerCase() ?? (line.stream === 'stderr' ? 'error' : line.stream === 'notice' ? 'notice' : 'normal');
    return levels.has(level) && (!query.trim() || line.text.toLocaleLowerCase().includes(query.toLocaleLowerCase()));
  }), [snapshot?.lines, viewStart, levels, query]);
  useLayoutEffect(() => {
    const saved = anchor.current;
    if (saved && list.current) {
      const row = list.current.querySelector<HTMLElement>('[data-sequence="' + saved.sequence + '"]');
      if (row) list.current.scrollTop += row.getBoundingClientRect().top - list.current.getBoundingClientRect().top - saved.top;
      anchor.current = null;
    }
  }, [snapshot]);
  const selected = match === null ? undefined : lines.find(line => (line.sequence ?? -1) >= match)?.sequence;
  useEffect(() => {
    if (follow) end.current?.scrollIntoView({ block: 'end' });
    else if (selected !== undefined) list.current?.querySelector('[data-sequence="' + selected + '"]')?.scrollIntoView({ block: 'center' });
  }, [snapshot, follow, selected]);

  function navigate(cursor: number | null) {
    anchor.current = null; scrollingRead.current = false;
    setFollow(cursor === null); setOffset(cursor);
    list.current?.scrollTo({ top: 0 });
  }
  function clearView() {
    anchor.current = null; scrollingRead.current = false;
    searchGeneration.current++; setSearching(false); setMatch(null); setSearchNotice(null);
    const start = snapshot?.total ?? 0;
    setViewStart(start); setSnapshot(null); setOffset(start); setFollow(true); setRetry(value => value + 1);
  }
  function scrollWindow(backwards: boolean) {
    if (!snapshot || !list.current || scrollingRead.current) return;
    const step = Math.max(1, Math.floor(snapshot.pageSize / 2));
    const cursor = backwards ? Math.max(viewStart, snapshot.offset - step) : snapshot.offset + step;
    if (backwards ? snapshot.offset <= viewStart : snapshot.nextOffset >= snapshot.total) return;
    const bounds = list.current.getBoundingClientRect();
    const visible = [...list.current.querySelectorAll<HTMLElement>('[data-sequence]')].find(row => row.getBoundingClientRect().bottom > bounds.top);
    anchor.current = visible ? { sequence: visible.dataset.sequence!, top: visible.getBoundingClientRect().top - bounds.top } : null;
    scrollingRead.current = true; setFollow(false); setOffset(cursor);
  }
  function onLogScroll() {
    const element = list.current;
    if (!element || !snapshot) return;
    const bottom = element.scrollHeight - element.clientHeight - element.scrollTop;
    if (follow && bottom > 60) { setFollow(false); setOffset(snapshot.offset); }
    if (!follow && element.scrollTop < 60) scrollWindow(true);
    else if (!follow && bottom < 60) scrollWindow(false);
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
  return <main className="game-log-window">
    <header className="log-toolbar">
      <div className="log-filter" role="search" aria-label="Buscar en el historial de Minecraft"><label htmlFor="log-query"><Search aria-hidden="true" />Filtrar mensajes</label><div className="log-search-input"><input id="log-query" placeholder="Buscar en el registro…" value={query} maxLength={256} onChange={event => { searchGeneration.current++; setSearching(false); setQuery(event.target.value); setMatch(null); setSearchNotice(null); }} onKeyDown={event => { if (event.key === 'Enter') { event.preventDefault(); void find(event.shiftKey); } }} /><Button variant="ghost" size="icon" aria-label="Coincidencia anterior" disabled={!query.trim() || searching} onClick={() => void find(true)}><ChevronUp aria-hidden="true" /></Button><Button variant="ghost" size="icon" aria-label="Coincidencia siguiente" disabled={!query.trim() || searching} onClick={() => void find(false)}><ChevronDown aria-hidden="true" /></Button></div></div>
      <fieldset className="log-levels"><legend>Niveles</legend>{['debug', 'info', 'warn', 'error', 'fatal'].map(level => <label key={level}><input type="checkbox" checked={levels.has(level)} onChange={event => setLevels(previous => { const next = new Set(previous); if (event.target.checked) next.add(level); else next.delete(level); return next; })} />{level.toUpperCase()}</label>)}</fieldset>
      <div className="log-actions"><Button variant="outline" disabled={!snapshot} onClick={clearView}><Trash2 aria-hidden="true" />Limpiar vista</Button>{viewStart > 0 && <Button variant="ghost" aria-label="Restaurar historial" onClick={() => { setViewStart(0); navigate(null); }}><RotateCcw aria-hidden="true" /></Button>}</div>
    </header>
    <div className="log-options"><label><input type="checkbox" checked={follow} onChange={event => { setFollow(event.target.checked); setOffset(event.target.checked ? null : snapshot?.offset ?? viewStart); }} />Seguir entradas nuevas</label><label><input type="checkbox" checked={wrap} onChange={event => setWrap(event.target.checked)} />Ajustar líneas</label><label><input type="checkbox" checked={colors} onChange={event => setColors(event.target.checked)} />Colores</label><span>{snapshot?.running ? 'Minecraft en ejecución' : 'Registro de Minecraft'}</span><Button variant="ghost" size="icon" aria-label="Ir al final" onClick={() => navigate(null)}><ArrowDownToLine aria-hidden="true" /></Button></div>
    {searching && <div className="log-feedback" role="status">Buscando en el historial… <button onClick={() => { searchGeneration.current++; setSearching(false); }}>Cancelar búsqueda</button></div>}
    {searchNotice && <div className="log-feedback" role="status">{searchNotice}</div>}
    {error && <div className="log-feedback" role="alert">{error}<Button variant="outline" onClick={() => setRetry(value => value + 1)}>Reintentar</Button></div>}
    <div ref={list} onScroll={onLogScroll} onWheel={event => { if (event.deltaY < 0 && (list.current?.scrollTop ?? 0) < 60) scrollWindow(true); else if (event.deltaY > 0 && list.current && list.current.scrollTop + list.current.clientHeight >= list.current.scrollHeight - 60 && !follow) scrollWindow(false); }} className={'game-log-lines ' + (wrap ? 'log-wrap ' : 'log-nowrap ') + (colors ? 'log-colored' : '')} tabIndex={0} aria-label="Entradas del registro de Minecraft">
      {lines.length === 0 && <p className="log-empty">{snapshot ? 'Sin mensajes para mostrar.' : 'Cargando registro…'}</p>}
      {lines.map((line, index) => {
        const level = line.metadata?.level?.toLowerCase() ?? (line.stream === 'stderr' ? 'error' : 'normal');
        return <div key={line.sequence ?? index} data-sequence={line.sequence} className={'log-row log-' + level + (line.sequence === selected ? ' log-match' : '')}>
          <span className="log-time">{displayLogTime(line.metadata?.timestamp ?? null)}<small>{line.metadata?.level ?? line.stream}</small></span><span className="log-logger" title={line.metadata?.logger ?? ''}>{line.metadata?.logger ?? line.stream}</span><span className="log-thread" title={line.metadata?.thread ?? ''}>{line.metadata?.thread}</span><pre>{line.text}</pre>
        </div>;
      })}<div ref={end} />
    </div>
  </main>;
}
