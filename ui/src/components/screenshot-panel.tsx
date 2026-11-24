import { useEffect, useRef, useState, type MouseEvent } from 'react';
import { ChevronLeft, ChevronRight, Copy, Ellipsis, Expand, ExternalLink, FolderOpen, ImageOff, LoaderCircle, Trash2, X, type LucideIcon } from 'lucide-react';
import { AccountDialog } from './account-dialog';
import { ScreenshotViewer } from './screenshot-viewer';
import { TransientNotice, useTransientNotice } from './transient-notice';
import { Button } from './ui/button';
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger } from './ui/context-menu';
import { messageForError } from '../errors';
import { loadScreenshotImage, loadScreenshots, runScreenshotAction } from '../screenshot-api';
import {
  SCREENSHOT_REFRESH_MS, clampScreenshotPage, indexAfterRemoval, sameScreenshotList, screenshotActionError, screenshotChoices, screenshotCopied, screenshotDateText, screenshotPage, screenshotPageCount,
  screenshotPageOf, screenshotSizeText, screenshotTrashed,
  type Screenshot, type ScreenshotAction, type ScreenshotChoice, type ScreenshotList,
} from '../screenshot-state';

const choiceIcons: Record<ScreenshotChoice, LucideIcon> = { view: Expand, open: ExternalLink, reveal: FolderOpen, copy: Copy, delete: Trash2 };

/** One capture. Its image is fetched only once the card scrolls into view. */
function ScreenshotCard({ instanceId, screenshot, onChoice }: { instanceId: string; screenshot: Screenshot; onChoice: (choice: ScreenshotChoice, screenshot: Screenshot) => void }) {
  const frame = useRef<HTMLButtonElement>(null);
  const [visible, setVisible] = useState(typeof IntersectionObserver === 'undefined');
  const [source, setSource] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  // A choice that opens a dialog keeps focus away from the menu trigger, which would
  // otherwise take it back as the menu closes.
  const opensDialog = useRef(false);

  useEffect(() => {
    const element = frame.current;
    if (visible || !element) return;
    const observer = new IntersectionObserver(entries => { if (entries.some(entry => entry.isIntersecting)) setVisible(true); });
    observer.observe(element);
    return () => observer.disconnect();
  }, [visible]);

  useEffect(() => {
    if (!visible) return;
    let current = true;
    setSource(null); setFailed(false);
    loadScreenshotImage(instanceId, screenshot)
      .then(url => { if (current) setSource(url); })
      .catch(() => { if (current) setFailed(true); });
    return () => { current = false; };
  }, [visible, instanceId, screenshot.name, screenshot.modifiedMs]);

  /** The same menu as a right-click, for keyboard and touch: ask the card to show it. */
  function openMenu(event: MouseEvent<HTMLButtonElement>) {
    const box = event.currentTarget.getBoundingClientRect();
    event.currentTarget.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: box.left, clientY: box.bottom }));
  }

  const date = screenshotDateText(screenshot.modifiedMs);
  return <li className="screenshot-card">
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div className="screenshot-card-body">
          <button ref={frame} type="button" className="screenshot-frame" data-screenshot={screenshot.name} aria-label={`Ver ${screenshot.name}`} onClick={() => onChoice('view', screenshot)}>
            {source ? <img src={source} alt="" decoding="async" /> : failed
              ? <span className="screenshot-placeholder"><ImageOff aria-hidden="true" />No se pudo cargar la imagen</span>
              : <span className="screenshot-placeholder" role="status"><LoaderCircle className="spin" aria-hidden="true" /><span className="sr-only">Cargando la captura {screenshot.name}</span></span>}
          </button>
          <button type="button" className="screenshot-more" aria-label={`Más acciones para ${screenshot.name}`} onClick={openMenu}><Ellipsis aria-hidden="true" /></button>
          <div className="screenshot-meta">
            <strong title={screenshot.name}>{screenshot.name}</strong>
            <small>{[date, screenshotSizeText(screenshot.size)].filter(Boolean).join(' · ')}</small>
          </div>
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent aria-label={`Acciones de ${screenshot.name}`}
        onCloseAutoFocus={event => { if (opensDialog.current) { event.preventDefault(); opensDialog.current = false; } }}>
        {screenshotChoices.map(({ choice, label }) => {
          const Icon = choiceIcons[choice];
          return <div key={choice}>
            {choice === 'delete' && <ContextMenuSeparator />}
            <ContextMenuItem className={choice === 'delete' ? 'text-destructive focus:text-destructive' : undefined}
              onSelect={() => { opensDialog.current = choice === 'view' || choice === 'delete'; onChoice(choice, screenshot); }}>
              <Icon aria-hidden="true" />{label}
            </ContextMenuItem>
          </div>;
        })}
      </ContextMenuContent>
    </ContextMenu>
  </li>;
}

