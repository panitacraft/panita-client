import { useEffect, useState, type ReactNode } from 'react';
import { ImageOff, LoaderCircle } from 'lucide-react';
import { BUST_YAW, CAPE_YAW, DEFAULT_YAW } from '../skin-angles';
import { BUST_FRAME, cachedThumbnail, renderThumbnail, type ThumbnailRequest } from '../skin-renderer';
import { textureUrl } from '../skin-api';
import type { SkinModel } from '../skin-state';
import { CapeThumbnail, SkinThumbnail } from './skin-thumbnails';

/** Where a skin comes from: the launcher's cache of a profile's skin, or an image shipped with the launcher. */
export type SkinSource = { kind: 'cached'; texture: string } | { kind: 'asset'; name: string; url: string };

type Spec = Omit<ThumbnailRequest, 'skin' | 'cape'> & { skin: SkinSource; cape: string | null };
type State = { key: string | null; src: string | null; failed: boolean };

const skinUrlOf = (source: SkinSource) => source.kind === 'asset' ? Promise.resolve(source.url) : textureUrl('skin', source.texture);

/**
 * The 3D picture of a spec: from the cache at once, otherwise rendered in turn once it is wanted. A picture
 * that is not cached waits for `allowed`, so a hidden page never starts the renderer.
 */
function usePicture(spec: Spec | null, allowed = true): { src: string | null; failed: boolean } {
  const [state, setState] = useState<State>({ key: null, src: null, failed: false });
  const key = spec?.key ?? null;
  useEffect(() => {
    if (!spec || !allowed || cachedThumbnail(spec.key)) return;
    let current = true;
    const { skin, cape } = spec;
    void (async () => {
      try {
        const [skinUrl, capeUrl] = await Promise.all([skinUrlOf(skin), cape ? textureUrl('cape', cape) : Promise.resolve(null)]);
        const src = await renderThumbnail({ ...spec, skin: skinUrl, cape: capeUrl });
        if (current) setState({ key: spec.key, src, failed: false });
      } catch {
        if (current) setState({ key: spec.key, src: null, failed: true });
      }
    })();
    return () => { current = false; };
    // The spec is rebuilt on every render; its key says whether it changed.
  }, [key, allowed]);
  if (!spec) return { src: null, failed: true };
  const cached = cachedThumbnail(spec.key);
  if (cached) return { src: cached, failed: false };
  return state.key === spec.key ? { src: state.src, failed: state.failed } : { src: null, failed: false };
}

function Picture({ src, failed, fallback, label }: { src: string | null; failed: boolean; fallback: ReactNode; label: string }) {
  if (src) return <img className="skin-picture" src={src} alt="" decoding="async" draggable={false} />;
  if (failed) return <>{fallback}</>;
  return <span className="skin-placeholder" role="status"><LoaderCircle className="spin" aria-hidden="true" /><span className="sr-only">Cargando {label}</span></span>;
}

/** A saved or worn skin as a small 3D picture at the angle of the main preview. */
export function SkinPicture({ texture, model, label }: { texture: string | null; model: SkinModel; label: string }) {
  const { src, failed } = usePicture(texture ? { key: `skin|${texture}|${model}`, skin: { kind: 'cached', texture }, model, cape: null, yaw: DEFAULT_YAW } : null);
  return texture === null
    ? <span className="skin-placeholder"><ImageOff aria-hidden="true" /><span className="sr-only">{label}: imagen no disponible</span></span>
    : <Picture src={src} failed={failed} label={label} fallback={<SkinThumbnail texture={texture} model={model} label={label} />} />;
}

/**
 * A cape as it hangs on the player, seen from behind. Without a worn skin to hang it on, or without
 * WebGL, it falls back to the flat picture of the cape.
 */
export function CapePicture({ id, cached, wearer, model, label }: { id: string; cached: boolean; wearer: string | null; model: SkinModel; label: string }) {
  const spec: Spec | null = cached && wearer ? { key: `cape|${id}|${wearer}|${model}`, skin: { kind: 'cached', texture: wearer }, model, cape: id, yaw: CAPE_YAW } : null;
  const { src, failed } = usePicture(spec);
  return spec === null
    ? <CapeThumbnail id={id} cached={cached} label={label} />
    : <Picture src={src} failed={failed} label={label} fallback={<CapeThumbnail id={id} cached={cached} label={label} />} />;
}

/** What makes a bust picture different from another: the framing and angle as well as the skin, so changing them never serves an old picture. */
const BUST_SIGNATURE = [BUST_YAW, ...Object.values(BUST_FRAME)].join(',');

/**
 * The head and torso of a skin, as a 3D picture shaped like the online busts it replaces. It is drawn only once
 * `allowed` (the page showing it is open), shows `pending` until then, and `fallback` whenever it cannot be drawn.
 */
export function BustPicture({ source, model, className, allowed = true, pending = null, fallback }: {
  source: SkinSource; model: SkinModel; className: string; allowed?: boolean; pending?: ReactNode; fallback: ReactNode;
}) {
  const name = source.kind === 'asset' ? `asset:${source.name}` : `cached:${source.texture}`;
  const { src, failed } = usePicture({ key: `bust|${BUST_SIGNATURE}|${name}|${model}`, skin: source, model, cape: null, yaw: BUST_YAW, frame: BUST_FRAME }, allowed);
  if (src) return <img className={className} src={src} alt="" aria-hidden="true" decoding="async" draggable={false} />;
  return <>{failed ? fallback : pending}</>;
}
