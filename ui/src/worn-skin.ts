import { useEffect, useSyncExternalStore } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { loadWornSkins } from './skin-api';
import type { SkinModel, SkinsView } from './skin-state';

/**
 * The skin each Microsoft profile wears, as the launcher last knew it. The accounts page and the rail draw their
 * pictures from it with the launcher's own renderer, so no online picture service is asked and nothing flashes from
 * one picture to another. One local read seeds every saved profile; the Aspectos page publishes each view it gets and
 * the background synchronisation announces its own, so a change shows up at once everywhere.
 */
export type WornSkin = { texture: string; model: SkinModel };

/** `undefined` while the launcher has not read what it knows yet, `null` for a profile with no skin of its own. */
export type WornState = WornSkin | null | undefined;

const worn = new Map<string, WornSkin | null>();
const listeners = new Set<() => void>();
let settled = false;
let seeding = false;
let listening = false;

const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; };
const notify = () => listeners.forEach(listener => listener());

function put(profileId: string, next: WornSkin | null): boolean {
  const previous = worn.get(profileId);
  if (worn.has(profileId) && previous?.texture === next?.texture && previous?.model === next?.model) return false;
  worn.set(profileId, next);
  return true;
}

/** Record the skin a view says its profile wears. A profile without a skin of its own is remembered as having none. */
export function publishSkinsView(view: SkinsView): void {
  const next: WornSkin | null = view.current?.custom && view.current.texture ? { texture: view.current.texture, model: view.current.model } : null;
  if (put(view.profileId, next)) notify();
}

function seed() {
  if (seeding || settled) return;
  seeding = true;
  void loadWornSkins()
    .then(entries => entries.forEach(entry => put(entry.profileId, entry.texture ? { texture: entry.texture, model: entry.model } : null)))
    // Whatever the outcome, an account the launcher knows nothing about is drawn with the default skin from now on.
    .catch(() => {})
    .finally(() => { settled = true; seeding = false; notify(); });
}

function listenForChanges() {
  if (listening) return;
  listening = true;
  void listen<SkinsView>('skins-changed', event => publishSkinsView(event.payload)).catch(() => { listening = false; });
}

/**
 * The cached skin of a profile: a skin, `null` when it wears none of its own or nothing is known about it once the
 * launcher has looked, and `undefined` only while it still looks. Never asks a service.
 */
export function useWornSkin(profileId: string | null): WornState {
  const state = useSyncExternalStore(subscribe, () => (profileId === null ? null : worn.has(profileId) ? worn.get(profileId) : settled ? null : undefined));
  useEffect(() => {
    if (profileId === null || !isTauri()) return;
    listenForChanges();
    seed();
  }, [profileId]);
  return state;
}

/** For tests: forget everything. */
export function resetWornSkins(): void {
  worn.clear(); settled = false; seeding = false; listening = false; notify();
}
