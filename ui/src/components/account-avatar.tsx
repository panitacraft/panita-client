import { useState } from 'react';
import type { AccountView } from '../account-state';
import defaultSkin from '../assets/default-skin.svg';

/** Offline accounts never request a skin from an online service. */
export function avatarUrl(metadata: AccountView['metadata'] | null): string | null {
  if (metadata?.kind !== 'microsoft') return null;
  const name = metadata.username;
  return /^[A-Za-z0-9_]{3,16}$/.test(name) ? `https://render.crafty.gg/2d/head/${encodeURIComponent(name)}?cors=true` : null;
}

export function AccountAvatar({ metadata }: { metadata: AccountView['metadata'] | null }) {
  const source = avatarUrl(metadata);
  const [failed, setFailed] = useState<string | null>(null);
  return <img className="account-avatar" src={source && source !== failed ? source : defaultSkin} alt="" aria-hidden="true" crossOrigin="anonymous" referrerPolicy="no-referrer" decoding="async" onError={() => { if (source) setFailed(source); }} />;
}
