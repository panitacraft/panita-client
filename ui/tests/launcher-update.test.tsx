import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { PlayControls } from '../src/play';
import { LauncherUpdateNotice, LauncherUpdateProvider, UpdatesPanel } from '../src/update';
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
  render(<LauncherUpdateProvider><LauncherUpdateNotice /><PlayControls instanceId="pack-a" allowed /><UpdatesPanel /></LauncherUpdateProvider>);
  await waitFor(() => expect(backend.invoke).toHaveBeenCalledWith('launcher_update_status', undefined));
}
async function emit(next: LauncherUpdateView) {
  current = next;
  await act(async () => { backend.listeners.get('launcher-update-changed')?.forEach(callback => callback({ payload: next })); });
}

test('without a required update Play stays and no notice is shown', async () => {
  await mount();
  expect(screen.getByRole('button', { name: 'Jugar' })).toBeTruthy();
  expect(screen.queryByRole('alert')).toBeNull();
});

test('a required update replaces Play with an update action that asks the backend to apply it', async () => {
  current = view({ phase: 'ready', ...required });
  await mount();
  await waitFor(() => expect(screen.getByRole('button', { name: 'Actualizar Panita Client' })).toBeTruthy());
  expect(screen.queryByRole('button', { name: 'Jugar' })).toBeNull();
  expect(screen.getByRole('alert').textContent).toContain('0.2.0-beta.4');
  fireEvent.click(screen.getByRole('button', { name: 'Actualizar ahora' }));
  await waitFor(() => expect(backend.invoke).toHaveBeenCalledWith('apply_launcher_update', undefined));
  await waitFor(() => expect(screen.getByRole('button', { name: 'Actualización pendiente…' }).getAttribute('aria-disabled')).toBe('true'));
});

test('backend events move the launcher between states and Play returns once nothing is required', async () => {
  await mount();
  await emit(view({ phase: 'downloading', progress: 30, ...required }));
  expect(screen.getByText(/Descargando Panita Client 0\.2\.0-beta\.4 \(30 %\)/)).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Jugar' })).toBeNull();
  await emit(view());
  await waitFor(() => expect(screen.getByRole('button', { name: 'Jugar' })).toBeTruthy());
});

test('an unavailable check only warns and never blocks Play', async () => {
  current = view({ phase: 'failed', error: 'network_unavailable' });
  await mount();
  await waitFor(() => expect(screen.getByText(/Puedes seguir jugando/)).toBeTruthy());
  expect(screen.getByRole('button', { name: 'Jugar' })).toBeTruthy();
});

test('a package that cannot update itself offers the release page, not a failing install', async () => {
  current = view({ phase: 'manual', delivery: 'manual', ...required });
  await mount();
  await waitFor(() => expect(screen.getAllByRole('button', { name: 'Ver nueva versión' }).length).toBeGreaterThan(0));
  fireEvent.click(screen.getAllByRole('button', { name: 'Ver nueva versión' })[0]);
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

test('a failed command shows a Spanish message instead of the backend code', async () => {
  await mount();
  backend.invoke.mockImplementation(async (command: string) => { if (command === 'check_launcher_update') throw 'launcher_update_failed'; return current; });
  fireEvent.click(screen.getByRole('button', { name: 'Buscar actualizaciones' }));
  await waitFor(() => expect(screen.getAllByRole('alert')[0].textContent).toContain('No se pudo actualizar Panita Client'));
});
