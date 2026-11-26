import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react';
import { ChevronLeft, ChevronRight, Copy, ExternalLink, FolderOpen, ImageOff, LoaderCircle, Trash2, X } from 'lucide-react';
import { Button } from './ui/button';
import { TransientNotice, type TransientNoticeState } from './transient-notice';
import { loadScreenshotImage } from '../screenshot-api';
import { createImageCache } from '../screenshot-images';
import { adjacentScreenshot, screenshotDateText, screenshotSizeText, type Screenshot, type ScreenshotAction } from '../screenshot-state';

/**
 * Full-size preview of one screenshot inside the launcher. It is a native modal dialog,
 * so the focus stays inside, Escape closes it and focus returns to the opener. Left and
 * Right step through the whole list. It carries its own toast, because the page's toast
 * sits underneath a modal dialog.
 */
export function ScreenshotViewer({ instanceId, items, index, working, notice, onIndex, onClose, onAction }: {
  instanceId: string;
  items: readonly Screenshot[];
  index: number;
  working: boolean;
  notice: TransientNoticeState | null;
  onIndex: (index: number) => void;
  onClose: () => void;
  onAction: (action: ScreenshotAction, screenshot: Screenshot) => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const cache = useMemo(() => createImageCache(screenshot => loadScreenshotImage(instanceId, screenshot)), [instanceId]);
  const [source, setSource] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const current = items[index];
  const previous = index > 0 ? items[index - 1] : undefined;
  const next = index + 1 < items.length ? items[index + 1] : undefined;

  useEffect(() => { if (!dialog.current?.open) dialog.current?.showModal(); }, []);

  useEffect(() => {
    if (!current) return;
    let active = true;
    setSource(null); setFailed(false);
    cache.get(current)
      .then(url => { if (active) setSource(url); })
      .catch(() => { if (active) setFailed(true); });
    // Warm the neighbours so stepping through is instant; their failures surface when shown.
    for (const neighbour of [next, previous]) neighbour && cache.get(neighbour).catch(() => {});
    return () => { active = false; };
  }, [cache, current?.name, current?.modifiedMs, next?.name, previous?.name]);

  if (!current) return null;
  const step = (direction: -1 | 1) => onIndex(adjacentScreenshot(index, direction, items.length));
  function keys(event: KeyboardEvent<HTMLDialogElement>) {
    if (event.key === 'ArrowLeft') { event.preventDefault(); step(-1); }
    else if (event.key === 'ArrowRight') { event.preventDefault(); step(1); }
  }
  const date = screenshotDateText(current.modifiedMs);
  return <dialog ref={dialog} className="screenshot-viewer" aria-label={`Captura ${current.name}`}
    onKeyDown={keys}
    onCancel={event => { event.preventDefault(); onClose(); }}
    // Only the backdrop is the dialog itself; the content fills an inner container.
    onClick={event => { if (event.target === event.currentTarget) onClose(); }}>
    <div className="screenshot-viewer-body">
      <header className="screenshot-viewer-header">
        <div className="screenshot-viewer-title">
          <strong title={current.name}>{current.name}</strong>
          <small>{[date, screenshotSizeText(current.size), `${index + 1} de ${items.length}`].filter(Boolean).join(' · ')}</small>
        </div>
        <Button variant="ghost" size="icon" className="screenshot-viewer-close" aria-label="Cerrar" onClick={onClose}><X aria-hidden="true" /></Button>
      </header>
      <div className="screenshot-viewer-stage">
        <Button variant="secondary" size="icon" aria-label="Captura anterior" disabled={!previous} onClick={() => step(-1)}><ChevronLeft aria-hidden="true" /></Button>
        <div className="screenshot-viewer-image">
          {source ? <img src={source} alt={`Captura ${current.name}`} /> : failed
            ? <span className="screenshot-placeholder"><ImageOff aria-hidden="true" />No se pudo cargar la imagen</span>
            : <span className="screenshot-placeholder" role="status"><LoaderCircle className="spin" aria-hidden="true" /><span className="sr-only">Cargando la captura {current.name}</span></span>}
        </div>
        <Button variant="secondary" size="icon" aria-label="Captura siguiente" disabled={!next} onClick={() => step(1)}><ChevronRight aria-hidden="true" /></Button>
      </div>
      <div className="screenshot-viewer-actions">
        <Button variant="secondary" size="sm" disabled={working} onClick={() => onAction('open', current)}><ExternalLink aria-hidden="true" />Abrir en fotos</Button>
        <Button variant="secondary" size="sm" disabled={working} onClick={() => onAction('reveal', current)}><FolderOpen aria-hidden="true" />Mostrar en la carpeta</Button>
        <Button variant="secondary" size="sm" disabled={working} onClick={() => onAction('copy', current)}><Copy aria-hidden="true" />Copiar imagen</Button>
        <Button variant="destructive" size="sm" disabled={working} onClick={() => onAction('delete', current)}><Trash2 aria-hidden="true" />Eliminar</Button>
      </div>
    </div>
    <TransientNotice notice={notice} />
  </dialog>;
}
