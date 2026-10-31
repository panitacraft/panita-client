import { useState } from 'react';

const STORAGE_KEY = 'panita.motion';

/** Motion is a per-device interface preference; it defaults to the operating system's reduced-motion setting. */
export function readMotion(): boolean {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored === 'on') return true;
    if (stored === 'off') return false;
  } catch { /* storage may be unavailable */ }
  return !(typeof window.matchMedia === 'function' && window.matchMedia('(prefers-reduced-motion: reduce)').matches);
}

export function applyMotion(enabled: boolean) {
  document.documentElement.dataset.motion = enabled ? 'on' : 'off';
}

export function useMotion() {
  const [enabled, setEnabled] = useState(readMotion);
  function change(next: boolean) {
    try { localStorage.setItem(STORAGE_KEY, next ? 'on' : 'off'); } catch { /* the preference then lasts until restart */ }
    applyMotion(next);
    setEnabled(next);
  }
  return [enabled, change] as const;
}
