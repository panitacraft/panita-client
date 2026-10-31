import { useState } from 'react';
import defaultSkin from '../assets/default-skin.svg';

/** Render only valid Minecraft names; Discord identifiers remain valid login inputs. */
export function ProfileArtwork({ username, bust = false }: { username: string; bust?: boolean }) {
  const valid = /^[A-Za-z0-9_]{3,16}$/.test(username);
  const source = valid ? `https://render.crafty.gg/${bust ? '3d/bust' : '2d/head'}/${encodeURIComponent(username)}?${bust ? 'width=128&height=128' : 'cors=true'}` : null;
  const [failed, setFailed] = useState<string | null>(null);
  return <img className={bust ? 'profile-bust' : 'profile-head'} src={source && failed !== source ? source : defaultSkin} alt="" aria-hidden="true" referrerPolicy="no-referrer" decoding="async" onError={() => { if (source) setFailed(source); }} />;
}

export function DiscordIcon() {
  return <svg viewBox="0 0 24 24" aria-hidden="true" className="brand-icon"><path fill="currentColor" d="M20.317 4.37a19.79 19.79 0 0 0-4.885-1.515.074.074 0 0 0-.079.037c-.21.375-.444.864-.608 1.25a18.27 18.27 0 0 0-5.487 0 12.64 12.64 0 0 0-.617-1.25.077.077 0 0 0-.079-.037A19.74 19.74 0 0 0 3.677 4.37a.07.07 0 0 0-.032.027C.533 9.046-.32 13.58.099 18.057a.082.082 0 0 0 .031.057 19.9 19.9 0 0 0 5.993 3.03.078.078 0 0 0 .084-.028c.462-.63.874-1.295 1.226-1.994a.076.076 0 0 0-.041-.106 13.1 13.1 0 0 1-1.872-.892.077.077 0 0 1-.008-.128c.126-.094.252-.192.372-.292a.074.074 0 0 1 .078-.01c3.928 1.793 8.18 1.793 12.062 0a.074.074 0 0 1 .079.009c.12.1.245.199.372.293a.077.077 0 0 1-.006.127 12.3 12.3 0 0 1-1.873.892.077.077 0 0 0-.041.107c.36.698.772 1.362 1.225 1.993a.076.076 0 0 0 .084.028 19.84 19.84 0 0 0 6.002-3.03.077.077 0 0 0 .032-.055c.5-5.177-.838-9.674-3.549-13.66a.06.06 0 0 0-.031-.028ZM8.02 15.331c-1.183 0-2.157-1.086-2.157-2.419 0-1.333.956-2.419 2.157-2.419 1.21 0 2.176 1.095 2.157 2.42 0 1.332-.956 2.418-2.157 2.418Zm7.975 0c-1.183 0-2.157-1.086-2.157-2.419 0-1.333.955-2.419 2.157-2.419 1.21 0 2.176 1.095 2.157 2.42 0 1.332-.946 2.418-2.157 2.418Z" /></svg>;
}

export function MicrosoftIcon() {
  return <svg viewBox="0 0 24 24" aria-hidden="true" className="brand-icon"><path fill="currentColor" d="M2 2h9v9H2zm11 0h9v9h-9zM2 13h9v9H2zm11 0h9v9h-9z" /></svg>;
}
