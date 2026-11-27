import { useEffect, useState } from 'react';
import { textureUrl } from './skin-api';

type Loaded = { key: string | null; url: string | null; failed: boolean };

/** The data URL of a cached skin or cape, or none while it loads or when it cannot be read. */
export function useTextureUrl(kind: 'skin' | 'cape', key: string | null): { url: string | null; failed: boolean } {
  const [state, setState] = useState<Loaded>({ key: null, url: null, failed: false });
  useEffect(() => {
    if (!key) return;
    let current = true;
    textureUrl(kind, key)
      .then(url => { if (current) setState({ key, url, failed: false }); })
      .catch(() => { if (current) setState({ key, url: null, failed: true }); });
    return () => { current = false; };
  }, [kind, key]);
  // A different key never shows the previous key's image while its own loads.
  return key !== null && state.key === key ? { url: state.url, failed: state.failed } : { url: null, failed: false };
}

/** Decode a data URL into an image, so drawing and 3D loading happen in a known order. */
export function decodeImage(url: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error('unreadable image'));
    image.src = url;
  });
}
