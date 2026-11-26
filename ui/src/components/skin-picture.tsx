import { useEffect, useState, type ReactNode } from "react";
import { ImageOff, LoaderCircle } from 'lucide-react';
import { CAPE_YAW, DEFAULT_YAW } from '../skin-angles';
import { BUST_FRAME, cachedThumbnail, renderThumbnail, type ThumbnailRequest } from '../skin-renderer';
import { textureUrl } from '../skin-api';
import type { SkinModel } from '../skin-state';
import { CapeThumbnail, SkinThumbnail } from './skin-thumbnails';

type Spec = Omit<ThumbnailRequest, 'skin' | 'cape'> & { skin: string; cape: string | null };
type State = { key: string | null; src: string | null; failed: boolean };

/** The 3D picture of a spec: from the cache at once, otherwise rendered in turn once it is wanted. */
function usePicture(spec: Spec | null): { src: string | null; failed: boolean } {
  const [state, setState] = useState<State>({ key: null, src: null, failed: false });
  const key = spec?.key ?? null;
  useEffect(() => {
    if (!spec || cachedThumbnail(spec.key)) return;
    let current = true;
    const { skin, cape } = spec;
    void (async () => {
      try {
        const [skinUrl, capeUrl] = await Promise.all([textureUrl('skin', skin), cape ? textureUrl('cape', cape) : Promise.resolve(null)]);
        const src = await renderThumbnail({ ...spec, skin: skinUrl, cape: capeUrl });
        if (current) setState({ key: spec.key, src, failed: false });
      } catch {
        if (current) setState({ key: spec.key, src: null, failed: true });
      }
    })();
    return () => { current = false; };
    // The spec is rebuilt on every render; its key says whether it changed.
  }, [key]);
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
  const { src, failed } = usePicture(texture ? { key: `skin|${texture}|${model}`, skin: texture, model, cape: null, yaw: DEFAULT_YAW } : null);
  return texture === null
    ? <span className="skin-placeholder"><ImageOff aria-hidden="true" /><span className="sr-only">{label}: imagen no disponible</span></span>
    : <Picture src={src} failed={failed} label={label} fallback={<SkinThumbnail texture={texture} model={model} label={label} />} />;
}

/**
 * A cape as it hangs on the player, seen from behind. Without a worn skin to hang it on, or without
 * WebGL, it falls back to the flat picture of the cape.
 */
export function CapePicture({ id, cached, wearer, model, label }: { id: string; cached: boolean; wearer: string | null; model: SkinModel; label: string }) {
  const spec = cached && wearer ? { key: `cape|${id}|${wearer}|${model}`, skin: wearer, model, cape: id, yaw: CAPE_YAW } : null;
  const { src, failed } = usePicture(spec);
  return spec === null
    ? <CapeThumbnail id={id} cached={cached} label={label} />
    : <Picture src={src} failed={failed} label={label} fallback={<CapeThumbnail id={id} cached={cached} label={label} />} />;
}

/**
 * The head and shoulders of a skin, as a 3D picture at the angle of the preview. `fallback` stands in
 * until the picture is ready and whenever it cannot be drawn.
 */
export function BustPicture({ texture, model, className, fallback }: { texture: string; model: SkinModel; className: string; fallback: ReactNode }) {
  const { src } = usePicture({ key: `bust|${texture}|${model}`, skin: texture, model, cape: null, yaw: DEFAULT_YAW, frame: BUST_FRAME });
  return src ? <img className={className} src={src} alt="" aria-hidden="true" decoding="async" draggable={false} /> : <>{fallback}</>;
}
