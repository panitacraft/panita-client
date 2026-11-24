import { useEffect, useState } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import type { SystemResources } from './variant-state';

/** Memory and free disk space; null outside the app or when they cannot be read. */
export function loadSystemResources(): Promise<SystemResources | null> {
  if (!isTauri()) return Promise.resolve(null);
  return invoke<SystemResources>('system_resources').catch(() => null);
}

/**
 * The machine's resources, read again each time the caller appears, since free space
 * changes. Null until they arrive and when they cannot be read: the recommendation then
 * falls back to Standard and nothing else is blocked.
 */
export function useSystemResources(): SystemResources | null {
  const [resources, setResources] = useState<SystemResources | null>(null);
  useEffect(() => {
    let current = true;
    void loadSystemResources().then(next => { if (current) setResources(next); });
    return () => { current = false; };
  }, []);
  return resources;
}
