import { act, cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { AccountAvatar } from '../src/components/account-avatar';
import { AccountBust } from '../src/components/profile-artwork';
import { publishSkinsView, resetWornSkins } from '../src/worn-skin';
import type { SkinsView } from '../src/skin-state';

type Entry = { profileId: string; texture: string | null; model: 'classic' | 'slim' };
const backend = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Set<(event: { payload: unknown }) => void>(),
  requests: [] as { key: string; skin: string; frame?: { zoom: number } }[],
}));
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, invoke: backend.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: async (_: string, callback: (event: { payload: unknown }) => void) => { backend.listeners.add(callback); return () => backend.listeners.delete(callback); } }));
vi.mock('../src/skin-renderer', () => ({
  BUST_FRAME: { width: 256, height: 256, zoom: 1.7, lift: -7, pitch: 0.2, spread: 0.1, fov: 37 },
  cachedThumbnail: () => undefined,
  renderThumbnail: async (request: { key: string; skin: string; frame?: { zoom: number } }) => { backend.requests.push({ key: request.key, skin: request.skin, frame: request.frame }); return `data:image/png;base64,${btoa(request.key)}`; },
}));

const hash = (letter: string) => letter.repeat(64);
const metadata = (kind: 'microsoft' | 'offline', profile = 'p1') => ({ account_id: `acc-${profile}`, kind, username: 'Aerosmith_', profile_id: profile, oauth_client_id: null });
const view = (texture: string | null, profile = 'p1'): SkinsView => ({
  profileId: profile, profileName: 'Aerosmith_', current: texture ? { custom: true, texture, model: 'classic', cape: null } : { custom: false, texture: null, model: 'classic', cape: null },
  capes: [], library: [], matchingLook: null, syncedMs: 1, incomplete: false,
});
let worn: Entry[] | 'fail';
let hold: Promise<void> | null;
const textureCalls = () => backend.invoke.mock.calls.filter(([command]) => command === 'skin_texture').map(([, args]) => (args as { key: string }).key);
const wornCalls = () => backend.invoke.mock.calls.filter(([command]) => command === 'skins_worn');
const anyOnlineImage = (container: HTMLElement) => container.querySelector('img[src^="http"]');

beforeEach(() => {
  resetWornSkins();
  backend.listeners.clear(); backend.requests = [];
  worn = [{ profileId: 'p1', texture: hash('a'), model: 'classic' }];
  hold = null;
  backend.invoke.mockReset();
  backend.invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
    if (command === 'skins_worn') { if (hold) await hold; if (worn === 'fail') throw 'storage_unavailable'; return worn; }
    if (command === 'skin_texture') return new TextEncoder().encode(String(args?.key)).buffer;
    throw new Error(`Unexpected test command: ${command}`);
  });
});
afterEach(cleanup);

test('the rail face is empty until the launcher has looked, then drawn from the cached skin, and follows a new one at once', async () => {
  let release = () => {};
  hold = new Promise(resolve => { release = resolve; });
  const { container } = render(<AccountAvatar metadata={metadata('microsoft')} />);
  // While it looks, nothing is shown from the network or anywhere else.
  await waitFor(() => expect(wornCalls()).toHaveLength(1));
  expect(container.querySelector('img')).toBeNull();
  expect(container.querySelector('canvas')).toBeNull();
  expect(container.querySelector('span.account-avatar')).toBeTruthy();
  await act(async () => { release(); });
  await waitFor(() => expect(textureCalls()).toContain(hash('a')));
  expect(container.querySelector('canvas.account-avatar')).toBeTruthy();
  expect(anyOnlineImage(container)).toBeNull();
  await waitFor(() => expect(backend.listeners.size).toBe(1));
  // The background synchronisation announces a changed skin.
  await act(async () => { backend.listeners.forEach(callback => callback({ payload: view(hash('b')) })); });
  await waitFor(() => expect(textureCalls()).toContain(hash('b')));
  // Publishing the same skin again changes nothing.
  const before = textureCalls().length;
  await act(async () => { publishSkinsView(view(hash('b'))); });
  expect(textureCalls()).toHaveLength(before);
});

test('an offline account is never looked up, and a failed read draws the default skin instead of asking a service', async () => {
  const offline = render(<AccountAvatar metadata={metadata('offline')} />);
  expect(wornCalls()).toHaveLength(0);
  expect(offline.container.querySelector('canvas')).toBeNull();
  expect(anyOnlineImage(offline.container)).toBeNull();
  cleanup();
  worn = 'fail';
  const failed = render(<AccountAvatar metadata={metadata('microsoft')} />);
  await waitFor(() => expect(wornCalls()).toHaveLength(1));
  await waitFor(() => expect(failed.container.querySelector('canvas.account-avatar')).toBeTruthy());
  expect(textureCalls()).toEqual([]);
  expect(anyOnlineImage(failed.container)).toBeNull();
});

test('a profile the launcher knows nothing about, or that wears no skin of its own, is drawn with the default skin', async () => {
  worn = [{ profileId: 'p1', texture: null, model: 'classic' }];
  const { container } = render(<><AccountBust kind="microsoft" profileId="p1" active /><AccountBust kind="microsoft" profileId="never-synced" active /></>);
  await waitFor(() => expect(container.querySelectorAll('img.profile-bust')).toHaveLength(2));
  expect(backend.requests).toHaveLength(2);
  expect(backend.requests.every(request => request.key.includes('asset:steve'))).toBe(true);
  expect(textureCalls()).toEqual([]);
  expect(anyOnlineImage(container)).toBeNull();
});

test('the bust of an account is drawn from its cached skin with the bust framing, and again when the skin changes', async () => {
  const { container } = render(<AccountBust kind="microsoft" profileId="p1" active />);
  await waitFor(() => expect(container.querySelector('img.profile-bust')!.getAttribute('src')).toBe(`data:image/png;base64,${btoa(backend.requests[0].key)}`));
  expect(backend.requests[0].key).toContain(`cached:${hash('a')}`);
  expect(backend.requests[0].frame!.zoom).toBeGreaterThan(1);
  await act(async () => { publishSkinsView(view(hash('b'))); });
  await waitFor(() => expect(backend.requests.at(-1)!.key).toContain(`cached:${hash('b')}`));
  expect(anyOnlineImage(container)).toBeNull();
});

test('a no-premium account gets the default skin without any lookup of a profile', async () => {
  const { container } = render(<AccountBust kind="offline" profileId="local-1" active />);
  await waitFor(() => expect(container.querySelector('img.profile-bust')).toBeTruthy());
  expect(backend.requests).toHaveLength(1);
  expect(backend.requests[0].key).toContain('asset:steve');
  expect(wornCalls()).toHaveLength(0);
  expect(textureCalls()).toEqual([]);
});

test('a hidden accounts page draws nothing, and draws once it is shown', async () => {
  const { container, rerender } = render(<AccountBust kind="microsoft" profileId="p1" active={false} />);
  await waitFor(() => expect(wornCalls()).toHaveLength(1));
  await act(async () => { await Promise.resolve(); });
  expect(backend.requests).toHaveLength(0);
  expect(container.querySelector('img')).toBeNull();
  expect(container.querySelector('.profile-bust-pending')).toBeTruthy();
  rerender(<AccountBust kind="microsoft" profileId="p1" active />);
  await waitFor(() => expect(container.querySelector('img.profile-bust')).toBeTruthy());
  expect(backend.requests).toHaveLength(1);
});
