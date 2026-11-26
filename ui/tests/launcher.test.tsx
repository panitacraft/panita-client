import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { Launcher } from '../src/launcher';
import { PlayProvider } from '../src/play';
import { PreferencesProvider } from '../src/settings';
import { CatalogArtwork } from '../src/components/catalog-artwork';
import logoWhite from '../src/assets/logo-white.svg';
import { AccountAvatar, avatarUrl } from '../src/components/account-avatar';
import { InstanceBadges } from '../src/components/instance-badges';
import type { CatalogOverview } from '../src/preview-catalog';
import type { PlayEntry, PlaySnapshot } from '../src/play-state';

const backend = vi.hoisted(() => ({ native: true, invoke: vi.fn(), listeners: new Map<string, Set<(event: { payload: unknown }) => void>>() }));
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => backend.native, invoke: backend.invoke }));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ onResized: async () => () => {} }) }));
vi.mock('@tauri-apps/api/app', () => ({ getVersion: async () => '0.2.0-alpha' }));
vi.mock('@tauri-apps/api/event', () => ({ listen: async (name: string, callback: (event: { payload: unknown }) => void) => {
  const callbacks = backend.listeners.get(name) ?? new Set();
  callbacks.add(callback); backend.listeners.set(name, callbacks);
  return () => callbacks.delete(callback);
} }));

let catalog: CatalogOverview;
let play: PlaySnapshot;
let accounts: { revision: number; selected_account_id: string; login_pending: boolean; login_error: null; accounts: { metadata: { account_id: string; kind: 'offline' | 'microsoft'; username: string; profile_id: string; oauth_client_id: null }; status: string; error: null; persisted: boolean }[] };
const first = {
  id: 'pack-a', name: 'Aventura', description: 'Un mundo para tus amigos.', minecraftVersion: '1.21.1',
  enabled: true, availability: 'enabled' as const, localState: 'installed' as const,
  installedVersion: null, hasInstallationManifest: true, compatible: true, supported: true, minLauncherVersion: null,
  source: { type: 'custom', current_version_id: 'v2', label: 'Dos', loader: 'neoforge' },
  installedSource: { type: 'custom', current_version_id: 'v2', label: 'Dos', loader: 'neoforge' },
};
const variantOptions = [
  { id: 'lite', fileCount: 12, size: 48_200_000, sameAsPrevious: false },
  { id: 'standard', fileCount: 28, size: 128_307_508, sameAsPrevious: false },
  { id: 'deluxe', fileCount: 35, size: 190_000_000, sameAsPrevious: false },
];
let changelog: string | null = null;
let namesGate: Promise<void> | undefined;
let systemResources: { memoryMib: number | null; freeDiskMib: number | null } | null = null;
let settingsRamAuto: boolean | undefined;
let ramPreview: Record<string, unknown> | null = null;
const manualFile ={ path: 'mods/manual.jar', name: 'Mod manual', fileName: 'manual.jar', folder: 'mods', projectId: 1, fileId: 2, size: 10, reason: 'manual' };
function entry(patch: Partial<PlayEntry>): PlayEntry {
  return { instanceId: 'pack-a', requestId: 'request', phase: 'running', operation: 'play', confirmationId: null, error: null, exitCode: null, exitSuccess: null, backupRequested: false, backupOperation: null, receivedFiles: 0, reusedFiles: 0, retainedFiles: 0, ...patch };
}
function mount() { return render(<PreferencesProvider><PlayProvider><Launcher /></PlayProvider></PreferencesProvider>); }
async function emit(entry: PlayEntry) {
  play = { ...play, revision: play.revision + 1, instances: [entry] };
  await act(async () => { backend.listeners.get('play-changed')?.forEach(callback => callback({ payload: play })); });
}

beforeEach(() => {
  backend.native = true;
  catalog = { instances: [{ ...first }, { ...first, id: 'pack-b', name: 'Otra aventura' }], stale: false, cacheWarning: null, registryWarning: null };
  play = { revision: 0, processSessionId: 'test', closeBlocked: false, gracefulStopSupported: true, instances: [] };
  backend.listeners.clear();
  systemResources = null; changelog = null; namesGate = undefined;
  settingsRamAuto = undefined; ramPreview = null;
  accounts = { revision: 1, selected_account_id: 'account', login_pending: false, login_error: null, accounts: [{ metadata: { account_id: 'account', kind: 'offline', username: 'Jugador', profile_id: 'profile', oauth_client_id: null }, status: 'offline', error: null, persisted: true }] };
  backend.invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
    switch (command) {
      case 'main_window_control': return false;
      case 'initialize': return;
      case 'catalog_snapshot': return catalog;
      case 'play_snapshot': return play;
      case 'settings_snapshot': return { instanceId: args?.instanceId ?? null, instance: null, windowWarning: null, global: { schema_version: 2, ram_mib: 4096, ram_auto: settingsRamAuto, extra_jvm_args: [], java_path_override: null, show_console_on_start: false, window_size: { width: 1280, height: 720 }, launcher: { selected_instance_id: 'pack-a', window: null } } };
      case 'save_global_settings': case 'save_instance_settings': return;
      case 'ram_preview': if (!ramPreview) throw 'storage_unavailable'; return ramPreview;
      case 'account_snapshot': return accounts;
      case 'select_account': accounts = { ...accounts, revision: accounts.revision + 1, selected_account_id: String(args?.accountId) }; return;
      case 'instance_folder_path': return 'C:\\Synthetic\\PanitaClient\\instances\\aventura';
      case 'begin_uninstall': return;
      case 'content_cache_size': return 0;
      case 'pack_version_names': await namesGate; return { installed: 'Uno', available: 'Dos', changelog };
      case 'pack_files': return { files: [manualFile], complete: false };
      case 'import_pack_file': throw 'artifact_mismatch';
      case 'instance_variants': return variantOptions;
      case 'list_screenshots': return { screenshots: [], truncated: false };
      case 'system_resources': if (!systemResources) throw 'storage_unavailable'; return systemResources;
      case 'select_instance': case 'begin_install': case 'cancel_play': case 'begin_update': case 'confirm_play': case 'begin_play': case 'begin_repair': case 'stop_game': case 'open_game_log': case 'open_instance_folder': case 'open_curseforge_project': return;
      default: throw new Error(`Unexpected test command: ${command}`);
    }
  });
});
afterEach(cleanup);

test('refreshing a selected vanilla instance replaces Play with a mandatory update', async () => {
  catalog.instances[0] = { ...first, source: { type: 'vanilla' }, installedSource: null, installedVersion: '1.21.1' };
  const user = userEvent.setup(); mount();
  await screen.findByRole('button', { name: 'Jugar', exact: true });
  catalog.instances[0] = { ...catalog.instances[0], minecraftVersion: '1.21.2' };
  await user.click(screen.getByRole('button', { name: 'Actualizar catálogo' }));
  const update = await screen.findByRole('button', { name: 'Actualizar…', exact: true });
  expect(screen.queryByRole('button', { name: 'Jugar', exact: true })).toBeNull();
  expect(backend.invoke.mock.calls.some(([command]) => command === 'begin_update')).toBe(false);
  await user.click(update);
  expect(screen.getByRole('dialog', { name: 'Actualizar Minecraft' })).toBeTruthy();
  expect((screen.getByLabelText(/^Copia de seguridad de mis mundos/) as HTMLInputElement).checked).toBe(true);
});

test('the running primary action stays available and crashes use the branded dialog', async () => {
  const user = userEvent.setup(); mount();
  await screen.findByRole('heading', { name: 'Aventura', exact: true });
  await emit(entry({}));
  expect((screen.getByRole('button', { name: 'Cerrar forzosamente…', exact: true }) as HTMLButtonElement).disabled).toBe(false);
  await emit(entry({ phase: 'exited', exitSuccess: false, exitCode: -1 }));
  const dialog = await screen.findByRole('dialog', { name: 'Minecraft se cerró de forma inesperada' });
  expect(dialog.classList.contains('crash-dialog')).toBe(true);
  await user.click(within(dialog).getByRole('button', { name: 'Ver registro', exact: true }));
  expect(backend.invoke).toHaveBeenCalledWith('open_game_log', { instanceId: 'pack-a', requestId: 'request' });
});

