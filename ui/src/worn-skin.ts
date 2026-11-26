import { useEffect, useSyncExternalStore } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { loadSkins } from './skin-api';
import type { SkinModel, SkinsView } from './skin-state';

/**
 * The skin each Microsoft profile wears, as the launcher last knew it. The accounts page and the rail
 * draw their pictures from the cached texture instead of asking an online render service, which keeps
 * showing the old skin for a while after it changes. The Aspectos page publishes every view it gets
 * and the background synchronisation announces its own, so a change shows up at once everywhere.
 */
export type WornSkin = { texture: string; model: SkinModel };

const worn = new Map<string, WornSkin | null>();
const listeners = new Set<() => void>();
const requested = new Set<string>();
let listening = false;

const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; };
const notify = () => listeners.forEach(listener => listener());

/** Record the skin a view says its profile wears. A profile without a skin of its own is remembered as having none. */
export function publishSkinsView(view: SkinsView): void {
  const next: WornSkin | null = view.current?.custom && view.current.texture ? { texture: view.current.texture, model: view.current.model } : null;
  const previous = worn.get(view.profileId);
  if (previous !== undefined && previous?.texture === next?.texture && previous?.model === next?.model) return;
  worn.set(view.profileId, next);
  notify();
}

function listen_() {
  if (listening || !isTauri()) return;
  listening = true;
  void listen<SkinsView>('skins-changed', event => publishSkinsView(event.payload)).catch(() => { listening = false; });
}

/** The cached skin of a profile, or none while nothing is known about it. Reading asks the backend once for the selected account. */
export function useWornSkin(profileId: string | null, selected: boolean): WornSkin | null {
  const known = useSyncExternalStore(subscribe, () => (profileId ? worn.get(profileId) : undefined));
  useEffect(() => {
    if (!profileId || !isTauri()) return;
    listen_();
    if (!selected || known !== undefined || requested.has(profileId)) return;
    requested.add(profileId);
    // Only the selected account has a local view to read; an error just leaves the online picture.
    void loadSkins().then(publishSkinsView).catch(() => { requested.delete(profileId); });
  }, [profileId, selected, known]);
  return known ?? null;
}

/** For tests: forget everything. */
export function resetWornSkins(): void {
  worn.clear(); requested.clear(); listening = false; notify();
}
