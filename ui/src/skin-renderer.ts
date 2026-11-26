import type { SkinViewer } from 'skinview3d';
import type { SkinModel } from './skin-state';
import { decodeImage } from './use-texture';

/**
 * Renders the 3D pictures of the library cards. A WebGL context per card would exhaust the few a
 * browser allows, so one hidden player renders every picture in turn, standing still, and the result
 * is kept as an image. The pictures are cached by what they show, so each skin is rendered once.
 */
export type ThumbnailRequest = {
  /** Identifies what is drawn; equal keys give the same picture. */
  key: string;
  skin: string;
  model: SkinModel;
  cape: string | null;
  yaw: number;
};

export const THUMBNAIL_WIDTH = 240;
export const THUMBNAIL_HEIGHT = 320;
const CACHE_LIMIT = 240;

const cache = new Map<string, string>();
let player: Promise<SkinViewer> | null = null;
let queue: Promise<unknown> = Promise.resolve();

export const cachedThumbnail = (key: string): string | undefined => cache.get(key);

function createPlayer(): Promise<SkinViewer> {
  player ??= import('skinview3d').then(module => {
    const viewer = new module.SkinViewer({
      canvas: document.createElement('canvas'), width: THUMBNAIL_WIDTH, height: THUMBNAIL_HEIGHT, pixelRatio: 1,
      zoom: 0.78, fov: 40, enableControls: false, renderPaused: true, preserveDrawingBuffer: true,
    });
    viewer.animation = null;
    return viewer;
  }).catch(error => { player = null; throw error; });
  return player;
}

function remember(key: string, picture: string) {
  cache.delete(key);
  cache.set(key, picture);
  while (cache.size > CACHE_LIMIT) {
    const oldest = cache.keys().next();
    if (oldest.done) break;
    cache.delete(oldest.value);
  }
}

/** The picture for a request, rendered after the ones already waiting. A failure is never remembered. */
export function renderThumbnail(request: ThumbnailRequest): Promise<string> {
  const known = cache.get(request.key);
  if (known) return Promise.resolve(known);
  const work = queue.then(async () => {
    const again = cache.get(request.key);
    if (again) return again;
    const [viewer, skin, cape] = await Promise.all([createPlayer(), decodeImage(request.skin), request.cape ? decodeImage(request.cape) : Promise.resolve(null)]);
    viewer.loadSkin(skin, { model: request.model === 'slim' ? 'slim' : 'default' });
    if (cape) viewer.loadCape(cape, { backEquipment: 'cape' }); else viewer.resetCape();
    viewer.playerWrapper.rotation.y = request.yaw;
    viewer.render();
    const picture = viewer.canvas.toDataURL('image/png');
    remember(request.key, picture);
    return picture;
  });
  // The chain must go on after a failed picture.
  queue = work.catch(() => undefined);
  return work;
}
