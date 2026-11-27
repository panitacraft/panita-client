import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { SkinsPage } from '../src/skins';
import { TooltipProvider } from '../src/components/ui/tooltip';
import type { AccountView } from '../src/account-state';
import type { SkinsView } from '../src/skin-state';

const backend = vi.hoisted(() => ({ native: true, invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => backend.native, invoke: backend.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
// The real renderer needs WebGL. The stand-in exposes what it was asked to show, which is what a
// preview that "updates in real time" means for this page.
vi.mock('../src/components/skin-viewer', async importOriginal => {
  const original = await importOriginal<typeof import('../src/components/skin-viewer')>();
  return {
    ...original,
    SkinViewer3D: (props: { skin: string | null; model: string; cape: string | null; playing: boolean; elytra?: boolean }) =>
      <div data-testid="viewer" data-skin={props.skin ?? ''} data-model={props.model} data-cape={props.cape ?? ''} data-playing={String(props.playing)} data-elytra={String(!!props.elytra)} />,
  };
});
// The real renderer needs WebGL. The stand-in answers with a picture named after what it was asked to draw.
const renderer = vi.hoisted(() => ({ fail: false, requests: [] as { key: string; yaw: number; cape: string | null }[] }));
vi.mock('../src/skin-renderer', () => ({
  BUST_FRAME: { width: 192, height: 216, zoom: 2.3, lift: -10 },
  cachedThumbnail: () => undefined,
  renderThumbnail: async (request: { key: string; yaw: number; cape: string | null }) => {
    renderer.requests.push({ key: request.key, yaw: request.yaw, cape: request.cape });
    if (renderer.fail) throw new Error('no webgl');
    return `data:image/png;base64,${btoa(request.key)}`;
  },
}));

const hash = (letter: string) => letter.repeat(64);
const image = (key: string) => `data:image/png;base64,${btoa(key)}`;
const account = (kind: 'microsoft' | 'offline'): AccountView => ({
  metadata: { account_id: 'acc', kind, username: 'PlayerOne', profile_id: 'p1', oauth_client_id: kind === 'microsoft' ? 'client' : null },
  status: kind === 'microsoft' ? 'ready' : 'offline', error: null, persisted: true,
});
const look = (id: string, overrides: Partial<SkinsView['library'][number]> = {}) => ({ id, name: `Look ${id}`, model: 'classic' as const, texture: hash(id), cape: null, createdMs: 1, ...overrides });

let view: SkinsView;
let failure: Record<string, unknown>;
let issued: number;

function baseView(): SkinsView {
  return {
    profileId: 'p1', profileName: 'PlayerOne', syncedMs: Date.now(), incomplete: false, matchingLook: null,
    current: { custom: true, texture: hash('a'), model: 'classic', cape: 'cape-a' },
    capes: [{ id: 'cape-a', alias: 'Migrator', cached: true, active: true }, { id: 'cape-b', alias: 'Vanilla', cached: true, active: false }],
    library: [look('b'), look('c', { model: 'slim', cape: 'cape-b' })],
  };
}
const calls = (command: string) => backend.invoke.mock.calls.filter(([name]) => name === command).map(([, args]) => args);
const viewer = () => screen.getByTestId('viewer');
const mount = (kind: 'microsoft' | 'offline' = 'microsoft') => render(<TooltipProvider><SkinsPage account={account(kind)} onBack={() => {}} /></TooltipProvider>);

beforeEach(() => {
  backend.native = true;
  view = baseView();
  failure = {};
  issued = 0;
  renderer.fail = false; renderer.requests = [];
  localStorage.clear();
  backend.invoke.mockReset();
  backend.invoke.mockImplementation(async (command: string, args: Record<string, unknown> = {}) => {
    if (command in failure) throw failure[command];
    switch (command) {
      case 'skins_snapshot': case 'skins_sync': return view;
      case 'skin_texture': return new TextEncoder().encode(String(args.key)).buffer;
      case 'skins_apply': {
        const request = args.request as { lookId: string | null; capeChange?: { id: string | null } };
        const chosen = view.library.find(item => item.id === request.lookId);
        const cape = request.capeChange ? request.capeChange.id : view.current?.cape ?? null;
        const current = { custom: true, texture: chosen?.texture ?? view.current?.texture ?? null, model: chosen?.model ?? view.current?.model ?? 'classic' as const, cape };
        view = { ...view, current, capes: view.capes.map(item => ({ ...item, active: item.id === cape })),
          matchingLook: view.library.find(item => item.texture === current.texture && item.model === current.model && item.cape === current.cape)?.id ?? null };
        return view;
      }
      case 'skins_add_look': {
        const id = `new${(issued += 1)}`;
        view = { ...view, library: [look(id, { name: String(args.name), model: args.model as 'classic' | 'slim', cape: args.cape as string | null, texture: hash('n') }), ...view.library] };
        return { id, view };
      }
      case 'skins_save_current': {
        const saved = look('s', { name: String(args.name), texture: view.current?.texture ?? hash('a'), cape: view.current?.cape ?? null });
        view = { ...view, library: [saved, ...view.library], matchingLook: 's' };
        return { id: 's', view };
      }
      case 'skins_delete_look': view = { ...view, library: view.library.filter(item => item.id !== args.id) }; return view;
      default: throw new Error(`Unexpected test command: ${command}`);
    }
  });
});
afterEach(cleanup);

test('the library opens from disk first and then synchronises with the account', async () => {
  mount();
  expect(await screen.findByRole('button', { name: /^Look b/ })).toBeTruthy();
  await waitFor(() => expect(calls('skins_sync')).toHaveLength(1));
  expect(calls('skins_snapshot')).toHaveLength(1);
  expect(viewer().dataset.skin).toBe(image(hash('a')));
  expect(viewer().dataset.cape).toBe(image('cape-a'));
  expect(viewer().dataset.model).toBe('classic');
  expect(screen.getByRole('heading', { name: 'Aspectos · PlayerOne' })).toBeTruthy();
});

test('an offline account never reaches the backend and is told why', async () => {
  mount('offline');
  expect(await screen.findByText('Necesitas una cuenta de Microsoft')).toBeTruthy();
  expect(backend.invoke).not.toHaveBeenCalled();
  expect(screen.queryByRole('button', { name: /^Nueva skin/ })).toBeNull();
});

test('outside the desktop app the page explains where it works', async () => {
  backend.native = false;
  mount();
  expect(await screen.findByText('Disponible en la aplicación de escritorio')).toBeTruthy();
  expect(backend.invoke).not.toHaveBeenCalled();
});

test('previewing a saved look updates the 3D view at once and applying sends exactly what changed', async () => {
  const user = userEvent.setup();
  mount();
  await user.click(await screen.findByRole('button', { name: /^Look c/ }));
  expect(viewer().dataset.skin).toBe(image(hash('c')));
  expect(viewer().dataset.model).toBe('slim');
  await waitFor(() => expect(viewer().dataset.cape).toBe(image('cape-b')));
  expect(screen.getByText('Vista previa')).toBeTruthy();
  expect(calls('skins_apply')).toHaveLength(0);
  await user.click(screen.getByRole('button', { name: 'Aplicar' }));
  await waitFor(() => expect(calls('skins_apply')).toEqual([{ request: { lookId: 'c', capeChange: { id: 'cape-b' } } }]));
  expect(await screen.findByText('Cambios aplicados a tu cuenta.')).toBeTruthy();
  expect(screen.queryByText('Vista previa')).toBeNull();
  expect(screen.getByRole('img', { name: 'Skin activa' })).toBeTruthy();
});

test('toggling a cape in the Capas tab previews it and discarding returns to the account', async () => {
  const user = userEvent.setup();
  mount();
  await screen.findByRole('button', { name: /^Look b/ });
  await user.click(screen.getByRole('tab', { name: /^Capas/ }));
  expect(screen.getByRole('button', { name: 'Migrator' }).getAttribute('aria-pressed')).toBe('true');
  expect(screen.getByRole('img', { name: 'Capa activa' })).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'Vanilla' }));
  expect(viewer().dataset.cape).toBe(image('cape-b'));
  expect(screen.getByText('Vista previa')).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'Sin capa' }));
  expect(viewer().dataset.cape).toBe('');
  await user.click(screen.getByRole('button', { name: 'Descartar' }));
  expect(viewer().dataset.cape).toBe(image('cape-a'));
  expect(calls('skins_apply')).toHaveLength(0);
  await user.click(screen.getByRole('button', { name: 'Sin capa' }));
  await user.click(screen.getByRole('button', { name: 'Aplicar' }));
  await waitFor(() => expect(calls('skins_apply')).toEqual([{ request: { lookId: null, capeChange: { id: null } } }]));
});

