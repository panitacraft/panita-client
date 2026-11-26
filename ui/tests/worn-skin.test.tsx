import { act, cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { AccountAvatar } from '../src/components/account-avatar';
import { ProfileArtwork } from '../src/components/profile-artwork';
import { publishSkinsView, resetWornSkins } from '../src/worn-skin';
import type { SkinsView } from '../src/skin-state';

const backend = vi.hoisted(() => ({ invoke: vi.fn(), listeners: new Set<(event: { payload: unknown }) => void>(), requests: [] as { key: string; frame?: { zoom: number } }[] }));
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, invoke: backend.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: async (_: string, callback: (event: { payload: unknown }) => void) => { backend.listeners.add(callback); return () => backend.listeners.delete(callback); } }));
vi.mock('../src/skin-renderer', () => ({
  BUST_FRAME: { width: 192, height: 216, zoom: 2.3, lift: -10 },
  cachedThumbnail: () => undefined,
  renderThumbnail: async (request: { key: string; frame?: { zoom: number } }) => { backend.requests.push({ key: request.key, frame: request.frame }); return `data:image/png;base64,${btoa(request.key)}`; },
}));

const hash = (letter: string) => letter.repeat(64);
const metadata = (kind: 'microsoft' | 'offline') => ({ account_id: 'acc', kind, username: 'Aerosmith_', profile_id: 'p1', oauth_client_id: null });
const view = (texture: string | null): SkinsView => ({
  profileId: 'p1', profileName: 'Aerosmith_', current: texture ? { custom: true, texture, model: 'classic', cape: null } : { custom: false, texture: null, model: 'classic', cape: null },
  capes: [], library: [], matchingLook: null, syncedMs: 1, incomplete: false,
});
let snapshot: SkinsView | 'fail';
const textureCalls = () => backend.invoke.mock.calls.filter(([command]) => command === 'skin_texture').map(([, args]) => (args as { key: string }).key);
const snapshotCalls = () => backend.invoke.mock.calls.filter(([command]) => command === 'skins_snapshot');

beforeEach(() => {
  resetWornSkins();
  backend.listeners.clear(); backend.requests = [];
  snapshot = view(hash('a'));
  backend.invoke.mockReset();
  backend.invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
    if (command === 'skins_snapshot') { if (snapshot === 'fail') throw 'storage_unavailable'; return snapshot; }
    if (command === 'skin_texture') return new TextEncoder().encode(String(args?.key)).buffer;
    throw new Error(`Unexpected test command: ${command}`);
  });
});
afterEach(cleanup);

test('the rail avatar of a Microsoft account is drawn from its cached skin, and follows a new one at once', async () => {
  const { container } = render(<AccountAvatar metadata={metadata('microsoft')} />);
  await waitFor(() => expect(textureCalls()).toContain(hash('a')));
  expect(snapshotCalls()).toHaveLength(1);
  // The online picture stands in only until the cached one is drawn.
  expect(container.querySelector('canvas.account-avatar')).toBeTruthy();
  await waitFor(() => expect(backend.listeners.size).toBe(1));
  // The background synchronisation announces a changed skin.
  await act(async () => { backend.listeners.forEach(callback => callback({ payload: view(hash('b')) })); });
  await waitFor(() => expect(textureCalls()).toContain(hash('b')));
  // Publishing the same skin again changes nothing.
  const before = textureCalls().length;
  await act(async () => { publishSkinsView(view(hash('b'))); });
  expect(textureCalls()).toHaveLength(before);
});

test('an offline account is never looked up, and a failed lookup leaves the online picture', async () => {
  const offline = render(<AccountAvatar metadata={metadata('offline')} />);
  expect(snapshotCalls()).toHaveLength(0);
  expect(offline.container.querySelector('canvas')).toBeNull();
  cleanup();
  snapshot = 'fail';
  const failed = render(<AccountAvatar metadata={metadata('microsoft')} />);
  await waitFor(() => expect(snapshotCalls()).toHaveLength(1));
  expect(failed.container.querySelector('canvas')).toBeNull();
  expect(failed.container.querySelector('img.account-avatar')!.getAttribute('src')).toContain('/2d/head/Aerosmith_');
});

test('an account that wears no skin of its own keeps the online picture', async () => {
  snapshot = view(null);
  const { container } = render(<AccountAvatar metadata={metadata('microsoft')} />);
  await waitFor(() => expect(snapshotCalls()).toHaveLength(1));
  expect(container.querySelector('canvas')).toBeNull();
  expect(textureCalls()).toEqual([]);
});

test('the bust beside an account in the accounts list is a 3D picture of the worn skin, and renews with it', async () => {
  const { container } = render(<ProfileArtwork username="Aerosmith_" bust profileId="p1" selected />);
  // Until the picture exists the online bust stands in.
  expect(container.querySelector('img.profile-bust')!.getAttribute('src')).toContain('/3d/bust/Aerosmith_');
  await waitFor(() => expect(container.querySelector('img.profile-bust')!.getAttribute('src')).toBe(`data:image/png;base64,${btoa(`bust|${hash('a')}|classic`)}`));
  expect(backend.requests.at(-1)!.frame!.zoom).toBeGreaterThan(1);
  await act(async () => { publishSkinsView(view(hash('b'))); });
  await waitFor(() => expect(container.querySelector('img.profile-bust')!.getAttribute('src')).toBe(`data:image/png;base64,${btoa(`bust|${hash('b')}|classic`)}`));
});

test('rows that are not the selected account use the cached skin only once it is known, and heads stay online', async () => {
  const other = render(<ProfileArtwork username="Otro" bust profileId="p2" selected={false} />);
  expect(snapshotCalls()).toHaveLength(0);
  expect(other.container.querySelector('img.profile-bust')!.getAttribute('src')).toContain('/3d/bust/Otro');
  cleanup();
  const head = render(<ProfileArtwork username="Aerosmith_" profileId="p1" selected />);
  expect(snapshotCalls()).toHaveLength(0);
  expect(head.container.querySelector('img.profile-head')!.getAttribute('src')).toContain('/2d/head/Aerosmith_');
});
