import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { LoaderCircle } from 'lucide-react';
import type { SkinViewer as Viewer } from 'skinview3d';
import type { SkinModel } from '../skin-state';
import { decodeImage } from '../use-texture';

export type ViewerAnimation = 'none' | 'idle' | 'walking' | 'running' | 'wave';

export const viewerAnimations: readonly (readonly [ViewerAnimation, string])[] = [
  ['idle', 'Reposo'], ['walking', 'Caminar'], ['running', 'Correr'], ['wave', 'Saludar'],
];

/** Players who ask the system for less motion get a still pose and no automatic turning. */
export const prefersReducedMotion = () => typeof window !== 'undefined' && !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;

/** One step of keyboard rotation, in radians. */
const KEY_TURN = Math.PI / 12;

/**
 * An interactive 3D player. It loads its renderer only when first shown, so the launcher starts
 * without it, and it shows `fallback` when the machine cannot provide WebGL. The player turns by
 * dragging, or with the arrow keys when the view has focus. Textures arrive as data URLs, which the
 * content security policy already allows. Every change of skin, model, cape or animation is applied
 * to the same renderer, so previews are immediate.
 */
export function SkinViewer3D({ skin, model, cape, animation, spin, paused = false, label, fallback }: {
  skin: string | null;
  model: SkinModel;
  cape: string | null;
  animation: ViewerAnimation;
  spin: boolean;
  /** Stop rendering while the view is not visible. */
  paused?: boolean;
  label: string;
  fallback?: ReactNode;
}) {
  const frame = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const viewer = useRef<Viewer | null>(null);
  const library = useRef<typeof import('skinview3d') | null>(null);
  const [status, setStatus] = useState<'loading' | 'ready' | 'unavailable'>('loading');
  // Image loads can finish out of order; only the latest request of each kind may apply.
  const latest = useRef({ skin: 0, cape: 0 });

  useEffect(() => {
    let disposed = false;
    let observer: ResizeObserver | undefined;
    let created: Viewer | undefined;
    void (async () => {
      try {
        const module = await import('skinview3d');
        const element = canvas.current;
        const box = frame.current;
        if (disposed || !element || !box) return;
        const { width, height } = box.getBoundingClientRect();
        created = new module.SkinViewer({ canvas: element, width: Math.max(1, Math.round(width)), height: Math.max(1, Math.round(height)), zoom: 0.85, fov: 40, enableControls: true });
        created.controls.enableZoom = false;
        created.controls.enablePan = false;
        library.current = module;
        viewer.current = created;
        observer = new ResizeObserver(() => {
          const size = box.getBoundingClientRect();
          if (created && size.width > 0 && size.height > 0) created.setSize(Math.round(size.width), Math.round(size.height));
        });
        observer.observe(box);
        setStatus('ready');
      } catch {
        // No WebGL, or the renderer failed to load: the flat picture stays.
        if (!disposed) setStatus('unavailable');
      }
    })();
    return () => {
      disposed = true;
      observer?.disconnect();
      created?.dispose();
      viewer.current = null;
      library.current = null;
    };
  }, []);

  useEffect(() => {
    const player = viewer.current;
    if (status !== 'ready' || !player) return;
    const token = ++latest.current.skin;
    if (!skin) { player.resetSkin(); return; }
    let current = true;
    decodeImage(skin).then(image => {
      if (current && token === latest.current.skin && !player.disposed) player.loadSkin(image, { model: model === 'slim' ? 'slim' : 'default' });
    }).catch(() => {});
    return () => { current = false; };
  }, [status, skin, model]);

  useEffect(() => {
    const player = viewer.current;
    if (status !== 'ready' || !player) return;
    const token = ++latest.current.cape;
    if (!cape) { player.resetCape(); return; }
    let current = true;
    decodeImage(cape).then(image => {
      if (current && token === latest.current.cape && !player.disposed) player.loadCape(image, { backEquipment: 'cape' });
    }).catch(() => {});
    return () => { current = false; };
  }, [status, cape]);

  useEffect(() => {
    const player = viewer.current;
    const module = library.current;
    if (status !== 'ready' || !player || !module) return;
    const reduced = prefersReducedMotion();
    player.animation = reduced || animation === 'none' ? null
      : animation === 'walking' ? new module.WalkingAnimation()
      : animation === 'running' ? new module.RunningAnimation()
      : animation === 'wave' ? new module.WaveAnimation()
      : new module.IdleAnimation();
    player.autoRotate = spin && !reduced;
    player.autoRotateSpeed = 0.8;
  }, [status, animation, spin]);

  useEffect(() => {
    const player = viewer.current;
    if (status === 'ready' && player) player.renderPaused = paused;
  }, [status, paused]);

  function turn(event: KeyboardEvent<HTMLCanvasElement>) {
    const player = viewer.current;
    if (!player || (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight')) return;
    event.preventDefault();
    player.playerWrapper.rotation.y += event.key === 'ArrowLeft' ? -KEY_TURN : KEY_TURN;
  }

  return <div ref={frame} className="skin-viewer-3d">
    <canvas ref={canvas} role="img" aria-label={`${label}. Arrastra para girar o usa las flechas del teclado.`} tabIndex={0} hidden={status === 'unavailable'} onKeyDown={turn} />
    {status === 'loading' && <span className="skin-viewer-state" role="status"><LoaderCircle className="spin" aria-hidden="true" />Preparando vista 3D…</span>}
    {status === 'unavailable' && <div className="skin-viewer-fallback" role="img" aria-label={label}>{fallback}<p>La vista 3D no está disponible en este equipo.</p></div>}
  </div>;
}