test('the tabs count what the account has and a search narrows the open tab only', async () => {
  const user = userEvent.setup();
  view = { ...baseView(), library: [look('b', { name: 'Caballero' }), look('c', { name: 'Árbol' }), look('d', { name: 'Arquero' })] };
  mount();
  await screen.findByRole('button', { name: 'Caballero' });
  expect(within(screen.getByRole('tab', { name: /^Skins/ })).getByText('3')).toBeTruthy();
  expect(within(screen.getByRole('tab', { name: /^Capas/ })).getByText('2')).toBeTruthy();
  expect(screen.getByRole('button', { name: /^Nueva skin/ })).toBeTruthy();
  // Accents and case do not matter, and the card for a new skin steps aside while searching.
  await user.type(screen.getByRole('searchbox', { name: 'Buscar skin' }), 'ARB');
  expect(screen.getByRole('button', { name: 'Árbol' })).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Caballero' })).toBeNull();
  expect(screen.queryByRole('button', { name: /^Nueva skin/ })).toBeNull();
  await user.clear(screen.getByRole('searchbox', { name: 'Buscar skin' }));
  await user.type(screen.getByRole('searchbox', { name: 'Buscar skin' }), 'zzz');
  expect(screen.getByText('Nada coincide con «zzz».')).toBeTruthy();
  // Changing tab clears the search, and the capes have their own.
  await user.click(screen.getByRole('tab', { name: /^Capas/ }));
  expect((screen.getByRole('searchbox', { name: 'Buscar capa' }) as HTMLInputElement).value).toBe('');
  await user.type(screen.getByRole('searchbox', { name: 'Buscar capa' }), 'van');
  expect(screen.getByRole('button', { name: 'Vanilla' })).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Migrator' })).toBeNull();
});

