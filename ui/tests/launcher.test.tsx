import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { Launcher } from '../src/launcher';
import { PlayProvider } from '../src/play';
import { PreferencesProvider } from '../src/settings';
import { CatalogArtwork } from '../src/components/catalog-artwork';
import type { CatalogOverview } from '../src/preview-catalog';
import type { PlayEntry, PlaySnapshot } from '../src/play-state';

const backend = vi.hoisted(() => ({ invoke: vi.fn(), listeners: new Map<string, Set<(event: { payload: unknown }) => void>>() }));
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, invoke: backend.invoke }));
vi.mock('@tauri-apps/api/app', () => ({ getVersion: async () => '0.2.0-alpha' }));
vi.mock('@tauri-apps/api/event', () => ({ listen: async (name: string, callback: (event: { payload: unknown }) => void) => {
  const callbacks = backend.listeners.get(name) ?? new Set();
  callbacks.add(callback); backend.listeners.set(name, callbacks);
  return () => callbacks.delete(callback);
} }));

let catalog: CatalogOverview;
let play: PlaySnapshot;
const first = {
  id: 'pack-a', name: 'Aventura', description: 'Un mundo para tus amigos.', minecraftVersion: '1.21.1',
  enabled: true, availability: 'enabled' as const, localState: 'installed' as const,
  installedVersion: null, hasInstallationManifest: true, compatible: true, supported: true, minLauncherVersion: null,
  source: { type: 'custom', current_version_id: 'v2', label: 'Dos', loader: 'neoforge' },
  installedSource: { type: 'custom', current_version_id: 'v2', label: 'Dos', loader: 'neoforge' },
};
const manualFile = { path: 'mods/manual.jar', name: 'Mod manual', fileName: 'manual.jar', folder: 'mods', projectId: 1, fileId: 2, size: 10, reason: 'manual' };
function entry(patch: Partial<PlayEntry>): PlayEntry {
  return { instanceId: 'pack-a', requestId: 'request', phase: 'running', operation: 'play', confirmationId: null, error: null, exitCode: null, exitSuccess: null, backupRequested: false, backupOperation: null, receivedFiles: 0, reusedFiles: 0, retainedFiles: 0, ...patch };
}
function mount() { return render(<PreferencesProvider><PlayProvider><Launcher /></PlayProvider></PreferencesProvider>); }
async function emit(entry: PlayEntry) {
  play = { ...play, revision: play.revision + 1, instances: [entry] };
  await act(async () => { backend.listeners.get('play-changed')?.forEach(callback => callback({ payload: play })); });
}

beforeEach(() => {
  catalog = { instances: [{ ...first }, { ...first, id: 'pack-b', name: 'Otra aventura' }], stale: false, cacheWarning: null, registryWarning: null };
  play = { revision: 0, processSessionId: 'test', closeBlocked: false, gracefulStopSupported: true, instances: [] };
  backend.listeners.clear();
  backend.invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
    switch (command) {
      case 'initialize': return;
      case 'catalog_snapshot': return catalog;
      case 'play_snapshot': return play;
      case 'settings_snapshot': return { instanceId: args?.instanceId ?? null, instance: null, windowWarning: null, global: { schema_version: 2, ram_mib: 4096, extra_jvm_args: [], java_path_override: null, show_console_on_start: false, window_size: { width: 1280, height: 720 }, launcher: { selected_instance_id: 'pack-a', window: null } } };
      case 'account_snapshot': return { revision: 1, selected_account_id: 'account', login_pending: false, login_error: null, accounts: [{ metadata: { account_id: 'account', kind: 'offline', username: 'Jugador', profile_id: 'profile', oauth_client_id: null }, status: 'offline', error: null, persisted: true }] };
      case 'content_cache_size': return 0;
      case 'pack_version_names': return { installed: 'Uno', available: 'Dos' };
      case 'pack_files': return { files: [manualFile], complete: false };
      case 'import_pack_file': throw 'artifact_mismatch';
      case 'select_instance': case 'begin_install': case 'cancel_play': case 'begin_update': case 'confirm_play': case 'begin_play': case 'begin_repair': case 'stop_game': case 'open_game_log': return;
      default: throw new Error(`Unexpected test command: ${command}`);
    }
  });
});
afterEach(cleanup);

