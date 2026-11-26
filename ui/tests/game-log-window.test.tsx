import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { GameLogWindow } from '../src/game-output';

const backend = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, invoke: backend.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
beforeEach(() => {
  backend.invoke.mockImplementation(async (command: string, args: { offset?: number | null }) => {
    if (command === 'find_game_log') return { found: 0, next: 1, total: 3000 };
    if (command !== 'game_log_snapshot') throw new Error('Unexpected synthetic command');
    const offset = args.offset ?? 2500;
    const next = Math.min(offset + 500, 3000);
    return { offset, nextOffset: next, total: 3000, pageSize: 500, running: true, exitCode: null,
      lines: Array.from({ length: next - offset }, (_, index) => ({ sequence: offset + index, stream: 'stdout', text: 'Message ' + (offset + index) })) };
  });
});
afterEach(cleanup);

test('scrolling reads overlapping history windows without page controls or unbounded DOM', async () => {
  const { container } = render(<GameLogWindow instanceId="fixture" requestId="run" />);
  await screen.findByText('Message 2500');
  expect(container.querySelectorAll('.log-row')).toHaveLength(500);
  expect(screen.queryByRole('navigation', { name: 'Páginas del registro de Minecraft' })).toBeNull();
  fireEvent.wheel(screen.getByLabelText('Entradas del registro de Minecraft'), { deltaY: -120 });
  await screen.findByText('Message 2250');
  expect(container.querySelectorAll('.log-row')).toHaveLength(500);
  expect(backend.invoke).toHaveBeenCalledWith('game_log_snapshot', expect.objectContaining({ offset: 2250 }));
});

test('history search can reach entries outside the current rendered window', async () => {
  const user = userEvent.setup(); render(<GameLogWindow instanceId="fixture" requestId="run" />);
  await screen.findByText('Message 2500');
  await user.type(screen.getByLabelText('Filtrar mensajes'), 'Message 0');
  await user.click(screen.getByRole('button', { name: 'Coincidencia siguiente' }));
  expect(await screen.findByText('Message 0')).toBeTruthy();
  expect(backend.invoke).toHaveBeenCalledWith('find_game_log', expect.objectContaining({ query: 'Message 0' }));
});

test('clear and restore only change the local view of retained history', async () => {
  const user = userEvent.setup(); render(<GameLogWindow instanceId="fixture" requestId="run" />);
  await screen.findByText('Message 2500');
  await user.click(screen.getByRole('button', { name: 'Limpiar vista' }));
  await waitFor(() => expect(screen.queryByText('Message 2500')).toBeNull());
  await user.click(screen.getByRole('button', { name: 'Restaurar historial' }));
  expect(await screen.findByText('Message 2500')).toBeTruthy();
  expect(backend.invoke.mock.calls.every(([command]) => command === 'game_log_snapshot')).toBe(true);
});

test('export and copy send the complete history, ignoring filters and a cleared view', async () => {
  const user = userEvent.setup();
  const sent: { command: string; text: string }[] = [];
  const base = backend.invoke.getMockImplementation()!;
  backend.invoke.mockImplementation(async (command: string, args: { offset?: number | null; text?: string }) => {
    if (command === 'export_game_log' || command === 'copy_game_log') { sent.push({ command, text: args.text! }); return true; }
    return base(command, args);
  });
  render(<GameLogWindow instanceId="fixture" requestId="run" />);
  await screen.findByText('Message 2500');
  await user.click(screen.getByRole('button', { name: 'Limpiar vista' }));
  await user.click(screen.getByRole('button', { name: 'Exportar' }));
  expect((await screen.findByText('Registro exportado.')).textContent).toBe('Registro exportado.');
  await user.click(screen.getByRole('button', { name: 'Copiar' }));
  expect(await screen.findByText('Registro copiado al portapapeles.')).toBeTruthy();
  expect(sent.map(item => item.command)).toEqual(['export_game_log', 'copy_game_log']);
  for (const { text } of sent) {
    const rows = text.trimEnd().split('\n');
    expect(rows).toHaveLength(3000);
    expect(rows[0]).toBe('Message 0');
    expect(rows[2999]).toBe('Message 2999');
  }
});

test('a failed export is reported without losing the log', async () => {
  const user = userEvent.setup();
  const base = backend.invoke.getMockImplementation()!;
  backend.invoke.mockImplementation(async (command: string, args: { offset?: number | null }) => {
    if (command === 'export_game_log') throw 'storage_unavailable';
    return base(command, args);
  });
  const { container } = render(<GameLogWindow instanceId="fixture" requestId="run" />);
  await screen.findByText('Message 2500');
  await user.click(screen.getByRole('button', { name: 'Exportar' }));
  await waitFor(() => expect(container.querySelector('.transient-notice.notice-error')).toBeTruthy());
  expect(screen.getByText('Message 2500')).toBeTruthy();
});
