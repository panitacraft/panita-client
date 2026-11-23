import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { ScreenshotPanel } from '../src/components/screenshot-panel';
import { TooltipProvider } from '../src/components/ui/tooltip';
import type { ScreenshotList } from '../src/screenshot-state';

const backend = vi.hoisted(() => ({ native: true, invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => backend.native, invoke: backend.invoke }));

const png = new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]).buffer;
let list: ScreenshotList;
let failure: Record<string, unknown>;

function shots(count: number) {
  return Array.from({ length: count }, (_, index) => ({ name: `2026-11-23_10.${String(59 - index).padStart(2, '0')}.00.png`, size: 2_000_000, modifiedMs: 1_700_000_000_000 - index * 60_000 }));
}
function mount() { return render(<TooltipProvider><ScreenshotPanel instanceId="pack-a" /></TooltipProvider>); }
const calls = (command: string) => backend.invoke.mock.calls.filter(([name]) => name === command);
const thumbnail = (name: string) => screen.findByRole('button', { name: `Ver ${name}` });
const viewer = () => screen.getByRole('dialog', { name: /^Captura 2026/ });
async function rightClick(user: ReturnType<typeof userEvent.setup>, name: string) {
  await user.pointer({ keys: '[MouseRight]', target: await thumbnail(name) });
}

// Cards fetch their image when they scroll into view; the tests choose whether that happens.
const view = { visible: true, observers: [] as Array<() => void> };
class FakeIntersectionObserver {
  constructor(private callback: (entries: { isIntersecting: boolean }[]) => void) {}
  observe() {
    const show = () => this.callback([{ isIntersecting: true }]);
    if (view.visible) queueMicrotask(show); else view.observers.push(show);
  }
  disconnect() {}
}

