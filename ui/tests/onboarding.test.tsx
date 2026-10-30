import startupHtml from '../index.html?raw';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
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
  expect(await screen.findByRole('heading', { name: 'Activa Panita Client' })).toBeTruthy();
  expect(screen.getByLabelText('Tu nombre de Minecraft o de Discord')).toBeTruthy();
  expect(screen.queryByRole('heading', { name: 'Elige cómo jugar' })).toBeNull();
});

test('an activated member without Minecraft accounts can complete offline onboarding', async () => {
  activate(); const user = userEvent.setup(); mount();
  expect(await screen.findByRole('heading', { name: 'Elige cómo jugar' })).toBeTruthy();
  expect(await screen.findByRole('button', { name: 'Iniciar sesión con Microsoft', exact: true })).toBeTruthy();
  expect((await screen.findByLabelText('Nombre para jugar sin conexión') as HTMLInputElement).value).toBe('Fixture_Player');
  await user.click(screen.getByRole('button', { name: 'Añadir cuenta sin conexión' }));
  expect(await screen.findByRole('heading', { name: 'Biblioteca de prueba' })).toBeTruthy();
});

test('initialization failure offers a visible retry instead of an empty window', async () => {
  native.invoke.mockRejectedValueOnce('storage_unavailable');
  mount();
  expect(await screen.findByRole('button', { name: 'Reintentar', exact: true })).toBeTruthy();
  const user = userEvent.setup(); await user.click(screen.getByRole('button', { name: 'Reintentar', exact: true }));
  expect(await screen.findByLabelText('Tu nombre de Minecraft o de Discord')).toBeTruthy();
});
