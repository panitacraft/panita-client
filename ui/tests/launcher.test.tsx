import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { Launcher } from '../src/launcher';
import { PlayProvider } from '../src/play';
import { PreferencesProvider } from '../src/settings';
import { CatalogArtwork } from '../src/components/catalog-artwork';
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
  backend.native = true;
  catalog = { instances: [{ ...first }, { ...first, id: 'pack-b', name: 'Otra aventura' }], stale: false, cacheWarning: null, registryWarning: null };
  play = { revision: 0, processSessionId: 'test', closeBlocked: false, gracefulStopSupported: true, instances: [] };
  backend.listeners.clear();
  accounts = { revision: 1, selected_account_id: 'account', login_pending: false, login_error: null, accounts: [{ metadata: { account_id: 'account', kind: 'offline', username: 'Jugador', profile_id: 'profile', oauth_client_id: null }, status: 'offline', error: null, persisted: true }] };
  backend.invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
    switch (command) {
      case 'main_window_control': return false;
      case 'initialize': return;
      case 'catalog_snapshot': return catalog;
      case 'play_snapshot': return play;
      case 'settings_snapshot': return { instanceId: args?.instanceId ?? null, instance: null, windowWarning: null, global: { schema_version: 2, ram_mib: 4096, extra_jvm_args: [], java_path_override: null, show_console_on_start: false, window_size: { width: 1280, height: 720 }, launcher: { selected_instance_id: 'pack-a', window: null } } };
      case 'account_snapshot': return accounts;
      case 'select_account': accounts = { ...accounts, revision: accounts.revision + 1, selected_account_id: String(args?.accountId) }; return;
      case 'instance_folder_path': return 'C:\\Synthetic\\PanitaClient\\instances\\aventura';
      case 'begin_uninstall': return;
      case 'content_cache_size': return 0;
      case 'pack_version_names': return { installed: 'Uno', available: 'Dos' };
      case 'pack_files': return { files: [manualFile], complete: false };
      case 'import_pack_file': throw 'artifact_mismatch';
      case 'select_instance': case 'begin_install': case 'cancel_play': case 'begin_update': case 'confirm_play': case 'begin_play': case 'begin_repair': case 'stop_game': case 'open_game_log': case 'open_instance_folder': return;
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
  expect(screen.getByRole('dialog', { name: 'Actualizar de Minecraft 1.21.1 a 1.21.2' })).toBeTruthy();
  expect((screen.getByLabelText('Crear copia de mis mundos (recomendado)') as HTMLInputElement).checked).toBe(true);
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
  await screen.findByText('Actualizar el modpack de Uno a Dos');
  expect(screen.getByRole('dialog', { name: 'Actualizar el modpack de Uno a Dos' })).toBeTruthy();
  expect((screen.getByLabelText('Crear copia de mis mundos (recomendado)') as HTMLInputElement).checked).toBe(true);
  expect(backend.invoke.mock.calls.some(([command]) => command === 'begin_update')).toBe(false);
  await user.click(screen.getByRole('button', { name: 'Actualizar', exact: true }));
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

test('disabled status explains why both instance controls are disabled', async () => {
  catalog.instances[0] = { ...first, availability: 'disabled', enabled: false };
  mount();
  const card = await screen.findByRole('article', { name: 'Aventura' });
  const status = within(card).getByText('Deshabilitada');
  fireEvent.focus(status);
  await screen.findByRole('tooltip', { name: 'Esta instancia está deshabilitada. Tus mundos y archivos se conservan.' });
  expect((screen.getByRole('button', { name: 'No disponible para jugar' }) as HTMLButtonElement).disabled).toBe(true);
  expect((within(card).getByRole('button', { name: 'Opciones de instancia' }) as HTMLButtonElement).disabled).toBe(true);
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
  expect((screen.getByRole('button', { name: 'Actualizar', exact: true }) as HTMLButtonElement).disabled).toBe(true);
  expect((screen.getByLabelText('Crear copia de mis mundos (recomendado)') as HTMLInputElement).checked).toBe(true);
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
  await user.click(screen.getByLabelText('Crear copia de mis mundos (recomendado)'));
  const dialog = screen.getByRole('dialog');
  fireEvent(dialog, new Event('cancel', { bubbles: false, cancelable: true }));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(backend.invoke.mock.calls.some(([command]) => command === 'begin_update')).toBe(false);
  await user.click(trigger);
  expect((screen.getByLabelText('Crear copia de mis mundos (recomendado)') as HTMLInputElement).checked).toBe(true);
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