test('navigation preserves settings drafts and account creation remains an explicit modal', async () => {
  const user = userEvent.setup(); mount();
  await screen.findByRole('heading', { name: 'Aventura', exact: true });
  await user.click(await screen.findByRole('button', { name: /^Gestionar cuenta/ }));
  await user.click(await screen.findByRole('button', { name: 'Añadir', exact: true }));
  await user.click(screen.getByRole('menuitem', { name: 'Añadir cuenta No-Premium' }));
  await user.type(await screen.findByLabelText('Nombre de usuario'), 'OtroJugador');
  await user.click(screen.getByRole('button', { name: 'Cancelar' }));
  await user.click(screen.getByRole('button', { name: 'Instancias', exact: true }));
  await user.click(screen.getByRole('button', { name: 'Ajustes Globales' }));
  const ram = await screen.findByLabelText('RAM Asignada (MiB)');
  await user.clear(ram); await user.type(ram, '8192');
  await user.click(screen.getByRole('button', { name: 'Inicio: tus instancias' }));
  await user.click(screen.getByRole('button', { name: 'Ajustes Globales' }));
  expect((screen.getByLabelText('RAM Asignada (MiB)') as HTMLInputElement).value).toBe('8192');
  await user.click(screen.getByRole('button', { name: /^Gestionar cuenta/ }));
  expect(screen.getByRole('heading', { name: 'Perfiles de Minecraft' })).toBeTruthy();
  expect(screen.queryByRole('dialog')).toBeNull();
  await user.click(screen.getByRole('button', { name: 'Otra aventura: Lista para jugar' }));
  await screen.findByRole('heading', { name: 'Otra aventura', exact: true });
  await user.click(within(screen.getByRole('article', { name: 'Otra aventura' })).getByRole('button', { name: 'Opciones de instancia' }));
  await user.click(screen.getByRole('menuitem', { name: 'Ajustes de la instancia', exact: true }));
  expect(screen.getByRole('heading', { name: 'Ajustes · Otra aventura', exact: true })).toBeTruthy();
  expect(screen.queryByRole('heading', { name: 'Caché de modpacks' })).toBeNull();
}, 15000);

test('mandatory pack updates open a modal and require explicit world-backup consent', async () => {
  catalog.instances[0] = { ...first, localState: 'update_available', installedSource: { ...first.installedSource, current_version_id: 'v1' } };
  const user = userEvent.setup(); mount();
  const update = await screen.findByRole('button', { name: 'Actualizar…' });
  expect(screen.queryByRole('button', { name: 'Jugar', exact: true })).toBeNull();
  await user.click(update);
  const dialog = await screen.findByRole('dialog', { name: 'Actualizar modpack' });
  // The header names both versions as chips, and the confirm button names the one being installed.
  await within(dialog).findByText('Uno');
  expect(within(dialog).getByText('Dos')).toBeTruthy();
  expect(within(dialog).getByRole('button', { name: 'Actualizar a Dos' })).toBeTruthy();
  expect((screen.getByLabelText(/^Copia de seguridad de mis mundos/) as HTMLInputElement).checked).toBe(true);
  expect(backend.invoke.mock.calls.some(([command]) => command === 'begin_update')).toBe(false);
  await user.click(screen.getByRole('button', { name: /^Actualizar a / }));
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
  expect(screen.getByRole('progressbar', { name: 'Descarga del modpack' }).getAttribute('value')).toBe('0.25');
  expect(screen.getByText('2 / 8 archivos')).toBeTruthy();
  await emit(entry({ phase: 'installing', operation: 'install', artifactProgress: { bytes: 512, total: 1024 } }));
  expect(screen.getByRole('progressbar', { name: 'Descarga del archivo actual' }).getAttribute('value')).toBe('512');
  await user.click(screen.getByRole('button', { name: 'Cancelar operación' }));
  expect(backend.invoke).toHaveBeenCalledWith('cancel_play', { instanceId: 'pack-a', requestId: 'request' });
  await emit(entry({ phase: 'failed', operation: 'install', error: 'artifact_mismatch' }));
  await screen.findByRole('alert');
  expect(screen.getByRole('button', { name: 'Reintentar instalación' })).toBeTruthy();
});

test('running controls survive navigation and force stop never happens without confirmation', async () => {
  const user = userEvent.setup(); mount();
  await screen.findByRole('heading', { name: 'Aventura', exact: true });
  await emit(entry({}));
  await user.click(screen.getByRole('button', { name: 'Otra aventura: Lista para jugar' }));
  await user.click(screen.getByRole('button', { name: 'Aventura: En ejecución' }));
  await user.click(screen.getByRole('button', { name: 'Cerrar forzosamente…', exact: true }));
  expect(backend.invoke.mock.calls.some(([command]) => command === 'stop_game')).toBe(false);
  await user.click(within(screen.getByRole('dialog', { name: 'Cerrar Minecraft' })).getByRole('button', { name: 'Cancelar', exact: true }));
  expect(screen.queryByRole('button', { name: 'Confirmar cierre forzoso' })).toBeNull();
});

test('update conflicts are a toast, and set-aside consent is asked in a confirmation modal', async () => {
  catalog.instances[0] = { ...first, localState: 'update_available', installedSource: { ...first.installedSource, current_version_id: 'v1' } };
  const user = userEvent.setup(); mount();
  await screen.findByRole('heading', { name: 'Aventura', exact: true });
  await emit(entry({ phase: 'failed', operation: 'update', error: 'pack_conflict', packConflicts: [{ path: 'mods/player.jar', reason: 'occupied' }] }));
  const toast = await screen.findByText('Hay 1 archivo tuyo que estorba a la actualización.');
  expect(toast.closest('.transient-notice')).not.toBeNull();
  // The list and the raw error text no longer sit in the page.
  expect(screen.queryByText('mods/player.jar')).toBeNull();
  expect(screen.queryByText(/Ya existen archivos tuyos/)).toBeNull();
  await user.click(screen.getByRole('button', { name: 'Ver detalles' }));
  const dialog = screen.getByRole('dialog', { name: 'Hay 1 archivo tuyo que estorba a la actualización' });
  expect(within(dialog).getByText('mods/player.jar')).toBeTruthy();
  expect(backend.invoke.mock.calls.some(([command]) => command === 'begin_update')).toBe(false);
  // Cancelling changes nothing and leaves the toast to come back to.
  await user.click(within(dialog).getByRole('button', { name: 'Cancelar' }));
  expect(screen.queryByRole('dialog')).toBeNull();
  expect(backend.invoke.mock.calls.some(([command]) => command === 'begin_update')).toBe(false);
  await user.click(screen.getByRole('button', { name: 'Ver detalles' }));
  await user.click(screen.getByRole('button', { name: 'Continuar y guardar mis archivos' }));
  expect(backend.invoke).toHaveBeenCalledWith('begin_update', expect.objectContaining({ update: { from_version: 'v1', to_version: 'v2', backup_worlds: true, move_aside: true } }));
});

test('the conflict toast leaves once the player starts the update it was about', async () => {
  catalog.instances[0] = { ...first, localState: 'update_available', installedSource: { ...first.installedSource, current_version_id: 'v1' } };
  const user = userEvent.setup(); mount();
  await screen.findByRole('heading', { name: 'Aventura', exact: true });
  await emit(entry({ phase: 'failed', operation: 'update', error: 'pack_conflict', packConflicts: [{ path: 'mods/player.jar', reason: 'occupied' }] }));
  await screen.findByText('Hay 1 archivo tuyo que estorba a la actualización.');
  await user.click(screen.getByRole('button', { name: 'Ver detalles' }));
  await user.click(screen.getByRole('button', { name: 'Continuar y guardar mis archivos' }));
  // Nothing has started yet, so the toast stays; it goes when the new operation does.
  expect(screen.queryByText('Hay 1 archivo tuyo que estorba a la actualización.')).not.toBeNull();
  await emit(entry({ requestId: 'again', phase: 'installing', operation: 'update' }));
  await waitFor(() => expect(screen.queryByText('Hay 1 archivo tuyo que estorba a la actualización.')).toBeNull());
  expect(screen.queryByRole('button', { name: 'Ver detalles' })).toBeNull();
});

test('a details toast of a finished operation leaves when that instance starts another one', async () => {
  mount();
  await screen.findByRole('heading', { name: 'Aventura', exact: true });
  await emit(entry({ phase: 'installed', operation: 'update', packSkipped: ['options.txt'] }));
  await screen.findByRole('button', { name: 'Ver detalles' });
  await emit(entry({ requestId: 'next', phase: 'installing', operation: 'repair' }));
  await waitFor(() => expect(screen.queryByRole('button', { name: 'Ver detalles' })).toBeNull());
});

test('modal footers read the same way: cancel first, the confirming action last', async () => {
  const labels = (dialog: HTMLElement) => [...dialog.querySelectorAll('.account-buttons > button, .update-foot > button')].map(button => button.textContent);
  catalog.instances[0] = { ...first, localState: 'update_available', installedSource: { ...first.installedSource, current_version_id: 'v1' } };
  const user = userEvent.setup(); mount();
  await screen.findByRole('heading', { name: 'Aventura', exact: true });
  // Uninstall
  await user.click(within(screen.getByRole('article', { name: 'Aventura' })).getByRole('button', { name: 'Opciones de instancia' }));
  await user.click(screen.getByRole('menuitem', { name: 'Desinstalar…' }));
  const uninstall = await screen.findByRole('dialog', { name: 'Desinstalar instancia' });
  expect(labels(uninstall)).toEqual(['Cancelar', 'Desinstalar']);
  await user.click(within(uninstall).getByRole('button', { name: 'Cancelar' }));
  // Update
  await user.click(screen.getByRole('button', { name: 'Actualizar…' }));
  const update = await screen.findByRole('dialog', { name: 'Actualizar modpack' });
  expect(labels(update)).toEqual(['Cancelar', 'Actualizar a Dos']);
  await user.click(within(update).getByRole('button', { name: 'Cancelar' }));
  // Details of an operation, and a completed one with a single action
  await emit(entry({ phase: 'failed', operation: 'update', error: 'pack_conflict', packConflicts: [{ path: 'mods/a.jar', reason: 'occupied' }] }));
  await user.click(await screen.findByRole('button', { name: 'Ver detalles' }));
  expect(labels(screen.getByRole('dialog'))).toEqual(['Cancelar', 'Continuar y guardar mis archivos']);
});