beforeEach(() => {
  vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver);
  view.visible = true; view.observers = [];
  backend.native = true;
  list = { screenshots: shots(3), truncated: false };
  failure = {};
  backend.invoke.mockReset();
  backend.invoke.mockImplementation(async (command: string) => {
    if (command in failure) throw failure[command];
    switch (command) {
      case 'list_screenshots': return list;
      case 'screenshot_image': return png;
      case 'open_screenshot': case 'reveal_screenshot': case 'copy_screenshot': case 'delete_screenshot': return true;
      default: throw new Error(`Unexpected test command: ${command}`);
    }
  });
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

test('an image is requested only when its card becomes visible', async () => {
  view.visible = false;
  mount();
  await screen.findAllByRole('listitem');
  expect(calls('screenshot_image').length).toBe(0);
  await act(async () => { view.observers.forEach(show => show()); });
  await waitFor(() => expect(calls('screenshot_image').length).toBe(3));
});

test('an instance without captures shows an empty state and still allows a refresh', async () => {
  list = { screenshots: [], truncated: false };
  mount();
  expect(await screen.findByRole('heading', { name: 'Aún no hay capturas' })).toBeTruthy();
  expect(screen.queryByRole('list', { name: 'Capturas de la instancia' })).toBeNull();
  expect(calls('screenshot_image').length).toBe(0);
  list = { screenshots: shots(1), truncated: false };
  await userEvent.setup().click(screen.getByRole('button', { name: 'Actualizar' }));
  expect(await screen.findByRole('list', { name: 'Capturas de la instancia' })).toBeTruthy();
});

test('captures are listed with their image, centered name and size, scoped to the instance', async () => {
  const { container } = mount();
  const cards = await screen.findAllByRole('listitem');
  expect(cards.length).toBe(3);
  expect(backend.invoke).toHaveBeenCalledWith('list_screenshots', { instanceId: 'pack-a' });
  await waitFor(() => expect(cards[0].querySelector('img')?.getAttribute('src')).toMatch(/^data:image\/png;base64,/));
  expect(within(cards[0]).getByText(/MB/)).toBeTruthy();
  expect(container.querySelector('.screenshot-meta strong')?.textContent).toBe(list.screenshots[0].name);
  // The old per-card action buttons are gone; the actions live in the menu.
  expect(screen.queryByRole('button', { name: /^Abrir / })).toBeNull();
  expect(screen.getByText('3 capturas de esta instancia, de la más reciente a la más antigua.')).toBeTruthy();
});

test('right-clicking a capture offers its actions with simple words and runs them by name only', async () => {
  const user = userEvent.setup(); mount();
  const name = list.screenshots[0].name;
  await rightClick(user, name);
  expect(within(await screen.findByRole('menu')).getAllByRole('menuitem').map(item => item.textContent))
    .toEqual(['Ver', 'Abrir en el visor del sistema', 'Mostrar en la carpeta', 'Copiar imagen', 'Eliminar']);
  await user.click(screen.getByRole('menuitem', { name: 'Abrir en el visor del sistema' }));
  await rightClick(user, name);
  await user.click(await screen.findByRole('menuitem', { name: 'Mostrar en la carpeta' }));
  await rightClick(user, name);
  await user.click(await screen.findByRole('menuitem', { name: 'Copiar imagen' }));
  for (const command of ['open_screenshot', 'reveal_screenshot', 'copy_screenshot']) {
    expect(backend.invoke).toHaveBeenCalledWith(command, { instanceId: 'pack-a', name });
  }
  // A successful copy is confirmed with a toast.
  expect(await screen.findByText('Imagen copiada al portapapeles.')).toBeTruthy();
});

test('the overflow button opens the same menu for keyboard and touch', async () => {
  const user = userEvent.setup(); mount();
  const name = list.screenshots[1].name;
  await user.click(await screen.findByRole('button', { name: `Más acciones para ${name}` }));
  await user.click(await screen.findByRole('menuitem', { name: 'Copiar imagen' }));
  expect(backend.invoke).toHaveBeenCalledWith('copy_screenshot', { instanceId: 'pack-a', name });
});

test('a failed action is reported in a toast, and a vanished file refreshes the list', async () => {
  const user = userEvent.setup(); mount();
  const name = list.screenshots[0].name;
  failure = { copy_screenshot: 'storage_unavailable' };
  await rightClick(user, name);
  await user.click(await screen.findByRole('menuitem', { name: 'Copiar imagen' }));
  expect(await screen.findByText(/No pudimos copiar la imagen al portapapeles/)).toBeTruthy();
  failure = { open_screenshot: 'invalid_data' };
  const before = calls('list_screenshots').length;
  await rightClick(user, name);
  await user.click(await screen.findByRole('menuitem', { name: 'Abrir en el visor del sistema' }));
  expect(await screen.findByText(/ya no está disponible/)).toBeTruthy();
  await waitFor(() => expect(calls('list_screenshots').length).toBe(before + 1));
});

test('clicking a thumbnail opens the viewer, and Escape or the close button closes it', async () => {
  const user = userEvent.setup(); mount();
  await user.click(await thumbnail(list.screenshots[0].name));
  const dialog = viewer();
  expect(within(dialog).getByText(/1 de 3/)).toBeTruthy();
  await waitFor(() => expect(dialog.querySelector('img')?.getAttribute('src')).toMatch(/^data:image\/png/));
  fireEvent(dialog, new Event('cancel', { cancelable: true }));
  await waitFor(() => expect(screen.queryByRole('dialog', { name: /^Captura 2026/ })).toBeNull());
  await user.click(await thumbnail(list.screenshots[0].name));
  await user.click(within(viewer()).getByRole('button', { name: 'Cerrar' }));
  await waitFor(() => expect(screen.queryByRole('dialog', { name: /^Captura 2026/ })).toBeNull());
});

test('the viewer steps through every capture with the arrow keys and does not wrap', async () => {
  const user = userEvent.setup(); mount();
  await user.click(await thumbnail(list.screenshots[0].name));
  const dialog = viewer();
  const previous = within(dialog).getByRole('button', { name: 'Captura anterior' }) as HTMLButtonElement;
  const next = within(dialog).getByRole('button', { name: 'Captura siguiente' }) as HTMLButtonElement;
  expect(previous.disabled).toBe(true);
  fireEvent.keyDown(dialog, { key: 'ArrowRight' });
  expect(within(viewer()).getByText(/2 de 3/)).toBeTruthy();
  expect(viewer().getAttribute('aria-label')).toBe(`Captura ${list.screenshots[1].name}`);
  fireEvent.keyDown(viewer(), { key: 'ArrowRight' });
  fireEvent.keyDown(viewer(), { key: 'ArrowRight' });
  expect(within(viewer()).getByText(/3 de 3/)).toBeTruthy();
  expect((within(viewer()).getByRole('button', { name: 'Captura siguiente' }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.keyDown(viewer(), { key: 'ArrowLeft' });
  expect(within(viewer()).getByText(/2 de 3/)).toBeTruthy();
  await user.click(next);
  expect(within(viewer()).getByText(/3 de 3/)).toBeTruthy();
  // The neighbours were requested ahead of time and the current one is shown.
  await waitFor(() => expect(viewer().querySelector('img')).toBeTruthy());
});

test('the viewer has the quick actions and shows the copy toast inside itself', async () => {
  const user = userEvent.setup(); mount();
  const name = list.screenshots[0].name;
  await user.click(await thumbnail(name));
  await user.click(within(viewer()).getByRole('button', { name: 'Copiar imagen' }));
  expect(backend.invoke).toHaveBeenCalledWith('copy_screenshot', { instanceId: 'pack-a', name });
  // The page's toast sits under a modal dialog, so the viewer carries its own.
  expect(await within(viewer()).findByText('Imagen copiada al portapapeles.')).toBeTruthy();
  await user.click(within(viewer()).getByRole('button', { name: 'Mostrar en la carpeta' }));
  expect(backend.invoke).toHaveBeenCalledWith('reveal_screenshot', { instanceId: 'pack-a', name });
  await user.click(within(viewer()).getByRole('button', { name: 'Abrir en el visor del sistema' }));
  expect(backend.invoke).toHaveBeenCalledWith('open_screenshot', { instanceId: 'pack-a', name });
});

test('deleting asks first, names the file, and only moves it to the trash after confirming', async () => {
  const user = userEvent.setup(); mount();
  const name = list.screenshots[0].name;
  await rightClick(user, name);
  await user.click(await screen.findByRole('menuitem', { name: 'Eliminar' }));
  const confirm = await screen.findByRole('dialog', { name: 'Mover a la papelera' });
  expect(within(confirm).getByText(new RegExp(name.replace(/\./g, '\\.')))).toBeTruthy();
  // The safe choice has the focus, and cancelling changes nothing.
  await user.click(within(confirm).getByRole('button', { name: 'Cancelar' }));
  expect(calls('delete_screenshot').length).toBe(0);
  expect(screen.getAllByRole('listitem').length).toBe(3);
  await rightClick(user, name);
  await user.click(await screen.findByRole('menuitem', { name: 'Eliminar' }));
  await user.click(within(await screen.findByRole('dialog', { name: 'Mover a la papelera' })).getByRole('button', { name: 'Mover a la papelera' }));
  expect(backend.invoke).toHaveBeenCalledWith('delete_screenshot', { instanceId: 'pack-a', name });
  await waitFor(() => expect(screen.getAllByRole('listitem').length).toBe(2));
  expect(await screen.findByText('Captura movida a la papelera.')).toBeTruthy();
  expect(screen.queryByRole('button', { name: `Ver ${name}` })).toBeNull();
});

test('a failed deletion keeps the capture and says so', async () => {
  const user = userEvent.setup(); mount();
  failure = { delete_screenshot: 'storage_busy' };
  await rightClick(user, list.screenshots[0].name);
  await user.click(await screen.findByRole('menuitem', { name: 'Eliminar' }));
  await user.click(within(await screen.findByRole('dialog', { name: 'Mover a la papelera' })).getByRole('button', { name: 'Mover a la papelera' }));
  expect(await screen.findByText(/No pudimos mover la captura a la papelera/)).toBeTruthy();
  expect(screen.getAllByRole('listitem').length).toBe(3);
});

test('deleting from the viewer moves on to the next capture and closes after the last one', async () => {
  list = { screenshots: shots(2), truncated: false };
  const user = userEvent.setup(); mount();
  await user.click(await thumbnail(list.screenshots[0].name));
  await user.click(within(viewer()).getByRole('button', { name: 'Eliminar' }));
  await user.click(within(await screen.findByRole('dialog', { name: 'Mover a la papelera' })).getByRole('button', { name: 'Mover a la papelera' }));
  await waitFor(() => expect(viewer().getAttribute('aria-label')).toBe(`Captura ${list.screenshots[1].name}`));
  expect(within(viewer()).getByText(/1 de 1/)).toBeTruthy();
  await user.click(within(viewer()).getByRole('button', { name: 'Eliminar' }));
  await user.click(within(await screen.findByRole('dialog', { name: 'Mover a la papelera' })).getByRole('button', { name: 'Mover a la papelera' }));
  await waitFor(() => expect(screen.queryByRole('dialog', { name: /^Captura 2026/ })).toBeNull());
  expect(await screen.findByRole('heading', { name: 'Aún no hay capturas' })).toBeTruthy();
});

test('a failed listing offers a retry and a thumbnail that cannot load keeps its menu', async () => {
  failure = { list_screenshots: 'storage_unavailable' };
  const user = userEvent.setup(); mount();
  expect((await screen.findByRole('alert')).textContent).toMatch(/No pudimos leer las capturas/);
  failure = { screenshot_image: 'invalid_data' };
  await user.click(screen.getByRole('button', { name: 'Reintentar' }));
  expect(await screen.findAllByText('No se pudo cargar la imagen')).toHaveLength(3);
  await rightClick(user, list.screenshots[0].name);
  expect(await screen.findByRole('menuitem', { name: 'Eliminar' })).toBeTruthy();
});

test('many captures are paged so only one page of images is requested', async () => {
  list = { screenshots: shots(30), truncated: true };
  const user = userEvent.setup(); mount();
  expect((await screen.findAllByRole('listitem')).length).toBe(12);
  expect(screen.getByText('Página 1 de 3')).toBeTruthy();
  expect(screen.getByText('Mostramos las 30 capturas más recientes. Las demás siguen en tu carpeta.')).toBeTruthy();
  await waitFor(() => expect(calls('screenshot_image').length).toBe(12));
  await user.click(screen.getByRole('button', { name: 'Siguiente' }));
  expect(screen.getByText('Página 2 de 3')).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Anterior' })).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'Siguiente' }));
  expect((await screen.findAllByRole('listitem')).length).toBe(6);
  expect((screen.getByRole('button', { name: 'Siguiente' }) as HTMLButtonElement).disabled).toBe(true);
});