test('the preview walks by default and the button pauses and resumes it', async () => {
  const user = userEvent.setup();
  mount();
  await screen.findByRole('button', { name: /^Look b/ });
  expect(viewer().dataset.playing).toBe('true');
  // There is no animation picker any more, and no automatic turning to switch.
  expect(screen.queryByRole('radiogroup', { name: 'Animación' })).toBeNull();
  expect(screen.queryByRole('button', { name: 'Giro automático' })).toBeNull();
  await user.click(screen.getByRole('button', { name: 'Pausar animación' }));
  expect(viewer().dataset.playing).toBe('false');
  await user.click(screen.getByRole('button', { name: 'Reanudar animación' }));
  expect(viewer().dataset.playing).toBe('true');
});

test('saved skins and capes are 3D pictures at the angles of the preview', async () => {
  mount();
  await waitFor(() => expect(document.querySelectorAll('.skin-card img.skin-picture').length).toBe(2));
  const skins = renderer.requests.filter(request => request.key.startsWith('skin|'));
  expect(skins.map(request => request.key).sort()).toEqual([`skin|${hash('a')}|classic`, `skin|${hash('b')}|classic`, `skin|${hash('c')}|slim`].sort());
  expect(new Set(skins.map(request => request.yaw)).size).toBe(1);
  expect(skins.every(request => request.cape === null)).toBe(true);
  expect(document.querySelector('.skin-card img.skin-picture')!.getAttribute('src')).toMatch(/^data:image\/png;base64,/);
  const user = userEvent.setup();
  await user.click(screen.getByRole('tab', { name: /^Capas/ }));
  await waitFor(() => expect(document.querySelectorAll('.skin-card img.skin-picture').length).toBe(2));
  const capes = renderer.requests.filter(request => request.key.startsWith('cape|'));
  // Capes hang on the skin the account wears, and are seen from the other side than the skins.
  expect(capes.map(request => request.cape).sort()).toEqual([image('cape-a'), image('cape-b')]);
  expect(capes.every(request => request.key.includes(hash('a')))).toBe(true);
  expect(capes[0].yaw).not.toBe(skins[0].yaw);
});