test('the completion modal keeps its single action on the right like every other footer', async () => {
  const user = userEvent.setup(); mount();
  await screen.findByRole('heading', { name: 'Aventura', exact: true });
  await emit(entry({ phase: 'installed', operation: 'update', packSkipped: ['options.txt'] }));
  await user.click(await screen.findByRole('button', { name: 'Ver detalles' }));
  const footer = screen.getByRole('dialog', { name: 'Actualización completada' }).querySelector('.update-foot');
  expect(footer?.classList.contains('is-start')).toBe(false);
});

test('conflicts that moving files aside cannot fix offer no continue button', async () => {
  const user = userEvent.setup(); mount();
  await screen.findByRole('heading', { name: 'Aventura', exact: true });
  await emit(entry({ phase: 'failed', operation: 'install', error: 'pack_conflict', packConflicts: [{ path: 'mods', reason: 'blocked' }] }));
  await user.click(await screen.findByRole('button', { name: 'Ver detalles' }));
  const dialog = screen.getByRole('dialog', { name: 'Hay 1 archivo tuyo que estorba a la instalación' });
  expect(within(dialog).queryByRole('button', { name: 'Continuar y guardar mis archivos' })).toBeNull();
  expect(within(dialog).getByText(/Mueve esa carpeta o enlace/)).toBeTruthy();
  await user.click(within(dialog).getByRole('button', { name: 'Cerrar' }));
  await user.click(screen.getByRole('button', { name: 'Cerrar aviso' }));
  expect(screen.queryByText('Hay 1 archivo tuyo que estorba a la instalación.')).toBeNull();
});

test('a required update is a toast without details instead of a block in the page', async () => {
  mount();
  await screen.findByRole('heading', { name: 'Aventura', exact: true });
  await emit(entry({ phase: 'failed', operation: 'play', error: 'update_required' }));
  const toast = await screen.findByText('Actualiza esta instancia antes de jugar.');
  expect(toast.closest('.transient-notice')).not.toBeNull();
  expect(document.querySelector('.account-error')).toBeNull();
  expect(screen.queryByRole('button', { name: 'Ver detalles' })).toBeNull();
});

test('missing manual files are a toast whose details open the download buttons', async () => {
  const user = userEvent.setup(); mount();
  await screen.findByRole('heading', { name: 'Aventura', exact: true });
  await emit(entry({ phase: 'failed', operation: 'update', error: 'manual_files_required', packAwaiting: [manualFile as never] }));
  await screen.findByText('Falta 1 archivo que debes descargar tú mismo.');
  await user.click(screen.getByRole('button', { name: 'Ver detalles' }));
  const dialog = screen.getByRole('dialog', { name: 'Falta 1 archivo que debes descargar tú mismo' });
  expect(within(dialog).getByText('Mod manual')).toBeTruthy();
  await user.click(within(dialog).getByRole('button', { name: 'Abrir CurseForge' }));
  expect(backend.invoke).toHaveBeenCalledWith('open_curseforge_project', { projectId: 1 });
  await user.click(within(dialog).getByRole('button', { name: 'Abrir carpeta' }));
  expect(backend.invoke).toHaveBeenCalledWith('open_instance_folder', { instanceId: 'pack-a', folder: 'mods' });
});

test('kept files after an update are announced once, with their list one click away', async () => {
  const user = userEvent.setup(); mount();
  await screen.findByRole('heading', { name: 'Aventura', exact: true });
  await emit(entry({ phase: 'installed', operation: 'update', packSkipped: ['options.txt', 'config/paperdoll.json'], packBackedUp: ['mods/a.jar'] }));
  await screen.findByText('Aventura: Actualización completada. Algunos de tus archivos se conservaron.');
  // The lists are not repeated in the page while the modal is closed.
  expect(screen.queryByText('options.txt')).toBeNull();
  await user.click(screen.getByRole('button', { name: 'Ver detalles' }));
  const dialog = screen.getByRole('dialog', { name: 'Actualización completada' });
  // The header says the instance is ready and which version it now has; the name is not part of the title.
  expect(within(dialog).getByText('Dos')).toBeTruthy();
  expect(within(dialog).getByText(/ya está listo para jugar\./).textContent).toBe('Aventura ya está listo para jugar.');
  // One card per kind, each with its count and rows.
  const kept = within(dialog).getByRole('group', { name: 'Archivos conservados' });
  expect(within(kept).getByText('2')).toBeTruthy();
  expect(within(kept).getByText('options.txt')).toBeTruthy();
  expect(within(kept).getByText('config/paperdoll.json')).toBeTruthy();
  const saved = within(dialog).getByRole('group', { name: 'Archivos guardados' });
  expect(within(saved).getByText('mods/a.jar')).toBeTruthy();
  await user.click(within(saved).getByRole('button', { name: 'Abrir copias de archivos' }));
  expect(backend.invoke).toHaveBeenCalledWith('open_instance_folder', { instanceId: 'pack-a', folder: 'pack-backups' });
  await user.click(within(dialog).getByRole('button', { name: 'Aceptar' }));
  expect(screen.queryByRole('dialog')).toBeNull();
  // Accepting is reading what the toast pointed to, so the toast goes with it.
  expect(screen.queryByRole('button', { name: 'Ver detalles' })).toBeNull();
  expect(screen.queryByText(/Algunos de tus archivos se conservaron/)).toBeNull();
});

test('the toast of a completed operation also closes with its own dismiss button', async () => {
  const user = userEvent.setup(); mount();
  await screen.findByRole('heading', { name: 'Aventura', exact: true });
  await emit(entry({ phase: 'installed', operation: 'update', packSkipped: ['options.txt'] }));
  await screen.findByRole('button', { name: 'Ver detalles' });
  await user.click(screen.getByRole('button', { name: 'Cerrar aviso' }));
  expect(screen.queryByRole('button', { name: 'Ver detalles' })).toBeNull();
});

test('leftover native folders after an update are listed with their reason in the same modal', async () => {
  const user = userEvent.setup(); mount();
  await screen.findByRole('heading', { name: 'Aventura', exact: true });
  await emit(entry({ phase: 'installed', operation: 'update', nativeCleanup: { cleanedGenerations: 1, retainedGenerations: 1, untrackedGenerations: 0, details: [{ directory: 'natives/old', reason: 'accessFailure' }] } }));
  await screen.findByText(/Quedaron carpetas nativas antiguas sin eliminar\./);
  await user.click(screen.getByRole('button', { name: 'Ver detalles' }));
  const natives = within(screen.getByRole('dialog', { name: 'Actualización completada' })).getByRole('group', { name: 'Carpetas nativas antiguas' });
  expect(within(natives).getByText('natives/old')).toBeTruthy();
  expect(within(natives).getByText(/No se pudo eliminar/)).toBeTruthy();
});

test('an uninstall that left something behind says so without a version', async () => {
  const user = userEvent.setup(); mount();
  await screen.findByRole('heading', { name: 'Aventura', exact: true });
  await emit(entry({ phase: 'uninstalled', operation: 'uninstall', nativeCleanup: { cleanedGenerations: 0, retainedGenerations: 1, untrackedGenerations: 0, details: [{ directory: 'natives/old', reason: 'accessFailure' }] } }));
  await user.click(await screen.findByRole('button', { name: 'Ver detalles' }));
  const dialog = screen.getByRole('dialog', { name: 'Instancia desinstalada' });
  expect(within(dialog).getByText(/se desinstaló\./)).toBeTruthy();
  expect(dialog.querySelector('.version-chip')).toBeNull();
});

test('a completed operation with nothing to explain keeps the plain toast', async () => {
  mount();
  await screen.findByRole('heading', { name: 'Aventura', exact: true });
  await emit(entry({ phase: 'installed', operation: 'update' }));
  await screen.findByText('Aventura: Actualización completada.');
  expect(screen.queryByRole('button', { name: 'Ver detalles' })).toBeNull();
});

test('a cancelled installation leaves no status chip beside the instance badges', async () => {
  catalog.instances[0] = { ...first, localState: 'cancelled', installedSource: null, hasInstallationManifest: false };
  mount();
  await screen.findByRole('heading', { name: 'Aventura', exact: true });
  const hero = screen.getByRole('article', { name: 'Aventura' });
  expect(within(hero).queryByText('Operación cancelada')).toBeNull();
  expect(hero.querySelector('.instance-status')).toBeNull();
  expect(within(hero).getByRole('button', { name: 'Instalar', exact: true })).toBeTruthy();
});

