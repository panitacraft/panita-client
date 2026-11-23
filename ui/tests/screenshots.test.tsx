import { act, cleanup, render, screen, waitFor, within } from '@testing-library/react';
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
  list = { screenshots: shots(2), truncated: false };
  failure = {};
  backend.invoke.mockReset();
  backend.invoke.mockImplementation(async (command: string) => {
    if (command in failure) throw failure[command];
    switch (command) {
      case 'list_screenshots': return list;
      case 'screenshot_image': return png;
      case 'open_screenshot': case 'reveal_screenshot': case 'copy_screenshot': return true;
      default: throw new Error(`Unexpected test command: ${command}`);
    }
  });
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

test('an image is requested only when its card becomes visible', async () => {
  view.visible = false;
  mount();
  await screen.findAllByRole('listitem');
  expect(backend.invoke.mock.calls.some(([command]) => command === 'screenshot_image')).toBe(false);
  await act(async () => { view.observers.forEach(show => show()); });
  await waitFor(() => expect(backend.invoke.mock.calls.filter(([command]) => command === 'screenshot_image').length).toBe(2));
});

test('an instance without captures shows an empty state and still allows a refresh', async () => {
  list = { screenshots: [], truncated: false };
  mount();
  expect(await screen.findByRole('heading', { name: 'Aún no hay capturas' })).toBeTruthy();
  expect(screen.queryByRole('list', { name: 'Capturas de la instancia' })).toBeNull();
  expect(backend.invoke.mock.calls.some(([command]) => command === 'screenshot_image')).toBe(false);
  list = { screenshots: shots(1), truncated: false };
  await userEvent.setup().click(screen.getByRole('button', { name: 'Actualizar' }));
  expect(await screen.findByRole('list', { name: 'Capturas de la instancia' })).toBeTruthy();
});

test('captures are listed with their image, name and size, scoped to the instance', async () => {
  mount();
  const cards = await screen.findAllByRole('listitem');
  expect(cards.length).toBe(2);
  expect(backend.invoke).toHaveBeenCalledWith('list_screenshots', { instanceId: 'pack-a' });
  const image = await within(cards[0]).findByRole('img');
  expect(image.getAttribute('src')).toMatch(/^data:image\/png;base64,/);
  expect(within(cards[0]).getByText(/MB/)).toBeTruthy();
  expect(backend.invoke).toHaveBeenCalledWith('screenshot_image', { instanceId: 'pack-a', name: list.screenshots[0].name });
  expect(screen.getByText('2 capturas de esta instancia, de la más reciente a la más antigua.')).toBeTruthy();
});

test('each capture can be opened, shown in the file manager and copied by name only', async () => {
  const user = userEvent.setup(); mount();
  const [first] = await screen.findAllByRole('listitem');
  const name = list.screenshots[0].name;
  await user.click(within(first).getByRole('button', { name: `Abrir ${name} en el visor de imágenes` }));
  await user.click(within(first).getByRole('button', { name: `Mostrar ${name} en el explorador de archivos` }));
  await user.click(within(first).getByRole('button', { name: `Copiar la imagen ${name} al portapapeles` }));
  for (const command of ['open_screenshot', 'reveal_screenshot', 'copy_screenshot']) {
    expect(backend.invoke).toHaveBeenCalledWith(command, { instanceId: 'pack-a', name });
  }
  expect(await screen.findByText('Imagen copiada al portapapeles.')).toBeTruthy();
});

test('a failed action is reported in Spanish, and a vanished file refreshes the list', async () => {
  const user = userEvent.setup(); mount();
  const [first] = await screen.findAllByRole('listitem');
  const name = list.screenshots[0].name;
  failure = { copy_screenshot: 'storage_unavailable' };
  await user.click(within(first).getByRole('button', { name: `Copiar la imagen ${name} al portapapeles` }));
  expect((await screen.findByRole('alert')).textContent).toMatch(/No pudimos copiar la imagen al portapapeles/);
  failure = { open_screenshot: 'invalid_data' };
  const before = backend.invoke.mock.calls.filter(([command]) => command === 'list_screenshots').length;
  await user.click(within(first).getByRole('button', { name: `Abrir ${name} en el visor de imágenes` }));
  expect((await screen.findByRole('alert')).textContent).toMatch(/ya no está disponible/);
  await waitFor(() => expect(backend.invoke.mock.calls.filter(([command]) => command === 'list_screenshots').length).toBe(before + 1));
});

test('a failed listing offers a retry and a failed image keeps the card usable', async () => {
  failure = { list_screenshots: 'storage_unavailable' };
  const user = userEvent.setup(); mount();
  expect((await screen.findByRole('alert')).textContent).toMatch(/No pudimos leer las capturas/);
  failure = { screenshot_image: 'invalid_data' };
  await user.click(screen.getByRole('button', { name: 'Reintentar' }));
  expect(await screen.findAllByText('No se pudo cargar la imagen')).toHaveLength(2);
  expect(screen.getAllByRole('button', { name: /^Abrir / }).length).toBe(2);
});

test('many captures are paged so only one page of images is requested', async () => {
  list = { screenshots: shots(30), truncated: true };
  const user = userEvent.setup(); mount();
  expect((await screen.findAllByRole('listitem')).length).toBe(12);
  expect(screen.getByText('Página 1 de 3')).toBeTruthy();
  expect(screen.getByText('Mostramos las 30 capturas más recientes. Las demás siguen en tu carpeta.')).toBeTruthy();
  await waitFor(() => expect(backend.invoke.mock.calls.filter(([command]) => command === 'screenshot_image').length).toBe(12));
  await user.click(screen.getByRole('button', { name: 'Siguiente' }));
  expect(screen.getByText('Página 2 de 3')).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Anterior' })).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'Siguiente' }));
  expect((await screen.findAllByRole('listitem')).length).toBe(6);
  expect((screen.getByRole('button', { name: 'Siguiente' }) as HTMLButtonElement).disabled).toBe(true);
});