test('without WebGL the cards keep their flat pictures and everything still works', async () => {
  renderer.fail = true;
  const user = userEvent.setup();
  mount();
  await screen.findByRole('button', { name: /^Look b/ });
  await waitFor(() => expect(renderer.requests.length).toBeGreaterThan(0));
  expect(document.querySelectorAll('.skin-card img.skin-picture')).toHaveLength(0);
  expect(document.querySelectorAll('.skin-card .skin-thumbnail').length).toBeGreaterThan(0);
  await user.click(screen.getByRole('button', { name: /^Look c/ }));
  expect(viewer().dataset.model).toBe('slim');
});

test('a current look that is not saved can be saved, and then stops being offered', async () => {
  const user = userEvent.setup();
  mount();
  expect(await screen.findByText('Aspecto actual sin guardar')).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'Guardar' }));
  const dialog = await screen.findByRole('dialog', { name: 'Guardar en mi biblioteca' });
  const name = within(dialog).getByLabelText('Nombre') as HTMLInputElement;
  expect(name.value).toBe('Skin de PlayerOne');
  await user.clear(name);
  await user.type(name, 'Mi aspecto');
  await user.click(within(dialog).getByRole('button', { name: 'Guardar' }));
  await waitFor(() => expect(calls('skins_save_current')).toEqual([{ name: 'Mi aspecto' }]));
  await waitFor(() => expect(screen.queryByText('Aspecto actual sin guardar')).toBeNull());
  expect(screen.getByRole('button', { name: /^Mi aspecto/ })).toBeTruthy();
});

test('a look that already matches the account is not offered for saving', async () => {
  view = { ...baseView(), library: [look('b', { texture: hash('a'), cape: 'cape-a' })] };
  mount();
  await screen.findByRole('button', { name: /^Look b/ });
  expect(screen.queryByText('Aspecto actual sin guardar')).toBeNull();
});

test('adding a skin previews the file, model and cape before saving and can apply it in one step', async () => {
  const user = userEvent.setup({ applyAccept: false });
  mount();
  await user.click(await screen.findByRole('button', { name: /^Nueva skin/ }));
  const dialog = await screen.findByRole('dialog', { name: 'Añadir skin' });
  const scope = within(dialog);
  expect((scope.getByRole('button', { name: 'Guardar' }) as HTMLButtonElement).disabled).toBe(true);
  expect((scope.getByRole('button', { name: 'Guardar y aplicar' }) as HTMLButtonElement).disabled).toBe(true);

  // A file that is not a PNG is refused in the interface, before anything is sent.
  const input = scope.getByLabelText('Archivo de skin') as HTMLInputElement;
  await user.upload(input, new File(['x'], 'foto.jpg', { type: 'image/jpeg' }));
  expect(await scope.findByText('Elige un archivo PNG.')).toBeTruthy();

  const bytes = new Uint8Array([137, 80, 78, 71]);
  await user.upload(input, new File([bytes], 'mi-skin.png', { type: 'image/png' }));
  expect(await scope.findByText('mi-skin.png')).toBeTruthy();
  expect(scope.getByTestId('viewer').dataset.skin).toBe(`data:image/png;base64,${btoa(String.fromCharCode(...bytes))}`);
  expect(scope.getByTestId('viewer').dataset.model).toBe('classic');

  await user.click(scope.getByRole('radio', { name: 'Delgado (Alex)' }));
  expect(scope.getByTestId('viewer').dataset.model).toBe('slim');
  await user.click(scope.getByRole('button', { name: /Vanilla/ }));
  expect(scope.getByTestId('viewer').dataset.cape).toBe(image('cape-b'));
  await user.clear(scope.getByLabelText('Nombre'));
  await user.type(scope.getByLabelText('Nombre'), 'Mi skin');
  await user.click(scope.getByRole('button', { name: 'Guardar y aplicar' }));

  await waitFor(() => expect(calls('skins_add_look')).toEqual([{ name: 'Mi skin', model: 'slim', cape: 'cape-b', png: btoa(String.fromCharCode(...bytes)) }]));
  await waitFor(() => expect(calls('skins_apply')).toEqual([{ request: { lookId: 'new1' } }]));
  expect(await screen.findByText('Skin guardada y aplicada a tu cuenta.')).toBeTruthy();
  expect(screen.queryByRole('dialog')).toBeNull();
});