test('the update prompt shows the new version\'s release notes as plain text', async () => {
  changelog = '- Nuevas misiones\n- <b>Menos</b> lag';
  catalog.instances[0] = { ...first, localState: 'update_available', installedSource: { ...first.installedSource, current_version_id: 'v1' } };
  const user = userEvent.setup(); mount();
  await user.click(await screen.findByRole('button', { name: 'Actualizar…' }));
  const notes = await screen.findByLabelText('Notas de la versión');
  expect(notes.textContent).toBe(changelog);
  expect(notes.querySelector('b')).toBeNull();
  expect(screen.getByRole('heading', { name: 'Notas de la versión' })).toBeTruthy();
});

test('the update prompt says so when the new version has no release notes', async () => {
  catalog.instances[0] = { ...first, localState: 'update_available', installedSource: { ...first.installedSource, current_version_id: 'v1' } };
  const user = userEvent.setup(); mount();
  await user.click(await screen.findByRole('button', { name: 'Actualizar…' }));
  await screen.findByText('El autor no ha publicado notas para esta versión.');
  expect(screen.queryByLabelText('Notas de la versión')).toBeNull();
});

test('a vanilla update does not claim to have release notes', async () => {
  catalog.instances[0] = { ...first, source: { type: 'vanilla' }, installedSource: null, installedVersion: '1.21.1', minecraftVersion: '1.21.2', localState: 'installed' };
  const user = userEvent.setup(); mount();
  await user.click(await screen.findByRole('button', { name: 'Actualizar…', exact: true }));
  const dialog = await screen.findByRole('dialog', { name: 'Actualizar Minecraft' });
  expect(screen.queryByRole('heading', { name: 'Notas de la versión' })).toBeNull();
  // Vanilla versions are named as they are, without the pack's "v" prefix.
  expect(within(dialog).getByText('1.21.1')).toBeTruthy();
  expect(within(dialog).getByRole('button', { name: 'Actualizar a 1.21.2' })).toBeTruthy();
});

test('the update prompt names both versions from the catalog before the backend answers, then adds the notes', async () => {
  let release = () => {};
  namesGate = new Promise<void>(resolve => { release = resolve; });
  changelog = '- Nuevas misiones';
  catalog.instances[0] = { ...first, localState: 'update_available',
    source: { ...first.source, label: '1.1.0' }, installedSource: { ...first.installedSource, label: '1.0.0', current_version_id: 'v1' } };
  const user = userEvent.setup(); mount();
  await user.click(await screen.findByRole('button', { name: 'Actualizar…' }));
  const dialog = await screen.findByRole('dialog', { name: 'Actualizar modpack' });
  // The backend is still waiting, yet the readable names and the confirm button are already right.
  expect(within(dialog).getByText('v1.0.0')).toBeTruthy();
  expect(within(dialog).getByRole('button', { name: 'Actualizar a v1.1.0' })).toBeTruthy();
  expect(within(dialog).getByText('Buscando las novedades…')).toBeTruthy();
  await act(async () => { release(); });
  expect((await within(dialog).findByLabelText('Notas de la versión')).textContent).toBe('- Nuevas misiones');
});

test('the update prompt has no world warning box and its confirm button uses the reload icon', async () => {
  catalog.instances[0] = { ...first, localState: 'update_available', installedSource: { ...first.installedSource, current_version_id: 'v1' } };
  const user = userEvent.setup(); mount();
  await user.click(await screen.findByRole('button', { name: 'Actualizar…' }));
  const dialog = await screen.findByRole('dialog', { name: 'Actualizar modpack' });
  expect(within(dialog).queryByText(/no revierte esos cambios/)).toBeNull();
  expect(within(dialog).getByRole('button', { name: /^Actualizar a / }).querySelector('.lucide-refresh-cw')).not.toBeNull();
});

test('the update prompt can be closed from its header without starting anything', async () => {
  catalog.instances[0] = { ...first, localState: 'update_available', installedSource: { ...first.installedSource, current_version_id: 'v1' } };
  const user = userEvent.setup(); mount();
  await user.click(await screen.findByRole('button', { name: 'Actualizar…' }));
  const dialog = await screen.findByRole('dialog', { name: 'Actualizar modpack' });
  await user.click(within(dialog).getByRole('switch', { name: /^Copia de seguridad de mis mundos/ }));
  await user.click(within(dialog).getByRole('button', { name: 'Cerrar' }));
  expect(screen.queryByRole('dialog')).toBeNull();
  expect(backend.invoke.mock.calls.some(([command]) => command === 'begin_update')).toBe(false);
});

test('missing and failed artwork use a local fallback without removing controls', async () => {
  const { container, rerender } = render(<CatalogArtwork url="https://cdn.example.com/icon.png" />);
  fireEvent.error(container.querySelector('img')!);
  await waitFor(() => expect(container.querySelector('.instance-fallback')).not.toBeNull());
  expect(container.querySelector('.instance-fallback')?.getAttribute('src')).toBe(logoWhite);
  expect(container.querySelector('img[crossorigin]')).toBeNull();
  rerender(<CatalogArtwork url="https://cdn.example.com/new.png" />);
  expect(container.querySelector('img')?.getAttribute('referrerpolicy')).toBe('no-referrer');
});

test('the rail shows only the selected account and opens management to change it', async () => {
  accounts.accounts.push({ ...accounts.accounts[0], metadata: { ...accounts.accounts[0].metadata, account_id: 'second', username: 'Otro' } });
  const user = userEvent.setup(); mount();
  const current = await screen.findByRole('button', { name: 'Gestionar cuenta: Jugador' });
  expect(within(screen.getByRole('group', { name: 'Cuentas de Minecraft' })).getAllByRole('button')).toHaveLength(1);
  expect(screen.queryByRole('button', { name: 'Gestionar cuenta: Otro' })).toBeNull();
  await user.click(current);
  const row = screen.getByText('Otro').closest('li')!;
  await user.click(within(row).getByRole('button', { name: 'Opciones de Otro' }));
  await user.click(screen.getByRole('menuitem', { name: 'Seleccionar', exact: true }));
  expect(backend.invoke).toHaveBeenCalledWith('select_account', { accountId: 'second' });
  expect(await screen.findByRole('button', { name: 'Gestionar cuenta: Otro' })).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'Instancias', exact: true }));
  expect(screen.getByRole('heading', { name: 'Aventura', exact: true })).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Acerca de Panita Client' })).toBeNull();
  expect(screen.getByText('v0.2.0-alpha')).toBeTruthy();
});

test('offline avatars stay local and Microsoft avatar failures use the same local default', () => {
  const metadata = accounts.accounts[0].metadata;
  expect(avatarUrl(metadata)).toBeNull();
  expect(avatarUrl({ ...metadata, kind: 'microsoft', username: '../unsafe' })).toBeNull();
  const online = { ...metadata, kind: 'microsoft' as const, profile_id: '12345678-1234-1234-1234-123456789abc' };
  expect(avatarUrl(online)).toBe('https://render.crafty.gg/2d/head/Jugador?cors=true');
  const { container, rerender } = render(<AccountAvatar metadata={metadata} />);
  const fallback = container.querySelector('img')!.getAttribute('src');
  expect(fallback).not.toMatch(/^https?:/);
  expect(fallback).toBeTruthy();
  rerender(<AccountAvatar metadata={online} />);
  fireEvent.error(container.querySelector('img')!);
  expect(container.querySelector('img')!.getAttribute('src')).toBe(fallback);
});

test('a disabled installed instance keeps its options menu open to uninstall', async () => {
  catalog.instances[0] = { ...first, availability: 'disabled', enabled: false };
  const user = userEvent.setup(); mount();
  const card = await screen.findByRole('article', { name: 'Aventura' });
  expect((screen.getByRole('button', { name: 'No disponible para jugar' }) as HTMLButtonElement).disabled).toBe(true);
  const options = within(card).getByRole('button', { name: 'Opciones de instancia' }) as HTMLButtonElement;
  expect(options.disabled).toBe(false);
  await user.click(options);
  expect(screen.getByRole('menuitem', { name: 'Desinstalar…' })).toBeTruthy();
});

test('disabled status explains why the play control is disabled', async () => {
  catalog.instances[0] = { ...first, availability: 'disabled', enabled: false };
  mount();
  const card = await screen.findByRole('article', { name: 'Aventura' });
  const status = within(card).getByText('Deshabilitada');
  fireEvent.focus(status);
  await screen.findByRole('tooltip', { name: 'Esta instancia está deshabilitada. Tus mundos y archivos se conservan.' });
  expect((screen.getByRole('button', { name: 'No disponible para jugar' }) as HTMLButtonElement).disabled).toBe(true);
  expect(screen.queryByRole('menu')).toBeNull();
});

test('real artwork replaces the decorative fallback, and a failed banner restores it', () => {
  const { container } = render(<CatalogArtwork banner url="https://cdn.example.com/banner.png" />);
  expect(container.querySelector('.hero-brand')).toBeNull();
  fireEvent.error(container.querySelector('.banner-image')!);
  expect(container.querySelector('.hero-brand')).not.toBeNull();
});

