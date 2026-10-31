import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { PlayControls } from '../src/play';
import { TooltipProvider } from '../src/components/ui/tooltip';
import { LauncherUpdateProvider, GeneralPanel, VersionBadge } from '../src/update';
import type { LauncherUpdateView } from '../src/update-state';

const backend = vi.hoisted(() => ({ invoke: vi.fn(), listeners: new Map<string, Set<(event: { payload: unknown }) => void>>() }));
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, invoke: backend.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: async (name: string, callback: (event: { payload: unknown }) => void) => {
  const callbacks = backend.listeners.get(name) ?? new Set();
  callbacks.add(callback); backend.listeners.set(name, callbacks);
  return () => callbacks.delete(callback);
} }));

const view = (patch: Partial<LauncherUpdateView> = {}): LauncherUpdateView => ({
  currentVersion: '0.2.0-beta.3', channel: 'stable', phase: 'idle', pendingVersion: null, delivery: 'in_place',
  releaseUrl: null, progress: null, applyRequested: false, error: null, ...patch,
});
const required = { pendingVersion: '0.2.0-beta.4' };
let current: LauncherUpdateView;

beforeEach(() => {
  backend.listeners.clear();
  current = view();
  backend.invoke.mockImplementation(async (command: string, args?: { channel?: 'stable' | 'beta' }) => {
    if (command === 'set_update_channel') { current = { ...current, channel: args?.channel ?? 'stable' }; return current; }
    if (command === 'apply_launcher_update') { current = { ...current, applyRequested: true }; return current; }
    if (['launcher_update_status', 'check_launcher_update', 'cancel_launcher_update_apply'].includes(command)) return current;
    return undefined;
  });
});
afterEach(cleanup);

async function mount() {
  render(<TooltipProvider><LauncherUpdateProvider><VersionBadge version="0.2.0-beta.3" /><PlayControls instanceId="pack-a" allowed /><GeneralPanel /></LauncherUpdateProvider></TooltipProvider>);
  await waitFor(() => expect(backend.invoke).toHaveBeenCalledWith('launcher_update_status', undefined));
}
async function emit(next: LauncherUpdateView) {
  current = next;
  await act(async () => { backend.listeners.get('launcher-update-changed')?.forEach(callback => callback({ payload: next })); });
}
const plainVersion = () => document.querySelector('.rail-version');
const updateButton = () => document.querySelector('.rail-update') as HTMLButtonElement | null;

test('without a required update the version is plain text and Play stays', async () => {
  await mount();
  expect(plainVersion()?.textContent).toBe('v0.2.0-beta.3');
  expect(updateButton()).toBeNull();
  expect(screen.getByRole('button', { name: 'Jugar' })).toBeTruthy();
});

test('a ready update turns the version into a button that asks the backend to apply it', async () => {
  current = view({ phase: 'ready', ...required });
  await mount();
  await waitFor(() => expect(updateButton()).not.toBeNull());
  expect(updateButton()?.textContent).toBe('Actualizar');
  fireEvent.click(updateButton() as HTMLButtonElement);
  await waitFor(() => expect(backend.invoke).toHaveBeenCalledWith('apply_launcher_update', undefined));
  await waitFor(() => expect(updateButton()?.textContent).toBe('Pendiente'));
});

test('Play is disabled with a short reason, and pressing it highlights the update button for a few seconds', async () => {
  current = view({ phase: 'ready', ...required });
  await mount();
  const play = await screen.findByRole('button', { name: 'Actualización requerida' });
  expect(play.getAttribute('aria-disabled')).toBe('true');
  expect(screen.queryByRole('button', { name: 'Jugar' })).toBeNull();
  vi.useFakeTimers({ shouldAdvanceTime: true });
  try {
    expect(updateButton()?.classList.contains('is-highlighted')).toBe(false);
    fireEvent.click(play);
    await waitFor(() => expect(updateButton()?.classList.contains('is-highlighted')).toBe(true));
    // Pressing Play never starts anything or asks the backend to update by itself.
    expect(backend.invoke).not.toHaveBeenCalledWith('begin_play', expect.anything());
    expect(backend.invoke).not.toHaveBeenCalledWith('apply_launcher_update', undefined);
    await act(async () => { vi.advanceTimersByTime(4000); });
    expect(updateButton()?.classList.contains('is-highlighted')).toBe(false);
  } finally { vi.useRealTimers(); }
});

