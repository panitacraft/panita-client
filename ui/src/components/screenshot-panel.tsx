import { useEffect, useRef, useState } from 'react';
import { ChevronLeft, ChevronRight, Copy, ExternalLink, FolderOpen, ImageOff, LoaderCircle, RefreshCw } from 'lucide-react';
import { Button } from './ui/button';
import { Hint } from './hint';
import { messageForError } from '../errors';
import { loadScreenshotImage, loadScreenshots, runScreenshotAction } from '../screenshot-api';
import {
  clampScreenshotPage, screenshotActionError, screenshotCopied, screenshotDateText, screenshotPage, screenshotPageCount, screenshotSizeText,
  type Screenshot, type ScreenshotAction, type ScreenshotList,
} from '../screenshot-state';

type Notice = { kind: 'ok' | 'error'; text: string };

/** One capture. Its image is fetched only once the card scrolls into view. */
function ScreenshotCard({ instanceId, screenshot, disabled, onAction }: { instanceId: string; screenshot: Screenshot; disabled: boolean; onAction: (action: ScreenshotAction, screenshot: Screenshot) => void }) {
  const frame = useRef<HTMLDivElement>(null);
  const [visible, setVisible] = useState(typeof IntersectionObserver === 'undefined');
  const [source, setSource] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

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

  const date = screenshotDateText(screenshot.modifiedMs);
  return <li className="screenshot-card" aria-label={screenshot.name}>
    <div ref={frame} className="screenshot-frame">
      {source ? <img src={source} alt={`Captura ${screenshot.name}`} decoding="async" /> : failed
        ? <span className="screenshot-placeholder"><ImageOff aria-hidden="true" />No se pudo cargar la imagen</span>
        : <span className="screenshot-placeholder" role="status"><LoaderCircle className="spin" aria-hidden="true" /><span className="sr-only">Cargando la captura {screenshot.name}</span></span>}
    </div>
    <div className="screenshot-meta">
      <strong title={screenshot.name}>{screenshot.name}</strong>
      <small>{[date, screenshotSizeText(screenshot.size)].filter(Boolean).join(' · ')}</small>
    </div>
    <div className="screenshot-actions">
      <Hint text="Abrir con el visor de imágenes de tu sistema"><Button variant="secondary" size="sm" disabled={disabled} aria-label={`Abrir ${screenshot.name} en el visor de imágenes`} onClick={() => onAction('open', screenshot)}><ExternalLink aria-hidden="true" />Abrir</Button></Hint>
      <Hint text="Mostrar en el explorador de archivos"><Button variant="secondary" size="sm" disabled={disabled} aria-label={`Mostrar ${screenshot.name} en el explorador de archivos`} onClick={() => onAction('reveal', screenshot)}><FolderOpen aria-hidden="true" />Mostrar</Button></Hint>
      <Hint text="Copiar la imagen al portapapeles"><Button variant="secondary" size="sm" disabled={disabled} aria-label={`Copiar la imagen ${screenshot.name} al portapapeles`} onClick={() => onAction('copy', screenshot)}><Copy aria-hidden="true" />Copiar</Button></Hint>
    </div>
  </li>;
}

/**
 * The "Capturas" tab of an instance's settings: the screenshots of this instance only.
 * The backend validates every name and path; this panel never handles a file path.
 */
export function ScreenshotPanel({ instanceId }: { instanceId: string }) {
  const [list, setList] = useState<ScreenshotList | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [page, setPage] = useState(0);
  const [attempt, setAttempt] = useState(0);
  const [working, setWorking] = useState(false);
  const [notice, setNotice] = useState<Notice | null>(null);

  useEffect(() => {
    let current = true;
    setLoading(true); setError(null);
    loadScreenshots(instanceId)
      .then(next => { if (current) { setList(next); setPage(previous => clampScreenshotPage(previous, next.screenshots.length)); } })
      .catch(cause => { if (current) setError(messageForError(cause)); })
      .finally(() => { if (current) setLoading(false); });
    return () => { current = false; };
  }, [instanceId, attempt]);

  async function act(action: ScreenshotAction, screenshot: Screenshot) {
    setWorking(true); setNotice(null);
    try {
      await runScreenshotAction(action, instanceId, screenshot);
      if (action === 'copy') setNotice({ kind: 'ok', text: screenshotCopied });
    } catch (cause) {
      setNotice({ kind: 'error', text: screenshotActionError(action, cause) });
      if (cause === 'invalid_data') setAttempt(previous => previous + 1);
    } finally {
      setWorking(false);
    }
  }

  const screenshots = list?.screenshots ?? [];
  const pages = screenshotPageCount(screenshots.length);
  const current = clampScreenshotPage(page, screenshots.length);
  return <section className="screenshot-panel" aria-labelledby="screenshot-heading">
    <h2 id="screenshot-heading" className="sr-only">Capturas</h2>
    <div className="screenshot-toolbar">
      <p>{screenshots.length === 0 ? 'Las capturas de pantalla de Minecraft de esta instancia.' : `${screenshots.length} ${screenshots.length === 1 ? 'captura' : 'capturas'} de esta instancia, de la más reciente a la más antigua.`}</p>
      <Button variant="secondary" size="sm" disabled={loading} onClick={() => setAttempt(previous => previous + 1)}><RefreshCw aria-hidden="true" />Actualizar</Button>
    </div>
    {error && <div role="alert"><p className="account-error">No pudimos leer las capturas. {error}</p><button type="button" onClick={() => setAttempt(previous => previous + 1)}>Reintentar</button></div>}
    {notice && <p role={notice.kind === 'error' ? 'alert' : 'status'} className={notice.kind === 'error' ? 'account-error' : 'screenshot-notice'}>{notice.text}</p>}
    {list === null && !error && <p className="screenshot-empty" role="status"><LoaderCircle className="spin" aria-hidden="true" />Cargando capturas…</p>}
    {list !== null && screenshots.length === 0 && <div className="screenshot-empty" role="status">
      <ImageOff aria-hidden="true" />
      <h3>Aún no hay capturas</h3>
      <p>Las capturas que hagas dentro de Minecraft con la tecla F2 aparecerán aquí.</p>
    </div>}
    {screenshots.length > 0 && <ul className="screenshot-grid" aria-label="Capturas de la instancia">
      {screenshotPage(screenshots, current).map(screenshot => <ScreenshotCard key={screenshot.name} instanceId={instanceId} screenshot={screenshot} disabled={working} onAction={(action, target) => void act(action, target)} />)}
    </ul>}
    {list?.truncated && <p className="screenshot-notice">Mostramos las {screenshots.length} capturas más recientes. Las demás siguen en tu carpeta.</p>}
    {pages > 1 && <nav className="screenshot-pages" aria-label="Páginas de capturas">
      <Button variant="secondary" size="sm" disabled={current === 0} onClick={() => setPage(current - 1)}><ChevronLeft aria-hidden="true" />Anterior</Button>
      <span>Página {current + 1} de {pages}</span>
      <Button variant="secondary" size="sm" disabled={current + 1 >= pages} onClick={() => setPage(current + 1)}>Siguiente<ChevronRight aria-hidden="true" /></Button>
    </nav>}
  </section>;
}
