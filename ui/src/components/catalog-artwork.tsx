import { useState } from 'react';
import { Blocks } from 'lucide-react';
import { safeArtwork } from '../launcher-state';

/** Artwork is optional and never hides the instance name or its controls. */
export function CatalogArtwork({ url, banner = false }: { url?: string | null; banner?: boolean }) {
  const source = safeArtwork(url);
  const [failed, setFailed] = useState<string | null>(null);
  if (source && source !== failed) return <img className={banner ? 'banner-image' : 'instance-icon'} src={source} alt="" aria-hidden="true" crossOrigin="anonymous" referrerPolicy="no-referrer" loading={banner ? 'eager' : 'lazy'} decoding="async" onError={() => setFailed(source)} />;
  return banner ? null : <Blocks aria-hidden="true" className="size-6" />;
}
