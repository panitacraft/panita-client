import { useEffect, useRef, useState } from 'react';
import { ImageOff, LoaderCircle } from 'lucide-react';
import { CAPE_HEIGHT, CAPE_WIDTH, FRONT_HEIGHT, FRONT_WIDTH, drawCape, drawFront } from '../skin-render';
import type { SkinModel } from '../skin-state';
import { decodeImage, useTextureUrl } from '../use-texture';

type Drawn = 'loading' | 'ready' | 'failed';

/** Draw a decoded texture into the canvas; `look` is whatever else the drawing depends on, such as the model. */
function useDrawn(url: string | null, failed: boolean, look: string, draw: (context: CanvasRenderingContext2D, image: HTMLImageElement) => void) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [state, setState] = useState<Drawn>('loading');
  const drawing = useRef(draw);
  drawing.current = draw;
  useEffect(() => {
    if (failed) { setState('failed'); return; }
    if (!url) { setState('loading'); return; }
    let current = true;
    decodeImage(url).then(image => {
      const context = canvas.current?.getContext('2d');
      if (!current || !context) { if (current) setState('failed'); return; }
      drawing.current(context, image);
      setState('ready');
    }).catch(() => { if (current) setState('failed'); });
    return () => { current = false; };
  }, [url, failed, look]);
  return { canvas, state };
}

function Placeholder({ state, label }: { state: Drawn; label: string }) {
  return state === 'failed'
    ? <span className="skin-placeholder"><ImageOff aria-hidden="true" /><span className="sr-only">{label}: imagen no disponible</span></span>
    : <span className="skin-placeholder" role="status"><LoaderCircle className="spin" aria-hidden="true" /><span className="sr-only">Cargando {label}</span></span>;
}

/** The front of a saved or worn skin, as a small pixel-art picture. */
export function SkinThumbnail({ texture, model, label }: { texture: string | null; model: SkinModel; label: string }) {
  const { url, failed } = useTextureUrl('skin', texture);
  const { canvas, state } = useDrawn(url, failed, model, (context, image) => drawFront(context, image, model));
  return <span className="skin-thumbnail">
    <canvas ref={canvas} width={FRONT_WIDTH} height={FRONT_HEIGHT} aria-hidden="true" hidden={state !== 'ready'} />
    {state !== 'ready' && <Placeholder state={texture === null ? 'failed' : state} label={label} />}
  </span>;
}

/** The outside of a cape. A cape whose image is not cached yet shows a placeholder. */
export function CapeThumbnail({ id, cached, label }: { id: string; cached: boolean; label: string }) {
  const { url, failed } = useTextureUrl('cape', cached ? id : null);
  const { canvas, state } = useDrawn(url, failed, 'cape', drawCape);
  return <span className="cape-thumbnail">
    <canvas ref={canvas} width={CAPE_WIDTH} height={CAPE_HEIGHT} aria-hidden="true" hidden={state !== 'ready'} />
    {state !== 'ready' && <Placeholder state={cached ? state : 'failed'} label={label} />}
  </span>;
}