test('fresh instances hide the dropdown and keep one primary action through installation completion', async () => {
  catalog.instances[0] = { ...first, localState: 'not_installed', installedSource: null, hasInstallationManifest: false };
  const user = userEvent.setup(); mount();
  const article = await screen.findByRole('article', { name: 'Aventura' });
  expect(within(article).queryByRole('button', { name: 'Opciones de instancia' })).toBeNull();
  expect(within(article).getByRole('button', { name: 'Instalar', exact: true })).toBeTruthy();
  await emit(entry({ operation: 'install', phase: 'installing' }));
  expect(within(article).getByRole('button', { name: 'Preparando…' }).querySelector('.spin')).not.toBeNull();
  expect(within(article).queryByRole('progressbar')).toBeNull();
  expect(within(article).queryByRole('button', { name: 'Opciones de instancia' })).toBeNull();
  await emit(entry({ operation: 'install', phase: 'installed' }));
  expect(within(article).getByRole('button', { name: 'Jugar', exact: true })).toBeTruthy();
  expect(await screen.findByText('Aventura: Instalación completada.')).toBeTruthy();
  catalog.instances[0] = { ...first };
  await user.click(screen.getByRole('button', { name: 'Actualizar catálogo' }));
  await waitFor(() => expect(within(article).getByRole('button', { name: 'Opciones de instancia' })).toBeTruthy());
});

test('uninstall closes an open options menu before a stale catalog refresh can re-enable it', async () => {
  const user = userEvent.setup(); mount();
  const article = await screen.findByRole('article', { name: 'Aventura' });
  const options = within(article).getByRole('button', { name: 'Opciones de instancia' }) as HTMLButtonElement;
  await user.click(options);
  expect(screen.getByRole('menu')).toBeTruthy();
  await emit(entry({ operation: 'uninstall', phase: 'uninstalled' }));
  await waitFor(() => expect(screen.queryByRole('menu')).toBeNull());
  expect(screen.queryByRole('button', { name: 'Opciones de instancia' })).toBeNull();
});

test('loader badges use known icons and do not guess a missing loader', () => {
  const { container, rerender } = render(<InstanceBadges instance={first} />);
  for (const [id, name] of [['forge', 'Forge'], ['neoforge', 'NeoForge'], ['fabric', 'Fabric'], ['quilt', 'Quilt']]) {
    rerender(<InstanceBadges instance={{ ...first, source: { ...first.source, loader: id } }} />);
    expect(screen.getByAltText(name)).toBeTruthy();
    expect(container.querySelector('.loader-icon')).not.toBeNull();
  }
  rerender(<InstanceBadges instance={{ ...first, source: { type: 'modrinth' }, installedSource: null }} />);
  expect(screen.getByText('Modpack')).toBeTruthy();
  expect(container.querySelector('.loader-icon')).toBeNull();
  rerender(<InstanceBadges instance={{ ...first, source: { type: 'vanilla' } }} />);
  expect(screen.queryByText('Modpack')).toBeNull();
});

test('instance tooltips reuse game and loader badges with exact installed loader versions', async () => {
  catalog.instances[0] = { ...first, installedLoader: { name: 'neoforge', version: '21.1.264' }, source: { ...first.source, label: 'v1.2.3' } };
  mount();
  const article = await screen.findByRole('article', { name: 'Aventura' });
  expect(within(article).getByAltText('NeoForge 21.1.264')).toBeTruthy();
  expect(within(article).getByText('v1.2.3')).toBeTruthy();
  expect(within(article).queryByText('vv1.2.3')).toBeNull();
  const rail = screen.getByRole('button', { name: 'Aventura: Lista para jugar' });
  fireEvent.focus(rail);
  const tooltip = await screen.findByRole('tooltip');
  expect(within(tooltip).getByText('Aventura')).toBeTruthy();
  expect(within(tooltip).getByText(first.description)).toBeTruthy();
  expect(within(tooltip).getByText('Minecraft 1.21.1')).toBeTruthy();
  expect(within(tooltip).getByAltText('NeoForge 21.1.264')).toBeTruthy();
  expect(within(tooltip).getByText('v1.2.3')).toBeTruthy();
  expect(tooltip.querySelector('.game-icon')).not.toBeNull();
  expect(document.querySelector('svg.fill-secondary')).not.toBeNull();
  expect(within(article).getByAltText('NeoForge 21.1.264').parentElement?.querySelector('.game-icon')).not.toBeNull();
  expect(within(article).queryByText('Lista para jugar')).toBeNull();
});


test('update status stays in the tooltip while details preserve only the pack version', async () => {
  catalog.instances[0] = { ...first, localState: 'update_available', installedSource: { ...first.installedSource, current_version_id: 'v1' } };
  const user = userEvent.setup(); mount();
  const article = await screen.findByRole('article', { name: 'Aventura' });
  expect(within(article).queryByRole('button', { name: 'Actualizar', exact: true })).toBeNull();
  expect(within(article).queryByText('Actualización disponible')).toBeNull();
  expect(within(article).getByText('vDos')).toBeTruthy();
  fireEvent.focus(screen.getByRole('button', { name: 'Aventura: Actualización disponible' }));
  const tooltip = await screen.findByRole('tooltip');
  expect(within(tooltip).getByText('Actualización disponible').querySelector('svg')).not.toBeNull();
  expect(within(tooltip).getByText('vDos')).toBeTruthy();
  expect(screen.queryByRole('dialog')).toBeNull();
  expect(backend.invoke.mock.calls.some(([command]) => command === 'begin_update')).toBe(false);
});

test('instance menu opens fixed folders and existing logs, and routes shared cache to confirmation', async () => {
  const user = userEvent.setup(); mount();
  const article = await screen.findByRole('article', { name: 'Aventura' });
  const options = within(article).getByRole('button', { name: 'Opciones de instancia' });
  expect(within(article).getByRole('button', { name: 'Jugar', exact: true }).querySelector('svg')).not.toBeNull();
  await user.click(options);
  expect(screen.getByRole('menuitem', { name: 'Ver registro', exact: true }).getAttribute('aria-disabled')).toBe('true');
  await user.click(screen.getByRole('menuitem', { name: 'Abrir carpeta', exact: true }));
  expect(backend.invoke).toHaveBeenCalledWith('open_instance_folder', { instanceId: 'pack-a', folder: 'instance' });
  await emit(entry({ phase: 'exited' }));
  await user.click(options);
  await user.click(screen.getByRole('menuitem', { name: 'Ver registro', exact: true }));
  expect(backend.invoke).toHaveBeenCalledWith('open_game_log', { instanceId: 'pack-a', requestId: 'request' });
  await user.click(options);
  await user.click(screen.getByRole('menuitem', { name: 'Limpiar caché de modpacks…', exact: true }));
  expect(await screen.findByRole('heading', { name: 'Caché de modpacks' })).toBeTruthy();
  expect(backend.invoke.mock.calls.some(([command]) => command === 'clear_content_cache')).toBe(false);
});


test('browser update preview permits inspection while desktop actions stay disabled', async () => {
  backend.native = false;
  const user = userEvent.setup(); mount();
  await user.click(await screen.findByRole('button', { name: 'Modpack con actualización: Actualización disponible' }));
  await user.click(screen.getByRole('button', { name: 'Actualizar…', exact: true }));
  expect((screen.getByRole('button', { name: /^Actualizar a / }) as HTMLButtonElement).disabled).toBe(true);
  expect((screen.getByLabelText(/^Copia de seguridad de mis mundos/) as HTMLInputElement).checked).toBe(true);
  await user.click(screen.getByRole('button', { name: 'Cancelar', exact: true }));
  expect(screen.queryByRole('dialog')).toBeNull();
  await user.click(screen.getByRole('button', { name: 'Opciones de instancia' }));
  expect(screen.getByRole('menuitem', { name: 'Abrir carpeta' }).getAttribute('aria-disabled')).toBe('true');
  expect(screen.getByRole('menuitem', { name: 'Ver registro' }).getAttribute('aria-disabled')).toBe('true');
  expect(backend.invoke.mock.calls.some(([command]) => command === 'begin_update')).toBe(false);
});


test('cancelling the update modal never starts an update and restores the default backup on reopening', async () => {
  catalog.instances[0] = { ...first, localState: 'update_available', installedSource: { ...first.installedSource, current_version_id: 'v1' } };
  const user = userEvent.setup(); mount();
  const trigger = await screen.findByRole('button', { name: 'Actualizar…', exact: true });
  await user.click(trigger);
  await user.click(screen.getByLabelText(/^Copia de seguridad de mis mundos/));
  const dialog = screen.getByRole('dialog');
  fireEvent(dialog, new Event('cancel', { bubbles: false, cancelable: true }));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(backend.invoke.mock.calls.some(([command]) => command === 'begin_update')).toBe(false);
  await user.click(trigger);
  expect((screen.getByLabelText(/^Copia de seguridad de mis mundos/) as HTMLInputElement).checked).toBe(true);
});