test('navigation preserves account input and settings drafts, and instance settings are separate', async () => {
  const user = userEvent.setup(); mount();
  await screen.findByRole('heading', { name: 'Aventura', exact: true });
  await user.click(await screen.findByRole('button', { name: 'Cuentas: Jugador', exact: true }));
  await user.type(screen.getByLabelText('Nombre para jugar sin conexión'), 'OtroJugador');
  await user.click(screen.getByRole('button', { name: 'Inicio: tus instancias' }));
  await user.click(screen.getByRole('button', { name: 'Ajustes del launcher' }));
  const ram = await screen.findByLabelText('RAM para Minecraft (MiB)');
  await user.clear(ram); await user.type(ram, '8192');
  await user.click(screen.getByRole('button', { name: 'Inicio: tus instancias' }));
  await user.click(screen.getByRole('button', { name: 'Ajustes del launcher' }));
  expect((screen.getByLabelText('RAM para Minecraft (MiB)') as HTMLInputElement).value).toBe('8192');
  await user.click(screen.getByRole('button', { name: 'Cuentas: Jugador', exact: true }));
  expect((screen.getByLabelText('Nombre para jugar sin conexión') as HTMLInputElement).value).toBe('OtroJugador');
  await user.click(screen.getByRole('button', { name: 'Otra aventura: Lista para jugar' }));
  await screen.findByRole('heading', { name: 'Otra aventura', exact: true });
  await user.click(within(screen.getByRole('article', { name: 'Otra aventura' })).getByText('Opciones de instancia', { selector: 'summary' }));
  await user.click(screen.getByRole('button', { name: 'Ajustes de la instancia', exact: true }));
  expect(screen.getByRole('heading', { name: 'Otra aventura', exact: true })).toBeTruthy();
  expect(screen.queryByRole('heading', { name: 'Caché de archivos de modpacks' })).toBeNull();
});

test('mandatory pack updates replace Play and keep explicit world-backup consent across navigation', async () => {
  catalog.instances[0] = { ...first, localState: 'update_available', installedSource: { ...first.installedSource, current_version_id: 'v1' } };
  const user = userEvent.setup(); mount();
  const update = await screen.findByRole('button', { name: 'Actualizar…' });
  expect(screen.queryByRole('button', { name: 'Jugar', exact: true })).toBeNull();
  await user.click(update);
  await screen.findByText('Actualizar el modpack de Uno a Dos');
  await user.click(screen.getByRole('button', { name: 'Ajustes del launcher' }));
  await user.click(screen.getByRole('button', { name: 'Inicio: tus instancias' }));
  expect((screen.getByLabelText('Crear una copia de mis mundos antes de actualizar (recomendado)') as HTMLInputElement).checked).toBe(true);
  expect(backend.invoke.mock.calls.some(([command]) => command === 'begin_update')).toBe(false);
  await user.click(screen.getByRole('button', { name: 'Confirmar actualización' }));
  expect(backend.invoke).toHaveBeenCalledWith('begin_update', expect.objectContaining({ instanceId: 'pack-a', update: { from_version: 'v1', to_version: 'v2', backup_worlds: true } }));
});

test('outage launches require explicit consent and retain the backend confirmation identity', async () => {
  catalog.stale = true;
  const user = userEvent.setup(); mount();
  await screen.findByRole('heading', { name: 'Aventura', exact: true });
  await emit(entry({ phase: 'warning', confirmationId: 'consent' }));
  expect(screen.getByText(/esta instalación puede estar desactualizada/)).toBeTruthy();
  expect(backend.invoke.mock.calls.some(([command]) => command === 'confirm_play')).toBe(false);
  await user.click(screen.getByRole('button', { name: 'Jugar de todos modos' }));
  expect(backend.invoke).toHaveBeenCalledWith('confirm_play', { instanceId: 'pack-a', requestId: 'request', confirmationId: 'consent' });
});