test('a rejected skin file keeps the dialog open with the reason and saves nothing', async () => {
  const user = userEvent.setup({ applyAccept: false });
  failure = { skins_add_look: 'invalid_skin' };
  mount();
  await user.click(await screen.findByRole('button', { name: /^Nueva skin/ }));
  const dialog = await screen.findByRole('dialog', { name: 'Añadir skin' });
  await user.upload(within(dialog).getByLabelText('Archivo de skin'), new File([new Uint8Array([1, 2, 3])], 'rota.png', { type: 'image/png' }));
  await user.click(within(dialog).getByRole('button', { name: 'Guardar' }));
  expect(await within(dialog).findByText(/no es una skin válida/)).toBeTruthy();
  expect(screen.getByRole('dialog', { name: 'Añadir skin' })).toBeTruthy();
  expect(calls('skins_apply')).toHaveLength(0);
});

test('editing keeps the saved image unless a new file is chosen', async () => {
  const user = userEvent.setup();
  backend.invoke.mockImplementation(async (command: string, args: Record<string, unknown> = {}) => {
    if (command === 'skins_snapshot' || command === 'skins_sync') return view;
    if (command === 'skin_texture') return new TextEncoder().encode(String(args.key)).buffer;
    if (command === 'skins_edit_look') { view = { ...view, library: view.library.map(item => item.id === args.id ? { ...item, name: String(args.name), model: args.model as 'classic' | 'slim', cape: args.cape as string | null } : item) }; return view; }
    throw new Error(`Unexpected test command: ${command}`);
  });
  mount();
  await user.click(await screen.findByRole('button', { name: 'Más acciones para Look c' }));
  await user.click(await screen.findByRole('menuitem', { name: 'Editar' }));
  const dialog = await screen.findByRole('dialog', { name: 'Editar skin' });
  const scope = within(dialog);
  expect((scope.getByLabelText('Nombre') as HTMLInputElement).value).toBe('Look c');
  expect((scope.getByRole('radio', { name: 'Delgado (Alex)' }) as HTMLInputElement).checked).toBe(true);
  expect(scope.getByTestId('viewer').dataset.cape).toBe(image('cape-b'));
  await user.click(scope.getByRole('radio', { name: 'Clásico (Steve)' }));
  expect(scope.getByTestId('viewer').dataset.model).toBe('classic');
  await user.click(scope.getByRole('button', { name: 'Guardar' }));
  await waitFor(() => expect(calls('skins_edit_look')).toEqual([{ id: 'c', name: 'Look c', model: 'classic', cape: 'cape-b', png: null }]));
});