/**
 * The "Capturas" tab of an instance's settings: the screenshots of this instance only.
 * The backend validates every name and path; this panel never handles a file path.
 */
export function ScreenshotPanel({ instanceId }: { instanceId: string }) {
  const [list, setList] = useState<ScreenshotList | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [page, setPage] = useState(0);
  const [attempt, setAttempt] = useState(0);
  const [working, setWorking] = useState(false);
  const [viewing, setViewing] = useState<string | null>(null);
  const [pendingDelete, setPendingDelete] = useState<Screenshot | null>(null);
  const { notice, show } = useTransientNotice();
  const grid = useRef<HTMLUListElement>(null);

  // The gallery stays current by itself: it reads the folder when it opens, every few seconds
  // while it is visible and as soon as the window comes back, which is when a screenshot taken
  // in the game is most likely to be waiting. Quiet reads never flash an error.
  useEffect(() => {
    let active = true;
    let reading = false;
    async function read(first: boolean) {
      if (reading) return;
      reading = true;
      try {
        const next = await loadScreenshots(instanceId);
        if (!active) return;
        setError(null);
        setList(previous => sameScreenshotList(previous, next) ? previous : next);
        setPage(previous => clampScreenshotPage(previous, next.screenshots.length));
      } catch (cause) {
        if (!active) return;
        // A failed quiet read keeps what is shown, except when the folder stopped being valid.
        if (first || cause === 'invalid_data') {
          setError(messageForError(cause));
          if (cause === 'invalid_data') setList({ screenshots: [], truncated: false });
        }
      } finally {
        reading = false;
      }
    }
    setError(null);
    void read(true);
    const again = () => { if (document.visibilityState === 'visible') void read(false); };
    const timer = setInterval(again, SCREENSHOT_REFRESH_MS);
    document.addEventListener('visibilitychange', again);
    window.addEventListener('focus', again);
    return () => { active = false; clearInterval(timer); document.removeEventListener('visibilitychange', again); window.removeEventListener('focus', again); };
  }, [instanceId, attempt]);

  const screenshots = list?.screenshots ?? [];
  const viewIndex = viewing === null ? -1 : screenshots.findIndex(item => item.name === viewing);
  // The viewer follows its file by name; if a refresh drops that file, it closes.
  useEffect(() => { if (viewing !== null && list && viewIndex < 0) setViewing(null); }, [viewing, list, viewIndex]);

  async function run(action: ScreenshotAction, screenshot: Screenshot): Promise<boolean> {
    setWorking(true);
    try {
      await runScreenshotAction(action, instanceId, screenshot);
      if (action === 'copy') show(screenshotCopied);
      return true;
    } catch (cause) {
      show(screenshotActionError(action, cause), 'error');
      if (cause === 'invalid_data') setAttempt(previous => previous + 1);
      return false;
    } finally {
      setWorking(false);
    }
  }

  function choose(choice: ScreenshotChoice, screenshot: Screenshot) {
    if (choice === 'view') setViewing(screenshot.name);
    else if (choice === 'delete') setPendingDelete(screenshot);
    else void run(choice, screenshot);
  }

  async function confirmDelete(screenshot: Screenshot) {
    const moved = await run('delete', screenshot);
    setPendingDelete(null);
    if (!moved) return;
    const index = screenshots.findIndex(item => item.name === screenshot.name);
    const remaining = screenshots.filter(item => item.name !== screenshot.name);
    setList(previous => previous && { ...previous, screenshots: remaining });
    setPage(previous => clampScreenshotPage(previous, remaining.length));
    if (viewing !== null) {
      const next = indexAfterRemoval(index, remaining.length);
      setViewing(next === null ? null : remaining[next].name);
    }
    show(screenshotTrashed);
  }

  function closeViewer() {
    if (viewIndex >= 0) setPage(screenshotPageOf(viewIndex));
    const name = viewing;
    setViewing(null);
    // Return to the thumbnail that was open, if it is still there.
    if (name !== null) setTimeout(() => grid.current?.querySelector<HTMLElement>(`[data-screenshot="${CSS.escape(name)}"]`)?.focus(), 0);
  }

  const pages = screenshotPageCount(screenshots.length);
  const current = clampScreenshotPage(page, screenshots.length);
  return <section className="screenshot-panel" aria-labelledby="screenshot-heading">
    <h2 id="screenshot-heading" className="sr-only">Capturas</h2>
    {error && <div role="alert"><p className="account-error">No pudimos leer las capturas. {error}</p><button type="button" onClick={() => setAttempt(previous => previous + 1)}>Reintentar</button></div>}
    {list === null && !error && <p className="screenshot-empty" role="status"><LoaderCircle className="spin" aria-hidden="true" />Cargando capturas…</p>}
    {list !== null && screenshots.length === 0 && <div className="screenshot-empty" role="status">
      <ImageOff aria-hidden="true" />
      <h3>Aún no hay capturas</h3>
    </div>}
    {screenshots.length > 0 && <ul ref={grid} className="screenshot-grid" aria-label="Capturas de la instancia">
      {screenshotPage(screenshots, current).map(screenshot => <ScreenshotCard key={screenshot.name} instanceId={instanceId} screenshot={screenshot} onChoice={choose} />)}
    </ul>}
    {list?.truncated && <p className="screenshot-note">Mostramos las {screenshots.length} capturas más recientes. Las demás siguen en tu carpeta.</p>}
    {pages > 1 && <nav className="screenshot-pages" aria-label="Páginas de capturas">
      <Button variant="secondary" size="sm" disabled={current === 0} onClick={() => setPage(current - 1)}><ChevronLeft aria-hidden="true" />Anterior</Button>
      <span>Página {current + 1} de {pages}</span>
      <Button variant="secondary" size="sm" disabled={current + 1 >= pages} onClick={() => setPage(current + 1)}>Siguiente<ChevronRight aria-hidden="true" /></Button>
    </nav>}
    {viewIndex >= 0 && <ScreenshotViewer instanceId={instanceId} items={screenshots} index={viewIndex} working={working} notice={notice}
      onIndex={next => setViewing(screenshots[next]?.name ?? null)} onClose={closeViewer} onAction={(action, screenshot) => choose(action, screenshot)} />}
    {pendingDelete && <AccountDialog title="Mover a la papelera" className="screenshot-delete" onCancel={() => setPendingDelete(null)}>
      <p>«{pendingDelete.name}» se moverá a la papelera de tu sistema. Podrás restaurarla desde allí.</p>
      <div className="account-buttons">
        <Button variant="destructive" disabled={working} onClick={() => void confirmDelete(pendingDelete)}><Trash2 aria-hidden="true" />Mover a la papelera</Button>
        <Button variant="secondary" autoFocus disabled={working} onClick={() => setPendingDelete(null)}><X aria-hidden="true" />Cancelar</Button>
      </div>
    </AccountDialog>}
    {viewIndex < 0 && !pendingDelete && <TransientNotice notice={notice} />}
  </section>;
}
