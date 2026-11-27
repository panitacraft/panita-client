import { useEffect, useRef, useState, type ReactNode } from 'react';
import type { AccountView } from '../account-state';
import defaultSkin from '../assets/default-skin.svg';
import steveSkin from '../assets/steve.png';
import { drawHead, HEAD_SIZE } from '../skin-render';
import { decodeImage, useTextureUrl } from '../use-texture';
import { useWornSkin } from '../worn-skin';
import type { SkinSource } from './skin-picture';

/** The skin of accounts that have none of their own: Minecraft's default, shipped with the launcher. */
export const STEVE: SkinSource = { kind: 'asset', name: 'steve', url: steveSkin };

/** The face of a skin, drawn flat from its own texture: from the cache for a profile, from the file for a shipped one. */
export function SkinHead({ source, className, fallback }: { source: SkinSource; className: string; fallback: ReactNode }) {
  const cached = useTextureUrl('skin', source.kind === 'cached' ? source.texture : null);
  const url = source.kind === 'asset' ? source.url : cached.url;
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
  }, [url]);
  return <>
    <canvas ref={canvas} className={className} width={HEAD_SIZE} height={HEAD_SIZE} aria-hidden="true" hidden={!ready} />
    {!ready && fallback}
  </>;
}

/**
 * The selected account's face, drawn from the launcher's own cache of its skin, never from an online picture service.
 * Until the launcher has looked it shows an empty box, so nothing flashes from one picture to another.
 */
export function AccountAvatar({ metadata }: { metadata: AccountView['metadata'] | null }) {
  const microsoft = metadata?.kind === 'microsoft';
  const worn = useWornSkin(microsoft ? metadata.profile_id : null);
  const local = <img className="account-avatar" src={defaultSkin} alt="" aria-hidden="true" decoding="async" />;
  if (!microsoft) return local;
  if (worn === undefined) return <span className="account-avatar" aria-hidden="true" />;
  return <SkinHead source={worn?.texture ? { kind: 'cached', texture: worn.texture } : STEVE} className="account-avatar" fallback={local} />;
}