test('deleting asks first, says the account is untouched and then removes the look', async () => {
  const user = userEvent.setup();
  mount();
  await user.click(await screen.findByRole('button', { name: 'Más acciones para Look b' }));
  await user.click(await screen.findByRole('menuitem', { name: 'Eliminar' }));
  const dialog = await screen.findByRole('dialog', { name: 'Eliminar de la biblioteca' });
  expect(within(dialog).getByText(/no cambia/)).toBeTruthy();
  expect(calls('skins_delete_look')).toHaveLength(0);
  await user.click(within(dialog).getByRole('button', { name: 'Cancelar' }));
  expect(calls('skins_delete_look')).toHaveLength(0);
  await user.click(await screen.findByRole('button', { name: 'Más acciones para Look b' }));
  await user.click(await screen.findByRole('menuitem', { name: 'Eliminar' }));
  await user.click(within(await screen.findByRole('dialog', { name: 'Eliminar de la biblioteca' })).getByRole('button', { name: 'Eliminar' }));
  await waitFor(() => expect(calls('skins_delete_look')).toEqual([{ id: 'b' }]));
  await waitFor(() => expect(screen.queryByRole('button', { name: /^Look b/ })).toBeNull());
});

test('a failed synchronisation keeps the saved library and says so', async () => {
  failure = { skins_sync: 'network_unavailable' };
  mount();
  expect(await screen.findByRole('button', { name: /^Look b/ })).toBeTruthy();
  expect(await screen.findByText(/No pudimos sincronizar con Minecraft/)).toBeTruthy();
  expect(viewer().dataset.skin).toBe(image(hash('a')));
});

test('a failed apply is reported and the page shows what the backend now holds', async () => {
  const user = userEvent.setup();
  mount();
  await user.click(await screen.findByRole('button', { name: /^Look c/ }));
  failure = { skins_apply: 'service_unavailable' };
  await user.click(screen.getByRole('button', { name: 'Aplicar' }));
  expect(await screen.findByText(/no está disponible por ahora/)).toBeTruthy();
  expect(calls('skins_snapshot').length).toBeGreaterThanOrEqual(2);
});

test('an account without a skin of its own says so instead of showing an empty viewer', async () => {
  view = { ...baseView(), current: { custom: false, texture: null, model: 'classic', cape: null }, capes: [], library: [] };
  mount();
  expect(await screen.findByText(/skin predeterminada de Minecraft/)).toBeTruthy();
  expect(screen.queryByTestId('viewer')).toBeNull();
  expect(screen.getByText('Aún no has guardado ninguna skin. Elige «Nueva skin» para añadir un archivo PNG.')).toBeTruthy();
});

test('right-clicking a card opens its own menu instead of the browser one', async () => {
  const user = userEvent.setup();
  mount();
  await user.pointer({ keys: '[MouseRight]', target: await screen.findByRole('button', { name: 'Look b' }) });
  const menu = await screen.findByRole('menu', { name: 'Acciones de Look b' });
  expect(within(menu).getAllByRole('menuitem').map(item => item.textContent)).toEqual(['Vista previa', 'Aplicar', 'Editar', 'Eliminar']);
  await user.click(within(menu).getByRole('menuitem', { name: 'Aplicar' }));
  await waitFor(() => expect(calls('skins_apply')).toEqual([{ request: { lookId: 'b' } }]));
  // Another choice from the same kind of menu previews, and one that edits opens its dialog.
  await user.pointer({ keys: '[MouseRight]', target: screen.getByRole('button', { name: 'Look c' }) });
  await user.click(within(await screen.findByRole('menu', { name: 'Acciones de Look c' })).getByRole('menuitem', { name: 'Vista previa' }));
  expect(viewer().dataset.model).toBe('slim');
  await user.pointer({ keys: '[MouseRight]', target: screen.getByRole('button', { name: 'Look c' }) });
  await user.click(within(await screen.findByRole('menu', { name: 'Acciones de Look c' })).getByRole('menuitem', { name: 'Editar' }));
  expect(await screen.findByRole('dialog', { name: 'Editar skin' })).toBeTruthy();
});