test('uninstall defaults to preserving player files and sends an explicit false deletion choice', async () => {
  const user = userEvent.setup(); mount();
  const article = await screen.findByRole('article', { name: 'Aventura' });
  await user.click(within(article).getByRole('button', { name: 'Opciones de instancia' }));
  await user.click(screen.getByRole('menuitem', { name: 'Desinstalar…' }));
  const dialog = screen.getByRole('dialog', { name: 'Desinstalar instancia' });
  expect((within(dialog).getByLabelText('Eliminar también la carpeta de la instancia') as HTMLInputElement).checked).toBe(false);
  expect(within(dialog).queryByRole('button', { name: 'Eliminar instancia', exact: true })).toBeNull();
  await user.click(within(dialog).getByRole('button', { name: 'Desinstalar', exact: true }));
  expect(backend.invoke).toHaveBeenCalledWith('begin_uninstall', expect.objectContaining({ instanceId: 'pack-a', deleteInstance: false }));
});

test('permanent erasure requires its folder and a separate acknowledgement, and cancellation sends nothing', async () => {
  const user = userEvent.setup(); mount();
  const article = await screen.findByRole('article', { name: 'Aventura' });
  async function open() { await user.click(within(article).getByRole('button', { name: 'Opciones de instancia' })); await user.click(screen.getByRole('menuitem', { name: 'Desinstalar…' })); }
  await open();
  await user.click(screen.getByLabelText('Eliminar también la carpeta de la instancia'));
  const erase = screen.getByRole('button', { name: 'Eliminar instancia', exact: true }) as HTMLButtonElement;
  expect(erase.disabled).toBe(true);
  await screen.findByText('C:\\Synthetic\\PanitaClient\\instances\\aventura');
  await user.click(screen.getByLabelText('Entiendo que perderé todos los datos de esta instancia'));
  expect(erase.disabled).toBe(false);
  await user.click(screen.getByRole('button', { name: 'Cancelar', exact: true }));
  expect(backend.invoke.mock.calls.some(([command]) => command === 'begin_uninstall')).toBe(false);
  await open();
  expect((screen.getByLabelText('Eliminar también la carpeta de la instancia') as HTMLInputElement).checked).toBe(false);
  await user.click(screen.getByLabelText('Eliminar también la carpeta de la instancia'));
  await user.click(screen.getByLabelText('Entiendo que perderé todos los datos de esta instancia'));
  await user.click(screen.getByRole('button', { name: 'Eliminar instancia', exact: true }));
  expect(backend.invoke).toHaveBeenCalledWith('begin_uninstall', expect.objectContaining({ instanceId: 'pack-a', deleteInstance: true }));
});

test('instance navigation starts with six cards and pages without a scrolling rail', async () => {
  catalog.instances = Array.from({ length: 7 }, (_, index) => ({ ...first, id: index === 0 ? first.id : 'extra-' + index, name: 'Instancia ' + (index + 1) }));
  const user = userEvent.setup(); const { container } = mount();
  await screen.findByRole('heading', { name: 'Instancia 1', exact: true });
  expect(container.querySelectorAll('.rail-instance')).toHaveLength(6);
  expect((screen.getByRole('button', { name: 'Instancias anteriores' }) as HTMLButtonElement).disabled).toBe(true);
  await user.click(screen.getByRole('button', { name: 'Más instancias' }));
  expect(container.querySelectorAll('.rail-instance')).toHaveLength(1);
  expect(screen.getByRole('button', { name: 'Instancia 7: Lista para jugar' })).toBeTruthy();
  expect((screen.getByRole('button', { name: 'Más instancias' }) as HTMLButtonElement).disabled).toBe(true);
  await user.click(screen.getByRole('button', { name: 'Instancia 7: Lista para jugar' }));
  expect(await screen.findByRole('heading', { name: 'Instancia 7', exact: true })).toBeTruthy();
  catalog.instances = catalog.instances.slice(0, 2);
  await user.click(screen.getByRole('button', { name: 'Actualizar catálogo' }));
  await screen.findByRole('heading', { name: 'Instancia 1', exact: true });
  expect(container.querySelectorAll('.rail-instance')).toHaveLength(2);
  expect(screen.queryByRole('button', { name: 'Más instancias' })).toBeNull();
});

test('resizing uses additional rail space and restores paging when complete cards no longer fit', async () => {
  let height = 600;
  const size = vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockImplementation(function (this: HTMLElement) {
    return this.classList.contains('rail-browser') ? height : 0;
  });
  try {
    catalog.instances = Array.from({ length: 10 }, (_, index) => ({ ...first, id: index === 0 ? first.id : 'extra-' + index, name: 'Instancia ' + (index + 1) }));
    const { container } = mount();
    await screen.findByRole('heading', { name: 'Instancia 1', exact: true });
    await waitFor(() => expect(container.querySelectorAll('.rail-instance')).toHaveLength(8));
    height = 800;
    fireEvent(window, new Event('resize'));
    await waitFor(() => expect(container.querySelectorAll('.rail-instance')).toHaveLength(10));
    expect(screen.queryByRole('button', { name: 'Más instancias' })).toBeNull();
    height = 455;
    fireEvent(window, new Event('resize'));
    await waitFor(() => expect(container.querySelectorAll('.rail-instance')).toHaveLength(6));
    expect(screen.getByRole('button', { name: 'Más instancias' })).toBeTruthy();
  } finally { size.mockRestore(); }
});

test('instance inheritance switches disable only their fields and advanced content expands accessibly', async () => {
  const user = userEvent.setup(); mount();
  const card = await screen.findByRole('article', { name: 'Aventura' });
  await user.click(within(card).getByRole('button', { name: 'Opciones de instancia' }));
  await user.click(screen.getByRole('menuitem', { name: 'Ajustes de la instancia' }));
  const inherit = await screen.findByRole('switch', { name: 'Heredar ajuste global: Memoria' });
  const ram = screen.getByLabelText('RAM Asignada (MiB)') as HTMLInputElement;
  expect(ram.disabled).toBe(true);
  await user.click(inherit);
  expect(ram.disabled).toBe(false);
  expect((screen.getByLabelText('Ancho de Minecraft (píxeles)') as HTMLInputElement).disabled).toBe(true);
  const advanced = screen.getByRole('button', { name: 'Java y argumentos avanzados' });
  expect(advanced.getAttribute('aria-expanded')).toBe('false');
  await user.click(advanced);
  expect(advanced.getAttribute('aria-expanded')).toBe('true');
  const jvm = screen.getByLabelText('Memoria y argumentos JVM adicionales (uno por línea)') as HTMLTextAreaElement;
  expect(jvm.disabled).toBe(true);
  await user.click(screen.getByRole('switch', { name: 'Heredar ajuste global: Java y argumentos avanzados' }));
  expect(jvm.disabled).toBe(false);
  expect(backend.invoke.mock.calls.some(([command]) => command === 'save_instance_settings')).toBe(false);
});

test('active repair shows one content spinner and disables the options dropdown', async () => {
  mount();
  const card = await screen.findByRole('article', { name: 'Aventura' });
  await emit(entry({ operation: 'repair', phase: 'installing' }));
  expect(within(card).queryByText('Reparando', { exact: true })).toBeNull();
  expect((within(card).getByRole('button', { name: 'Opciones de instancia' }) as HTMLButtonElement).disabled).toBe(true);
  expect(within(card).getByRole('button', { name: 'Preparando…' }).getAttribute('aria-disabled')).toBe('true');
  expect(card.querySelectorAll('.spin').length).toBe(1);
});

const withVariants = { variants: ['lite', 'standard', 'deluxe'] };

test('an instance with several variants asks which to install, with Standard preselected', async () => {
  catalog.instances[0] = { ...first, ...withVariants, localState: 'not_installed', hasInstallationManifest: false, installedSource: null };
  const user = userEvent.setup(); mount();
  await user.click(await screen.findByRole('button', { name: 'Instalar', exact: true }));
  const dialog = await screen.findByRole('dialog', { name: 'Elige cómo instalar' });
  expect(backend.invoke).toHaveBeenCalledWith('instance_variants', { instanceId: 'pack-a' });
  expect(backend.invoke.mock.calls.some(([command]) => command === 'begin_install')).toBe(false);
  // The sizes arrive after the cards, which are already in place.
  await within(dialog).findByText(/28 archivos/);
  expect(((await within(dialog).findByRole('radio', { name: /^Estándar/ })) as HTMLInputElement).checked).toBe(true);
  expect(within(dialog).getByText(/Sin mods cosméticos ni shaders/)).toBeTruthy();
  await user.click(within(dialog).getByRole('radio', { name: /^Mejorado/ }));
  await user.click(within(dialog).getByRole('button', { name: 'Instalar', exact: true }));
  expect(backend.invoke).toHaveBeenCalledWith('begin_install', expect.objectContaining({ instanceId: 'pack-a', variant: 'deluxe' }));
});

