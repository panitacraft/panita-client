import { useEffect, useRef, useState, type ReactNode } from 'react';
import type { AccountView } from '../account-state';
import defaultSkin from '../assets/default-skin.svg';
import { drawHead, HEAD_SIZE } from '../skin-render';
import type { SkinModel } from '../skin-state';
import { decodeImage, useTextureUrl } from '../use-texture';
import { useWornSkin } from '../worn-skin';

/** Offline accounts never request a skin from an online service. */
export function avatarUrl(metadata: AccountView['metadata'] | null): string | null {
  if (metadata?.kind !== 'microsoft') return null;
  const name = metadata.username;
  return /^[A-Za-z0-9_]{3,16}$/.test(name) ? `https://render.crafty.gg/2d/head/${encodeURIComponent(name)}?cors=true` : null;
}

/** The face of a cached skin, drawn from its own texture so it is right the moment the skin changes. */
export function WornHead({ texture, model, className, fallback }: { texture: string; model: SkinModel; className: string; fallback: ReactNode }) {
  const { url } = useTextureUrl('skin', texture);
  const canvas = useRef<HTMLCanvasElement>(null);
  const [ready, setReady] = useState(false);
  useEffect(() => {
    if (!url) { setReady(false); return; }
    let current = true;
    decodeImage(url).then(image => {
      const context = canvas.current?.getContext('2d');
      if (!current || !context) return;
      drawHead(context, image);
      setReady(true);
    }).catch(() => { if (current) setReady(false); });
    return () => { current = false; };
  }, [url, model]);
  return <>
    <canvas ref={canvas} className={className} width={HEAD_SIZE} height={HEAD_SIZE} aria-hidden="true" hidden={!ready} />
    {!ready && fallback}
  </>;
}

/** The selected account's face: its cached skin when the launcher knows it, the online render until then. */
export function AccountAvatar({ metadata }: { metadata: AccountView['metadata'] | null }) {
  const source = avatarUrl(metadata);
  const [failed, setFailed] = useState<string | null>(null);
  const worn = useWornSkin(metadata?.kind === 'microsoft' ? metadata.profile_id : null, true);
  const online = <img className="account-avatar" src={source && source !== failed ? source : defaultSkin} alt="" aria-hidden="true" crossOrigin="anonymous" referrerPolicy="no-referrer" decoding="async" onError={() => { if (source) setFailed(source); }} />;
  return worn ? <WornHead texture={worn.texture} model={worn.model} className="account-avatar" fallback={online} /> : online;
}
