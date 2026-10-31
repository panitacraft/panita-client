import { useEffect, useRef, useState } from 'react';
import { Blocks } from 'lucide-react';
import { safeArtwork } from '../launcher-state';
import logoWhite from '../assets/logo-white.svg';

/** Artwork is optional and never hides the instance name or its controls. It fades in once it has loaded. */
export function CatalogArtwork({ url, banner = false }: { url?: string | null; banner?: boolean }) {
  const source = safeArtwork(url);
  const [failed, setFailed] = useState<string | null>(null);
  const [loaded, setLoaded] = useState<string | null>(null);
  // After the slow first reveal, later reveals (switching instances) use shorter durations.
  const [played, setPlayed] = useState(false);
  const image = useRef<HTMLImageElement>(null);
  // A cached image can finish loading before React attaches onLoad.
  useEffect(() => { if (source && image.current?.complete && image.current.naturalWidth > 0) setLoaded(source); }, [source]);
  if (source && source !== failed) return <img ref={image} className={`${banner ? 'banner-image' : 'instance-icon'} artwork-fade ${loaded === source ? 'is-loaded' : ''} ${played ? 'has-played' : ''}`} src={source} alt="" aria-hidden="true" crossOrigin="anonymous" referrerPolicy="no-referrer" loading={banner ? 'eager' : 'lazy'} decoding="async" onLoad={() => setLoaded(source)} onAnimationEnd={() => setPlayed(true)} onError={() => setFailed(source)} />;
  return banner ? <><div className="hero-grid" aria-hidden="true" /><img className="hero-brand" src={logoWhite} alt="" aria-hidden="true" /></> : <Blocks aria-hidden="true" className="size-6" />;
}
