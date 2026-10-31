import startupHtml from '../index.html?raw';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { ActivationGate, ActivationProvider } from '../src/activation';
import type { ActivationView } from '../src/activation-state';

const native = vi.hoisted(() => ({ invoke: vi.fn(), listeners: new Map<string, Set<(event: { payload: unknown }) => void>>() }));
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, invoke: native.invoke }));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ onResized: async () => () => {} }) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: async (name: string, callback: (event: { payload: unknown }) => void) => {
  const callbacks = native.listeners.get(name) ?? new Set(); callbacks.add(callback); native.listeners.set(name, callbacks);
  return () => callbacks.delete(callback);
} }));
let activation: ActivationView;
let accounts: { revision: number; selected_account_id: string | null; login_pending: boolean; login_error: null; accounts: unknown[] };
beforeEach(() => {
  native.listeners.clear();
  activation = { revision: 1, state: 'no_session', user: null, persisted: false, error: null, serial: null, expires_at: null, verified_at: null, retry_after: null };
  accounts = { revision: 1, selected_account_id: null, login_pending: false, login_error: null, accounts: [] };
  native.invoke.mockImplementation(async (command: string) => {
    if (command === 'initialize') return;
    if (command === 'main_window_control') return false;
    if (command === 'activation_snapshot') return activation;
    if (command === 'account_snapshot') return accounts;
    if (command === 'add_offline_account') {
      accounts = { ...accounts, revision: 2, selected_account_id: 'synthetic', accounts: [{ metadata: { account_id: 'synthetic', kind: 'offline', username: 'Fixture_Player', profile_id: 'profile', oauth_client_id: null }, status: 'offline', error: null, persisted: true }] };
      native.listeners.get('accounts-changed')?.forEach(callback => callback({ payload: accounts }));
      return;
    }
    throw new Error('Unexpected synthetic command: ' + command);
  });
});
afterEach(cleanup);
function mount() { render(<ActivationProvider><ActivationGate><h1>Biblioteca de prueba</h1></ActivationGate></ActivationProvider>); }
function activate() {
  activation = { ...activation, state: 'active', persisted: true, user: { id: 1, ign: 'Fixture_Player', discord_name: 'Fixture', avatar_url: null, minecraft_status: null, capabilities: {} } };
}

test('startup module failures retain a readable retry screen before React loads', async () => {
  const html = startupHtml;
  const parsed = new DOMParser().parseFromString(html.replace(/<script\b[^>]*>[\s\S]*?<\/script>/gi, '').replace(/<link\b[^>]*>/gi, ''), 'text/html');
  document.body.innerHTML = parsed.getElementById('root')!.outerHTML;
  const entry = document.createElement('script'); entry.id = 'application-entry'; document.body.append(entry);
  await import('../public/startup.js');
  fireEvent.error(document.getElementById('application-entry')!);
  expect(screen.getByRole('alert').textContent).toContain('No pudimos cargar la aplicación');
  expect(screen.getByRole('link', { name: 'Volver a cargar' })).toBeTruthy();
  document.body.innerHTML = '';
});

test('activation loads before Minecraft account onboarding', async () => {
  mount();
  expect(await screen.findByRole('heading', { name: 'Acceder a Panita Client' })).toBeTruthy();
  expect(screen.getByLabelText('IGN o usuario de Discord')).toBeTruthy();
  expect(screen.queryByRole('heading', { name: 'Acceder a Minecraft' })).toBeNull();
});

test('an activated member without Minecraft accounts can complete offline onboarding', async () => {
  activate(); const user = userEvent.setup(); mount();
  expect(await screen.findByRole('heading', { name: 'Acceder a Minecraft' })).toBeTruthy();
  expect(await screen.findByRole('button', { name: 'Añadir cuenta Microsoft', exact: true })).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'Añadir cuenta No-Premium' }));
  expect((await screen.findByLabelText('Nombre de usuario') as HTMLInputElement).value).toBe('Fixture_Player');
  await user.click(screen.getByRole('button', { name: 'Confirmar' }));
  expect(await screen.findByRole('heading', { name: 'Biblioteca de prueba' })).toBeTruthy();
});

test('a code screen restored without a typed name does not greet an empty name', async () => {
  activation = { ...activation, state: 'pending', serial: 'ABCD-EFGH-IJKL', expires_at: Math.floor(Date.now() / 1000) + 120 };
  mount();
  expect(await screen.findByLabelText('Código ABCD-EFGH-IJKL')).toBeTruthy();
  expect(screen.queryByText(/Hola,/)).toBeNull();
  expect(screen.getByText('Confirma este código en Discord')).toBeTruthy();
});

test('the Minecraft onboarding card uses the same card as the activation screen', async () => {
  activate(); mount();
  const heading = await screen.findByRole('heading', { name: 'Acceder a Minecraft' });
  expect(heading.closest('section')?.classList.contains('login-card')).toBe(true);
});

test('initialization failure offers a visible retry instead of an empty window', async () => {
  const implementation = native.invoke.getMockImplementation()!;
  let failed = false;
  native.invoke.mockImplementation((command: string, ...args: unknown[]) => {
    if (command === 'initialize' && !failed) { failed = true; return Promise.reject('storage_unavailable'); }
    return implementation(command, ...args);
  });
  mount();
  expect(await screen.findByRole('button', { name: 'Reintentar', exact: true })).toBeTruthy();
  const user = userEvent.setup(); await user.click(screen.getByRole('button', { name: 'Reintentar', exact: true }));
  expect(await screen.findByLabelText('IGN o usuario de Discord')).toBeTruthy();
});

test('activation preserves the full serial, supports copy, and regenerates expired codes', async () => {
  const original = native.invoke.getMockImplementation()!;
  native.invoke.mockImplementation(async (command: string, ...args: unknown[]) => {
    if (command === 'start_activation') {
      activation = { ...activation, revision: activation.revision + 1, state: 'pending', serial: 'ABCD-EFGH-IJKL', expires_at: Math.floor(Date.now() / 1000) + 180, error: null };
      return;
    }
    return original(command, ...args);
  });
  const user = userEvent.setup(); mount();
  await user.type(await screen.findByLabelText('IGN o usuario de Discord'), 'Fixture_Player');
  await user.click(screen.getByRole('button', { name: 'Continuar' }));
  expect(await screen.findByLabelText('Código ABCD-EFGH-IJKL')).toBeTruthy();
  expect(screen.getByRole('heading', { name: 'Acceder a Panita Client' })).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'Copiar código' }));
  expect(await navigator.clipboard.readText()).toBe('ABCD-EFGH-IJKL');
  await act(async () => {
    activation = { ...activation, revision: activation.revision + 1, state: 'no_session', serial: null, expires_at: null, error: 'activation_expired' };
    native.listeners.get('activation-changed')?.forEach(callback => callback({ payload: activation }));
  });
  expect((screen.getByRole('button', { name: 'Copiar código' }) as HTMLButtonElement).disabled).toBe(true);
  await user.click(screen.getByRole('button', { name: 'Generar otro código' }));
  await waitFor(() => expect((screen.getByRole('button', { name: 'Copiar código' }) as HTMLButtonElement).disabled).toBe(false));
  expect(native.invoke).toHaveBeenLastCalledWith('activation_snapshot', undefined);
});
