import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { WindowControls } from '../src/components/window-controls';
import { TooltipProvider } from '../src/components/ui/tooltip';

const desktop = vi.hoisted(() => ({ native: true, maximized: false, invoke: vi.fn(), resize: null as null | (() => void), stop: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => desktop.native, invoke: desktop.invoke }));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ onResized: async (callback: () => void) => { desktop.resize = callback; return desktop.stop; } }) }));
beforeEach(() => {
  desktop.native = true; desktop.maximized = false; desktop.resize = null;
  desktop.invoke.mockImplementation(async (_: string, args: { action: string }) => {
    if (args.action === 'toggle_maximize') desktop.maximized = !desktop.maximized;
    return args.action === 'state' || args.action === 'toggle_maximize' ? desktop.maximized : null;
  });
});
afterEach(cleanup);
function mount() { return render(<TooltipProvider><WindowControls /></TooltipProvider>); }

test('window controls minimize, toggle maximize and restore, and request the guarded close', async () => {
  const user = userEvent.setup(); mount();
  await waitFor(() => expect(desktop.invoke).toHaveBeenCalledWith('main_window_control', { action: 'state' }));
  await user.click(screen.getByRole('button', { name: 'Minimizar', exact: true }));
  expect(desktop.invoke).toHaveBeenCalledWith('main_window_control', { action: 'minimize' });
  await user.click(screen.getByRole('button', { name: 'Maximizar', exact: true }));
  await user.click(await screen.findByRole('button', { name: 'Restaurar', exact: true }));
  expect(await screen.findByRole('button', { name: 'Maximizar', exact: true })).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'Cerrar', exact: true }));
  expect(desktop.invoke).toHaveBeenLastCalledWith('main_window_control', { action: 'close' });
});

test('native resize changes refresh the restore icon and unsubscribe when unmounted', async () => {
  const { unmount } = mount();
  await waitFor(() => expect(desktop.resize).not.toBeNull());
  desktop.maximized = true;
  await act(async () => desktop.resize?.());
  expect(await screen.findByRole('button', { name: 'Restaurar', exact: true })).toBeTruthy();
  unmount(); expect(desktop.stop).toHaveBeenCalledTimes(1);
});

test('browser previews expose disabled window controls without issuing desktop requests', async () => {
  desktop.native = false; const user = userEvent.setup(); mount();
  for (const name of ['Minimizar', 'Maximizar', 'Cerrar']) {
    const button = screen.getByRole('button', { name, exact: true }) as HTMLButtonElement;
    expect(button.disabled).toBe(true); await user.click(button);
  }
  expect(desktop.invoke).not.toHaveBeenCalled();
});

test('failed window operations report a Spanish error and leave retry available', async () => {
  const user = userEvent.setup(); mount();
  await waitFor(() => expect(desktop.invoke).toHaveBeenCalled());
  desktop.invoke.mockRejectedValueOnce(new Error('native failure'));
  await user.click(screen.getByRole('button', { name: 'Minimizar', exact: true }));
  expect(await screen.findByRole('alert')).toHaveProperty('textContent', 'No pudimos controlar la ventana. Vuelve a intentarlo.');
  expect((screen.getByRole('button', { name: 'Minimizar', exact: true }) as HTMLButtonElement).disabled).toBe(false);
});