test('an instance that offers only Standard installs without any question', async () => {
  catalog.instances[0] = { ...first, localState: 'not_installed', hasInstallationManifest: false, installedSource: null };
  const user = userEvent.setup(); mount();
  await user.click(await screen.findByRole('button', { name: 'Instalar', exact: true }));
  expect(screen.queryByRole('dialog')).toBeNull();
  expect(backend.invoke).toHaveBeenCalledWith('begin_install', expect.objectContaining({ instanceId: 'pack-a' }));
  expect(backend.invoke.mock.calls.some(([command]) => command === 'instance_variants')).toBe(false);
});

test('the variant that suits the machine is preselected, marked and explained', async () => {
  catalog.instances[0] = { ...first, ...withVariants, localState: 'not_installed', hasInstallationManifest: false, installedSource: null };
  systemResources = { memoryMib: 4096, freeDiskMib: 500 * 1024 };
  const user = userEvent.setup(); mount();
  await user.click(await screen.findByRole('button', { name: 'Instalar', exact: true }));
  const dialog = await screen.findByRole('dialog', { name: 'Elige cómo instalar' });
  await within(dialog).findByText(/28 archivos/);
  await waitFor(() => expect(dialog.querySelector('.variant-reason')?.textContent).toBe('Te recomendamos el perfil de rendimiento Ligero porque tu equipo tiene 4 GB de RAM.'));
  expect(dialog.querySelector('.variant-reason > svg')).not.toBeNull();
  expect(dialog.querySelector('.variant-reason-name svg')).toBeNull();
  expect((within(dialog).getByRole('radio', { name: /^Ligero/ }) as HTMLInputElement).checked).toBe(true);
  expect(within(dialog.querySelector('label:has(input[value="lite"])') as HTMLElement).getByText('Recomendada')).toBeTruthy();
  expect(dialog.querySelectorAll('.variant-chip').length).toBe(1);
  // The player can still choose another one.
  await user.click(within(dialog).getByRole('radio', { name: /^Estándar/ }));
  await user.click(within(dialog).getByRole('button', { name: 'Instalar', exact: true }));
  expect(backend.invoke).toHaveBeenCalledWith('begin_install', expect.objectContaining({ variant: 'standard' }));
});

test('a machine with 12 GB or more is offered Mejorado, and one that could not be read keeps Standard', async () => {
  catalog.instances[0] = { ...first, ...withVariants, localState: 'not_installed', hasInstallationManifest: false, installedSource: null };
  systemResources = { memoryMib: 16 * 1024, freeDiskMib: 500 * 1024 };
  const user = userEvent.setup(); const view = mount();
  await user.click(await screen.findByRole('button', { name: 'Instalar', exact: true }));
  const dialog = await screen.findByRole('dialog', { name: 'Elige cómo instalar' });
  await within(dialog).findByText(/28 archivos/);
  await waitFor(() => expect((within(dialog).getByRole('radio', { name: /^Mejorado/ }) as HTMLInputElement).checked).toBe(true));
  await user.click(within(dialog).getByRole('button', { name: 'Cancelar' }));
  view.unmount();
  systemResources = null;
  mount();
  await user.click(await screen.findByRole('button', { name: 'Instalar', exact: true }));
  const unread = await screen.findByRole('dialog', { name: 'Elige cómo instalar' });
  await within(unread).findByText(/28 archivos/);
  expect((within(unread).getByRole('radio', { name: /^Estándar/ }) as HTMLInputElement).checked).toBe(true);
  expect(unread.querySelector('.variant-reason')).toBeNull();
});

test('the Rendimiento tab marks the recommended variant but keeps the installed one selected', async () => {
  catalog.instances[0] = { ...first, ...withVariants, installedVariant: 'standard' };
  systemResources = { memoryMib: 2048, freeDiskMib: 500 * 1024 };
  const user = userEvent.setup(); mount();
  await openVariantTab(user);
  await screen.findByText(/28 archivos/);
  await waitFor(() => expect(document.querySelector('.variant-reason')?.textContent).toBe('Te recomendamos el perfil de rendimiento Ligero porque tu equipo tiene 2 GB de RAM.'));
  expect(screen.getByText('Recomendada')).toBeTruthy();
  expect((screen.getByRole('radio', { name: 'Estándar' }) as HTMLInputElement).checked).toBe(true);
  expect((screen.getByRole('button', { name: 'Cambiar rendimiento' }) as HTMLButtonElement).disabled).toBe(true);
});

async function openVariantTab(user: ReturnType<typeof userEvent.setup>) {
  await screen.findByRole('button', { name: 'Jugar', exact: true });
  await user.click(within(screen.getByRole('article', { name: 'Aventura' })).getByRole('button', { name: 'Opciones de instancia' }));
  // Changing the variant lives in the instance settings, not in this menu.
  expect(screen.queryByRole('menuitem', { name: /Cambiar (variante|rendimiento)/ })).toBeNull();
  await user.click(screen.getByRole('menuitem', { name: 'Ajustes de la instancia', exact: true }));
  await user.click(await screen.findByRole('tab', { name: 'Rendimiento' }));
}

test('the variant is changed from a tab of the instance settings, as an update of the same version', async () => {
  catalog.instances[0] = { ...first, ...withVariants, installedVariant: 'standard' };
  const user = userEvent.setup(); mount();
  await openVariantTab(user);
  const lite = await screen.findByRole('radio', { name: 'Ligero' });
  await screen.findByText(/28 archivos/);
  expect(document.querySelector('.settings-tabs ~ div:not([hidden]) > section.variant-panel')).toBeTruthy();
  // The installed variant is marked and cannot be "changed" to itself.
  expect(screen.getByText('Instalada')).toBeTruthy();
  expect((screen.getByRole('button', { name: 'Cambiar rendimiento' }) as HTMLButtonElement).disabled).toBe(true);
  await user.click(lite);
  await user.click(screen.getByRole('button', { name: 'Cambiar rendimiento' }));
  expect(backend.invoke).toHaveBeenCalledWith('begin_update', expect.objectContaining({
    instanceId: 'pack-a',
    update: { from_version: 'v2', to_version: 'v2', backup_worlds: false, from_variant: 'standard', to_variant: 'lite' },
  }));
  // Progress and results are on the instance page, so the player goes back to it.
  expect(await screen.findByRole('button', { name: 'Jugar', exact: true })).toBeTruthy();
});

test('a conflict while changing the variant can be retried from the instance page with the same variant', async () => {
  catalog.instances[0] = { ...first, ...withVariants, installedVariant: 'standard' };
  const user = userEvent.setup(); mount();
  await openVariantTab(user);
  await user.click(await screen.findByRole('radio', { name: 'Ligero' }));
  await user.click(screen.getByRole('button', { name: 'Cambiar rendimiento' }));
  await emit(entry({ operation: 'update', phase: 'failed', error: 'pack_conflict', packConflicts: [{ path: 'mods/a.jar', reason: 'occupied' }] }));
  await user.click(await screen.findByRole('button', { name: 'Ver detalles' }));
  await user.click(await screen.findByRole('button', { name: 'Continuar y guardar mis archivos' }));
  expect(backend.invoke).toHaveBeenCalledWith('begin_update', expect.objectContaining({
    update: { from_version: 'v2', to_version: 'v2', backup_worlds: false, from_variant: 'standard', to_variant: 'lite', move_aside: true },
  }));
});

test('an instance with one variant has no variant tab, and one that is not installed explains why', async () => {
  const user = userEvent.setup(); mount();
  await screen.findByRole('button', { name: 'Jugar', exact: true });
  await user.click(within(screen.getByRole('article', { name: 'Aventura' })).getByRole('button', { name: 'Opciones de instancia' }));
  await user.click(screen.getByRole('menuitem', { name: 'Ajustes de la instancia', exact: true }));
  await screen.findByRole('heading', { name: 'Ajustes · Aventura', exact: true });
  expect(screen.queryByRole('tab', { name: 'Rendimiento' })).toBeNull();
});

async function openInstanceSettings(user: ReturnType<typeof userEvent.setup>) {
  await screen.findByRole('button', { name: 'Jugar', exact: true });
  await user.click(within(screen.getByRole('article', { name: 'Aventura' })).getByRole('button', { name: 'Opciones de instancia' }));
  await user.click(screen.getByRole('menuitem', { name: 'Ajustes de la instancia', exact: true }));
  await screen.findByRole('heading', { name: 'Ajustes · Aventura', exact: true });
  return within(screen.getByRole('tablist', { name: 'Secciones de ajustes' })).getAllByRole('tab').map(tab => tab.textContent);
}

test('the instance settings put Capturas between Minecraft and Rendimiento', async () => {
  catalog.instances[0] = { ...first, ...withVariants, installedVariant: 'standard' };
  const user = userEvent.setup(); mount();
  expect(await openInstanceSettings(user)).toEqual(['Minecraft', 'Capturas', 'Rendimiento']);
  await user.click(screen.getByRole('tab', { name: 'Capturas' }));
  expect(await screen.findByRole('heading', { name: 'Aún no hay capturas' })).toBeTruthy();
  // The stylesheet joins a tab's panel to the tab bar only when the panel is a section inside a div.
  expect(document.querySelector('.settings-tabs ~ div:not([hidden]) > section.screenshot-panel')).toBeTruthy();
  expect(backend.invoke).toHaveBeenCalledWith('list_screenshots', { instanceId: 'pack-a' });
  // The Minecraft settings are hidden, not removed, so unsaved edits survive the tab change.
  await user.click(screen.getByRole('tab', { name: 'Minecraft' }));
  expect(screen.queryByRole('heading', { name: 'Aún no hay capturas' })).toBeNull();
});

