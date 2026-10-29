import { useState } from 'react';
import type { AccountView } from '../account-state';
import defaultSkin from '../assets/default-skin.svg';

/** Offline accounts never request a skin from an online service. */
export function avatarUrl(metadata: AccountView['metadata'] | null): string | null {
  if (metadata?.kind !== 'microsoft') return null;
  const id = metadata.profile_id.replaceAll('-', '');
  return /^[a-f0-9]{32}$/i.test(id) ? `https://crafatar.com/avatars/${id}?size=48&overlay` : null;
}

export function AccountAvatar({ metadata }: { metadata: AccountView['metadata'] | null }) {
  const source = avatarUrl(metadata);
  const [failed, setFailed] = useState<string | null>(null);
  return <img className="account-avatar" src={source && source !== failed ? source : defaultSkin} alt="" aria-hidden="true" crossOrigin="anonymous" referrerPolicy="no-referrer" decoding="async" onError={() => { if (source) setFailed(source); }} />;
}
