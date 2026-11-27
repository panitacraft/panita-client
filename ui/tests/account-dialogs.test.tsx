import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { Accounts } from '../src/accounts';
import { ProfileArtwork } from '../src/components/profile-artwork';
import type { AccountsView } from '../src/account-state';

const backend = vi.hoisted(() => ({ invoke: vi.fn(), listeners: new Set<(event: { payload: AccountsView }) => void>() }));
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, invoke: backend.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: async (_: string, callback: (event: { payload: AccountsView }) => void) => {
  backend.listeners.add(callback); return () => backend.listeners.delete(callback);
} }));
let view: AccountsView;
beforeEach(() => {
  backend.listeners.clear();
  view = { revision: 1, accounts: [], selected_account_id: null, login_pending: false, login_error: null };
  backend.invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
    if (command === 'initialize') return;
    if (command === 'account_snapshot') return view;
    if (command === 'start_microsoft_login') { view = { ...view, revision: view.revision + 1, login_pending: true }; return 'fixture-operation'; }
    if (command === 'cancel_microsoft_login') { view = { ...view, revision: view.revision + 1, login_pending: false }; return; }
    if (command === 'microsoft_login_link') return 'https://login.microsoftonline.com/fixture';
    if (command === 'add_offline_account') return;
    if (command === 'select_account') { view = { ...view, revision: view.revision + 1, selected_account_id: String(args?.accountId) }; return; }
    throw new Error('Unexpected synthetic command: ' + command);
  });
});
afterEach(cleanup);

test('offline cancellation never submits and validation errors remain inside the dialog', async () => {
  const user = userEvent.setup(); render(<Accounts onboarding />);
  await waitFor(() => expect((screen.getByRole('button', { name: 'Añadir cuenta No-Premium' }) as HTMLButtonElement).disabled).toBe(false));
  await user.click(screen.getByRole('button', { name: 'Añadir cuenta No-Premium' }));
  await user.type(screen.getByLabelText('Nombre de usuario'), 'Fixture');
  await user.click(screen.getByRole('button', { name: 'Cancelar' }));
  expect(backend.invoke.mock.calls.some(([command]) => command === 'add_offline_account')).toBe(false);
  expect(screen.queryByRole('dialog')).toBeNull();
  await user.click(screen.getByRole('button', { name: 'Añadir cuenta No-Premium' }));
  await user.type(screen.getByLabelText('Nombre de usuario'), 'Fixture');
  backend.invoke.mockRejectedValueOnce('invalid_username');
  await user.click(screen.getByRole('button', { name: 'Confirmar' }));
  expect(await within(screen.getByRole('dialog')).findByRole('alert')).toBeTruthy();
});

test('Microsoft link copying and cancellation use the active backend operation', async () => {
  const user = userEvent.setup(); render(<Accounts onboarding />);
  const button = screen.getByRole('button', { name: 'Añadir cuenta Microsoft' });
  await waitFor(() => expect((button as HTMLButtonElement).disabled).toBe(false));
  await user.click(button);
  const dialog = await screen.findByRole('dialog', { name: 'Añadir cuenta Microsoft' });
  await waitFor(() => expect((within(dialog).getByRole('button', { name: 'Copiar enlace' }) as HTMLButtonElement).disabled).toBe(false));
  // While the browser step is pending, a spinner shares the line with the waiting sentence.
  const waiting = within(dialog).getByText(/Completa el inicio de sesión en tu navegador/);
  expect(waiting.classList.contains('login-wait')).toBe(true);
  expect(waiting.querySelector('svg.spin')).not.toBeNull();
  await user.click(within(dialog).getByRole('button', { name: 'Copiar enlace' }));
  expect(await screen.findByText('Enlace copiado.')).toBeTruthy();
  expect(await navigator.clipboard.readText()).toBe('https://login.microsoftonline.com/fixture');
  await user.click(within(dialog).getByRole('button', { name: 'Cancelar' }));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(backend.invoke).toHaveBeenCalledWith('cancel_microsoft_login', undefined);
});

test('browser failures retain the dialog and offer the copy fallback without another login', async () => {
  const original = backend.invoke.getMockImplementation()!;
  backend.invoke.mockImplementation(async (command: string) => {
    if (command === 'start_microsoft_login') {
      view = { ...view, revision: 2, login_pending: true };
      backend.listeners.forEach(callback => callback({ payload: view }));
      throw 'browser_unavailable';
    }
    return original(command);
  });
  const user = userEvent.setup(); render(<Accounts onboarding />);
  const button = screen.getByRole('button', { name: 'Añadir cuenta Microsoft' });
  await waitFor(() => expect((button as HTMLButtonElement).disabled).toBe(false));
  await user.click(button);
  const dialog = screen.getByRole('dialog');
  expect(await within(dialog).findByRole('alert')).toBeTruthy();
  expect((within(dialog).getByRole('button', { name: 'Copiar enlace' }) as HTMLButtonElement).disabled).toBe(false);
  await act(async () => {
    view = { ...view, revision: 3, login_pending: false, login_error: 'authentication_timeout' };
    backend.listeners.forEach(callback => callback({ payload: view }));
  });
  expect((within(dialog).getByRole('button', { name: 'Copiar enlace' }) as HTMLButtonElement).disabled).toBe(true);
  expect(backend.invoke.mock.calls.filter(([command]) => command === 'start_microsoft_login')).toHaveLength(1);
});