test('an instance with one variant has Minecraft and Capturas tabs and global settings gain none', async () => {
  const user = userEvent.setup(); mount();
  expect(await openInstanceSettings(user)).toEqual(['Minecraft', 'Capturas']);
  await user.click(screen.getByRole('button', { name: 'Ajustes Globales' }));
  const global = within(screen.getByRole('tablist', { name: 'Secciones de ajustes' })).getAllByRole('tab').map(tab => tab.textContent);
  expect(global).toEqual(['General', 'Minecraft', 'Almacenamiento', 'Diagnóstico']);
});

test('a version update of a pack installed before variants names no variant, so the backend keeps it', async () => {
  catalog.instances[0] = { ...first, ...withVariants, localState: 'update_available', installedSource: { ...first.installedSource, current_version_id: 'v1' } };
  const user = userEvent.setup(); mount();
  await user.click(await screen.findByRole('button', { name: 'Actualizar…' }));
  await user.click(screen.getByRole('button', { name: /^Actualizar a / }));
  expect(backend.invoke).toHaveBeenCalledWith('begin_update', expect.objectContaining({ update: { from_version: 'v1', to_version: 'v2', backup_worlds: true } }));
});

test('a variant the instance stopped offering moves to Standard by itself, once, and tells the player', async () => {
  catalog.instances[0] = { ...first, variants: ['standard'], installedVariant: 'lite' };
  mount();
  await waitFor(() => expect(backend.invoke).toHaveBeenCalledWith('begin_update', expect.objectContaining({
    instanceId: 'pack-a',
    update: { from_version: 'v2', to_version: 'v2', backup_worlds: false, from_variant: 'lite', to_variant: 'standard' },
  })));
  expect(await screen.findByText(/Ligero ya no se ofrece/)).toBeTruthy();
  // A failed attempt is not repeated on its own.
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 50)); });
  expect(backend.invoke.mock.calls.filter(([command]) => command === 'begin_update')).toHaveLength(1);
});

test('the variant cards appear at once and only their sizes wait for the backend', async () => {
  const base = backend.invoke.getMockImplementation();
  backend.invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => command === 'instance_variants' ? new Promise(() => {}) : base?.(command, args));
  catalog.instances[0] = { ...first, ...withVariants, localState: 'not_installed', hasInstallationManifest: false, installedSource: null };
  const user = userEvent.setup(); mount();
  await user.click(await screen.findByRole('button', { name: 'Instalar', exact: true }));
  const dialog = await screen.findByRole('dialog', { name: 'Elige cómo instalar' });
  for (const name of ['Ligero', 'Estándar', 'Mejorado']) {
    expect((within(dialog).getByRole('radio', { name }) as HTMLInputElement).disabled).toBe(true);
  }
  expect(within(dialog).getAllByText('Calculando tamaño…')).toHaveLength(3);
  // The default is already marked, but nothing can be confirmed until the sizes are known.
  expect((within(dialog).getByRole('radio', { name: 'Estándar' }) as HTMLInputElement).checked).toBe(true);
  expect((within(dialog).getByRole('button', { name: 'Instalar', exact: true }) as HTMLButtonElement).disabled).toBe(true);
});

const resolution = (mib: number, basis = 'target') => ({ mib, basis });

test('automatic memory disables the manual controls, explains the value and keeps the saved number', async () => {
  ramPreview = { totalMib: 16384, vanilla: resolution(4096), lite: resolution(4096), standard: resolution(6144), deluxe: resolution(8192) };
  const user = userEvent.setup(); mount();
  await screen.findByRole('heading', { name: 'Aventura', exact: true });
  await user.click(screen.getByRole('button', { name: 'Ajustes Globales' }));
  await user.click(screen.getByRole('tab', { name: 'Minecraft' }));
  const ram = await screen.findByLabelText('RAM Asignada (MiB)') as HTMLInputElement;
  const auto = screen.getByRole('switch', { name: 'Automática' }) as HTMLInputElement;
  expect([auto.checked, ram.disabled]).toEqual([false, false]);
  await user.click(auto);
  expect(auto.checked).toBe(true);
  expect(ram.disabled).toBe(true);
  expect((screen.getByLabelText('Control deslizante de RAM para Minecraft') as HTMLInputElement).disabled).toBe(true);
  expect(ram.value).toBe('4096');
  // The explanation is a tooltip on the option, announced to screen readers, never a visible line.
  expect(await screen.findByText('Se calcula al jugar según la RAM de tu equipo y el tipo de instancia.')).toBeTruthy();
  expect(document.querySelector('.memory-auto-note')).toBeNull();
  await user.click(screen.getByRole('button', { name: 'Guardar ajustes' }));
  await waitFor(() => expect(backend.invoke).toHaveBeenCalledWith('save_global_settings', { settings: expect.objectContaining({ ram_auto: true, ram_mib: 4096 }) }));
  await user.click(auto);
  expect(auto.checked).toBe(false);
  expect(ram.disabled).toBe(false);
  expect(screen.getByText('Se calcula al jugar según la RAM de tu equipo y el tipo de instancia.')).toBeTruthy();
});

test('automatic memory says so plainly when the machine cannot be read', async () => {
  settingsRamAuto = true;
  const user = userEvent.setup(); mount();
  await screen.findByRole('heading', { name: 'Aventura', exact: true });
  await user.click(screen.getByRole('button', { name: 'Ajustes Globales' }));
  await user.click(screen.getByRole('tab', { name: 'Minecraft' }));
  expect(await screen.findByText(/No pudimos leer la RAM de tu equipo/)).toBeTruthy();
  expect((screen.getByRole('switch', { name: 'Automática' }) as HTMLInputElement).checked).toBe(true);
});

test('an instance can choose automatic memory on its own and names its own value', async () => {
  ramPreview = { totalMib: 16384, vanilla: resolution(4096), lite: resolution(4096), standard: resolution(6144), deluxe: resolution(8192) };
  const user = userEvent.setup(); mount();
  const card = await screen.findByRole('article', { name: 'Aventura' });
  await user.click(within(card).getByRole('button', { name: 'Opciones de instancia' }));
  await user.click(screen.getByRole('menuitem', { name: 'Ajustes de la instancia' }));
  const inherit = await screen.findByRole('switch', { name: 'Heredar ajuste global: Memoria' }) as HTMLInputElement;
  // While the memory is inherited the instance has no choice of its own to make.
  expect(inherit.checked).toBe(true);
  expect(screen.queryByRole('switch', { name: 'Automática' })).toBeNull();
  await user.click(inherit);
  await user.click(screen.getByRole('switch', { name: 'Automática' }));
  await user.click(screen.getByRole('button', { name: 'Guardar ajustes' }));
  await waitFor(() => expect(backend.invoke).toHaveBeenCalledWith('save_instance_settings', expect.objectContaining({ settings: expect.objectContaining({ ram_auto: true, ram_mib: 4096 }) })));
  expect(await screen.findByText('Se calcula al jugar según la RAM de tu equipo.')).toBeTruthy();
  // The automatic value for this kind of instance is shown in the field and on the slider.
  expect((screen.getByLabelText('RAM Asignada (MiB)') as HTMLInputElement).value).toBe('6144');
  expect((screen.getByLabelText('Control deslizante de RAM para Minecraft') as HTMLInputElement).value).toBe('6144');
});

test('the memory slider stops at the RAM of this machine', async () => {
  systemResources = { memoryMib: 8000, freeDiskMib: 500 * 1024 };
  const user = userEvent.setup(); mount();
  await screen.findByRole('heading', { name: 'Aventura', exact: true });
  await user.click(screen.getByRole('button', { name: 'Ajustes Globales' }));
  await user.click(screen.getByRole('tab', { name: 'Minecraft' }));
  const slider = await screen.findByLabelText('Control deslizante de RAM para Minecraft') as HTMLInputElement;
  await waitFor(() => expect(slider.max).toBe('8192'));
  await user.clear(screen.getByLabelText('RAM Asignada (MiB)')); await user.type(screen.getByLabelText('RAM Asignada (MiB)'), '12288');
  expect(await screen.findByText(/Este valor es mayor que la RAM de tu equipo \(8 GiB\)/)).toBeTruthy();
  expect(slider.max).toBe('8192');
});

test('without a reading of the machine the memory slider keeps its former range', async () => {
  const user = userEvent.setup(); mount();
  await screen.findByRole('heading', { name: 'Aventura', exact: true });
  await user.click(screen.getByRole('button', { name: 'Ajustes Globales' }));
  await user.click(screen.getByRole('tab', { name: 'Minecraft' }));
  expect(((await screen.findByLabelText('Control deslizante de RAM para Minecraft')) as HTMLInputElement).max).toBe('32768');
});
