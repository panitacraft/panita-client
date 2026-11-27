import type { SkinViewer } from 'skinview3d';
import type { SkinModel } from './skin-state';
import { decodeImage } from './use-texture';

/**
 * Renders the 3D pictures of the library cards. A WebGL context per card would exhaust the few a
 * browser allows, so one hidden player renders every picture in turn, standing still, and the result
 * is kept as an image. The pictures are cached by what they show, so each skin is rendered once.
 */
/**
 * How a picture is framed. `pitch` raises the camera so it looks down on the player, `spread` angles the arms
 * outward (both in radians) and `fov` is the camera's field of view in degrees; a narrower one flattens perspective.
 */
export type Frame = { width: number; height: number; zoom: number; lift: number; pitch?: number; spread?: number; fov?: number };

export type ThumbnailRequest = {
  /** Identifies what is drawn; equal keys give the same picture. */
  key: string;
  skin: string;
  model: SkinModel;
  cape: string | null;
  yaw: number;
  /** A different framing, for the bust shown beside an account. Everything else draws the whole body. */
  frame?: Frame;
};

export const THUMBNAIL_WIDTH = 240;
export const THUMBNAIL_HEIGHT = 320;
const BODY_FRAME: Frame = { width: THUMBNAIL_WIDTH, height: THUMBNAIL_HEIGHT, zoom: 0.78, lift: 0 };
/** Head and torso seen from a little above, square like the online busts it replaces so it sits in the same place. */
export const BUST_FRAME: Frame = { width: 256, height: 256, zoom: 1.6, lift: -5.9, pitch: 0.3, spread: 0.18, fov: 24 };
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
    const frame = request.frame ?? BODY_FRAME;
    viewer.setSize(frame.width, frame.height);
    // Aim the camera from above before the library rescales its distance, then look at the player's centre.
    const pitch = frame.pitch ?? 0;
    viewer.camera.position.set(0, Math.sin(pitch), Math.cos(pitch));
    viewer.fov = frame.fov ?? 40;
    viewer.zoom = frame.zoom;
    viewer.camera.lookAt(0, 0, 0);
    viewer.playerWrapper.position.y = frame.lift;
    viewer.playerWrapper.rotation.y = request.yaw;
    viewer.playerObject.skin.leftArm.rotation.z = frame.spread ?? 0;
    viewer.playerObject.skin.rightArm.rotation.z = -(frame.spread ?? 0);
    viewer.render();
    const picture = viewer.canvas.toDataURL('image/png');
    remember(request.key, picture);
    return picture;
  });
  // The chain must go on after a failed picture.
  queue = work.catch(() => undefined);
  return work;
}