test('the capes, the empty cape and the new skin card have menus too', async () => {
  const user = userEvent.setup();
  mount();
  await user.pointer({ keys: '[MouseRight]', target: await screen.findByRole('button', { name: /^Nueva skin/ }) });
  await user.click(within(await screen.findByRole('menu', { name: 'Acciones de Nueva skin' })).getByRole('menuitem', { name: 'Añadir skin' }));
  expect(await screen.findByRole('dialog', { name: 'Añadir skin' })).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'Cancelar' }));

  await user.click(screen.getByRole('tab', { name: /^Capas/ }));
  await user.pointer({ keys: '[MouseRight]', target: screen.getByRole('button', { name: 'Vanilla' }) });
  await user.click(within(await screen.findByRole('menu', { name: 'Acciones de Vanilla' })).getByRole('menuitem', { name: 'Aplicar' }));
  await waitFor(() => expect(calls('skins_apply')).toEqual([{ request: { lookId: null, capeChange: { id: 'cape-b' } } }]));
  // The cape that is already worn cannot be applied again.
  await user.pointer({ keys: '[MouseRight]', target: screen.getByRole('button', { name: 'Vanilla' }) });
  expect(within(await screen.findByRole('menu', { name: 'Acciones de Vanilla' })).getByRole('menuitem', { name: 'Aplicar' }).getAttribute('aria-disabled')).toBe('true');
});

test('the reminder to save can be dismissed for that skin, is remembered, and another skin asks again', async () => {
  const user = userEvent.setup();
  mount();
  expect(await screen.findByText('Aspecto actual sin guardar')).toBeTruthy();
  expect(screen.getByText('Añádelo a tu biblioteca para volver a usarlo cuando quieras.')).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'Ocultar este aviso' }));
  expect(screen.queryByText('Aspecto actual sin guardar')).toBeNull();
  cleanup();
  mount();
  await screen.findByRole('button', { name: /^Look b/ });
  expect(screen.queryByText('Aspecto actual sin guardar')).toBeNull();
  cleanup();
  view = { ...baseView(), current: { ...baseView().current!, texture: hash('f') } };
  mount();
  expect(await screen.findByText('Aspecto actual sin guardar')).toBeTruthy();
});

test('a saved skin worn with a different cape is not offered for saving, and its card is the active one', async () => {
  view = { ...baseView(), library: [look('x', { texture: hash('a'), cape: null }), look('y', { texture: hash('c'), model: 'slim' })] };
  mount();
  await screen.findByRole('button', { name: /^Look x/ });
  expect(screen.queryByText('Aspecto actual sin guardar')).toBeNull();
  const card = screen.getByRole('button', { name: 'Look x' }).closest('.skin-card-body')!;
  expect(within(card as HTMLElement).getByRole('img', { name: 'Skin activa' })).toBeTruthy();
  expect(screen.getAllByRole('img', { name: 'Skin activa' })).toHaveLength(1);
});

test('the elytra switch needs a cape and shows the cape as elytra in the preview', async () => {
  const user = userEvent.setup();
  mount();
  await screen.findByRole('button', { name: /^Look b/ });
  const toggle = () => screen.getByRole('switch', { name: 'Elytra' }) as HTMLInputElement;
  await waitFor(() => expect(viewer().dataset.cape).toBe(image('cape-a')));
  expect(toggle().disabled).toBe(false);
  expect(viewer().dataset.elytra).toBe('false');
  await user.click(toggle());
  expect(viewer().dataset.elytra).toBe('true');
  // Without a cape there is nothing to make the elytra from.
  await user.click(screen.getByRole('tab', { name: /^Capas/ }));
  await user.click(screen.getByRole('button', { name: 'Sin capa' }));
  expect(toggle().disabled).toBe(true);
  expect(toggle().checked).toBe(false);
  expect(viewer().dataset.elytra).toBe('false');
  await user.click(screen.getByRole('button', { name: 'Migrator' }));
  await waitFor(() => expect(viewer().dataset.cape).toBe(image('cape-a')));
  expect(toggle().checked).toBe(true);
});