test('manual imports report wrong files without enabling Play', async () => {
  catalog.instances[0] = { ...first, localState: 'awaiting_files' };
  const user = userEvent.setup(); mount();
  await screen.findByText('Mod manual');
  expect((screen.getByRole('button', { name: 'No disponible para jugar' }) as HTMLButtonElement).disabled).toBe(true);
  await user.click(screen.getByRole('button', { name: 'Seleccionar archivo…' }));
  await screen.findByText('El archivo no es la versión esperada.');
  expect(backend.invoke).toHaveBeenCalledWith('import_pack_file', { instanceId: 'pack-a', path: 'mods/manual.jar' });
});

test('installation shows byte progress, cancellation and an explicit retry after failure', async () => {
  catalog.instances[0] = { ...first, localState: 'not_installed', installedSource: null, hasInstallationManifest: false };
  const user = userEvent.setup(); mount();
  await user.click(await screen.findByRole('button', { name: 'Instalar', exact: true }));
  expect(backend.invoke).toHaveBeenCalledWith('begin_install', expect.objectContaining({ instanceId: 'pack-a' }));
  await emit(entry({ phase: 'installing', operation: 'install', packProgress: { filesDone: 2, filesTotal: 8, bytesDone: 1024, bytesTotal: 4096, reusedFiles: 0 } }));
  expect(screen.getByRole('progressbar', { name: 'Descarga de los archivos del modpack' }).getAttribute('value')).toBe('0.25');
  expect(screen.getByText('2 de 8 archivos (1 KB de 4 KB).')).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'Cancelar instalación' }));
  expect(backend.invoke).toHaveBeenCalledWith('cancel_play', { instanceId: 'pack-a', requestId: 'request' });
  await emit(entry({ phase: 'failed', operation: 'install', error: 'artifact_mismatch' }));
  await screen.findByText('No pudimos completar la instalación.');
  expect(screen.getByRole('button', { name: 'Reintentar instalación' })).toBeTruthy();
});

test('running controls survive navigation and force stop never happens without confirmation', async () => {
  const user = userEvent.setup(); mount();
  await screen.findByRole('heading', { name: 'Aventura', exact: true });
  await emit(entry({}));
  await user.click(screen.getByRole('button', { name: 'Otra aventura: Lista para jugar' }));
  await user.click(screen.getByRole('button', { name: 'Aventura: En ejecución' }));
  await user.click(screen.getByRole('button', { name: 'Ver registro de Minecraft' }));
  expect(backend.invoke).toHaveBeenCalledWith('open_game_log', { instanceId: 'pack-a', requestId: 'request' });
  await user.click(screen.getByRole('button', { name: 'Cerrar forzosamente…' }));
  expect(backend.invoke.mock.calls.some(([command]) => command === 'stop_game')).toBe(false);
  await user.click(screen.getByRole('button', { name: 'Volver', exact: true }));
  expect(screen.queryByRole('button', { name: 'Confirmar cierre forzoso' })).toBeNull();
});

test('update conflicts require explicit set-aside consent before retrying', async () => {
  catalog.instances[0] = { ...first, localState: 'update_available', installedSource: { ...first.installedSource, current_version_id: 'v1' } };
  const user = userEvent.setup(); mount();
  await screen.findByRole('heading', { name: 'Aventura', exact: true });
  await emit(entry({ phase: 'failed', operation: 'update', error: 'pack_conflict', packConflicts: [{ path: 'mods/player.jar', reason: 'occupied' }] }));
  expect(screen.getByText('mods/player.jar')).toBeTruthy();
  expect(backend.invoke.mock.calls.some(([command]) => command === 'begin_update')).toBe(false);
  await user.click(screen.getByRole('button', { name: 'Continuar y guardar mis archivos' }));
  expect(backend.invoke).toHaveBeenCalledWith('begin_update', expect.objectContaining({ update: { from_version: 'v1', to_version: 'v2', backup_worlds: true, move_aside: true } }));
});

test('missing and failed artwork use a local fallback without removing controls', async () => {
  const { container, rerender } = render(<CatalogArtwork url="https://cdn.example.com/icon.png" />);
  fireEvent.error(container.querySelector('img')!);
  await waitFor(() => expect(container.querySelector('svg')).not.toBeNull());
  expect(container.querySelector('img')).toBeNull();
  rerender(<CatalogArtwork url="https://cdn.example.com/new.png" />);
  expect(container.querySelector('img')?.getAttribute('referrerpolicy')).toBe('no-referrer');
});