test('backend events move the badge between states and Play returns once nothing is required', async () => {
  await mount();
  await emit(view({ phase: 'downloading', progress: 30, ...required }));
  expect(updateButton()?.textContent).toBe('30 %');
  expect(updateButton()?.getAttribute('aria-disabled')).toBe('true');
  expect(screen.queryByRole('button', { name: 'Jugar' })).toBeNull();
  await emit(view());
  await waitFor(() => expect(screen.getByRole('button', { name: 'Jugar' })).toBeTruthy());
  expect(updateButton()).toBeNull();
});

test('a failed check shows a red retry icon beside the version and never blocks Play', async () => {
  current = view({ phase: 'failed', error: 'network_unavailable' });
  await mount();
  const retry = await screen.findByRole('button', { name: 'Reintentar la búsqueda de actualizaciones' });
  expect(plainVersion()?.textContent).toBe('v0.2.0-beta.3');
  expect(screen.getByRole('button', { name: 'Jugar' })).toBeTruthy();
  current = view();
  fireEvent.click(retry);
  await waitFor(() => expect(backend.invoke).toHaveBeenCalledWith('check_launcher_update', { force: true }));
  await waitFor(() => expect(screen.queryByRole('button', { name: 'Reintentar la búsqueda de actualizaciones' })).toBeNull());
});

test('a package that cannot update itself offers the release page, not a failing install', async () => {
  current = view({ phase: 'manual', delivery: 'manual', ...required });
  await mount();
  await waitFor(() => expect(updateButton()?.textContent).toBe('Ver nueva'));
  fireEvent.click(updateButton() as HTMLButtonElement);
  await waitFor(() => expect(backend.invoke).toHaveBeenCalledWith('open_launcher_release_page', undefined));
  expect(backend.invoke).not.toHaveBeenCalledWith('apply_launcher_update', undefined);
});

test('the beta channel is an explicit opt-in and is locked while an update downloads', async () => {
  await mount();
  const toggle = await screen.findByRole('switch', { name: 'Recibir versiones beta' }) as HTMLInputElement;
  expect(toggle.checked).toBe(false);
  fireEvent.click(toggle);
  await waitFor(() => expect(backend.invoke).toHaveBeenCalledWith('set_update_channel', { channel: 'beta' }));
  await waitFor(() => expect((screen.getByRole('switch', { name: 'Recibir versiones beta' }) as HTMLInputElement).checked).toBe(true));
  await emit(view({ channel: 'beta', phase: 'downloading', ...required }));
  expect((screen.getByRole('switch', { name: 'Recibir versiones beta' }) as HTMLInputElement).disabled).toBe(true);
});

test('the channel row puts the text first and the switch last, in one centered row', async () => {
  await mount();
  const row = (await screen.findByRole('switch', { name: 'Recibir versiones beta' })).closest('.setting-row') as HTMLElement;
  expect(row).not.toBeNull();
  expect(row.firstElementChild?.classList.contains('setting-text')).toBe(true);
  expect(row.lastElementChild?.getAttribute('role')).toBe('switch');
});

test('a failed command shows a Spanish message instead of the backend code', async () => {
  await mount();
  backend.invoke.mockImplementation(async (command: string) => { if (command === 'check_launcher_update') throw 'launcher_update_failed'; return current; });
  fireEvent.click(screen.getByRole('button', { name: 'Buscar actualizaciones' }));
  await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('No se pudo actualizar Panita Client'));
});