test('account management reuses offline creation and only removes after confirmation', async () => {
  view.accounts = [{ metadata: { account_id: 'fixture', kind: 'offline', username: 'Fixture', profile_id: 'fixture', oauth_client_id: null }, persisted: true, status: 'offline', error: null }];
  view.selected_account_id = 'fixture';
  const user = userEvent.setup(); render(<Accounts />);
  await screen.findByText('Fixture');
  expect(screen.getByText('Activa')).toBeTruthy();
  // The bust is drawn by the launcher from its own default skin: no online picture is requested for any row.
  expect(document.querySelector('.profile-bust')).toBeTruthy();
  expect(document.querySelector('img[src^="http"]')).toBeNull();
  await user.click(screen.getByRole('button', { name: 'Añadir', exact: true }));
  await user.click(screen.getByRole('menuitem', { name: 'Añadir cuenta No-Premium' }));
  expect(screen.getByRole('dialog', { name: 'Nueva cuenta No-Premium' })).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'Cancelar' }));
  await user.click(screen.getByRole('button', { name: 'Opciones de Fixture' }));
  await user.click(screen.getByRole('menuitem', { name: 'Quitar cuenta' }));
  expect(screen.getByRole('dialog', { name: 'Quitar cuenta' })).toBeTruthy();
  expect(backend.invoke.mock.calls.some(([command]) => command === 'remove_account')).toBe(false);
  await user.click(screen.getByRole('button', { name: 'Cancelar' }));
});

test('the Aspectos button sits beside the menu of Microsoft accounts only and edits the selected account', async () => {
  const row = (id: string, kind: 'microsoft' | 'offline', name: string) => ({ metadata: { account_id: id, kind, username: name, profile_id: id, oauth_client_id: null }, persisted: true, status: kind === 'offline' ? 'offline' as const : 'ready' as const, error: null });
  view.accounts = [row('micro', 'microsoft', 'Premium'), row('other', 'microsoft', 'Segunda'), row('off', 'offline', 'Local')];
  view.selected_account_id = 'micro';
  const open = vi.fn();
  const user = userEvent.setup(); render(<Accounts onEditSkins={open} />);
  await screen.findByText('Premium');
  expect(screen.queryByRole('button', { name: 'Aspectos de Local' })).toBeNull();
  expect(screen.getAllByRole('button', { name: /^Aspectos de/ })).toHaveLength(2);
  await user.click(screen.getByRole('button', { name: 'Aspectos de Premium' }));
  expect(open).toHaveBeenCalledTimes(1);
  expect(backend.invoke.mock.calls.some(([command]) => command === 'select_account')).toBe(false);
  // Another Microsoft account is selected first, so the editor never opens on the wrong profile.
  await user.click(screen.getByRole('button', { name: 'Aspectos de Segunda' }));
  await waitFor(() => expect(open).toHaveBeenCalledTimes(2));
  expect(backend.invoke).toHaveBeenCalledWith('select_account', { accountId: 'other' });
});

test('without an editor to open, no account row offers the Aspectos button', async () => {
  view.accounts = [{ metadata: { account_id: 'micro', kind: 'microsoft', username: 'Premium', profile_id: 'micro', oauth_client_id: null }, persisted: true, status: 'ready', error: null }];
  view.selected_account_id = 'micro';
  render(<Accounts />);
  await screen.findByText('Premium');
  expect(screen.queryByRole('button', { name: /^Aspectos/ })).toBeNull();
});

test('skin requests fall back locally and a different name can load again', () => {
  const { container, rerender } = render(<ProfileArtwork username="Fixture" />);
  const image = container.querySelector('img')!;
  expect(image.src).toContain('/2d/head/Fixture?cors=true');
  fireEvent.error(image);
  expect(image.src).toContain('default-skin.svg');
  rerender(<ProfileArtwork username="OtherFixture" />);
  expect(image.src).toContain('/2d/head/OtherFixture?cors=true');
  rerender(<ProfileArtwork username="a discord name" />);
  expect(image.src).toContain('default-skin.svg');
});
