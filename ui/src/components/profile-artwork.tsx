import { useState } from 'react';
import defaultSkin from '../assets/default-skin.svg';

/** Render only valid Minecraft names; Discord identifiers remain valid login inputs. */
export function ProfileArtwork({ username, bust = false }: { username: string; bust?: boolean }) {
  const valid = /^[A-Za-z0-9_]{3,16}$/.test(username);
  const source = valid ? `https://render.crafty.gg/${bust ? '3d/bust' : '2d/head'}/${encodeURIComponent(username)}?${bust ? 'width=128&height=128' : 'cors=true'}` : null;
  const [failed, setFailed] = useState<string | null>(null);
  return <img className={bust ? 'profile-bust' : 'profile-head'} src={source && failed !== source ? source : defaultSkin} alt="" aria-hidden="true" referrerPolicy="no-referrer" decoding="async" onError={() => { if (source) setFailed(source); }} />;
}

export function MicrosoftIcon() {
  return <svg viewBox="0 0 24 24" aria-hidden="true" className="brand-icon"><path fill="currentColor" d="M2 2h9v9H2zm11 0h9v9h-9zM2 13h9v9H2zm11 0h9v9h-9z" /></svg>;
}
