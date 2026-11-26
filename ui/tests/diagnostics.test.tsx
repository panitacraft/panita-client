import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, test, vi } from 'vitest';
import { DiagnosticsPanel } from '../src/diagnostics';
const backend = vi.hoisted(() => ({ load: vi.fn(), clear: vi.fn(), open: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true }));
vi.mock('../src/diagnostic-api', () => ({ loadDiagnostics: backend.load, clearOldLogs: backend.clear, openLogsFolder: backend.open }));
afterEach(cleanup);
test('diagnostics explain storage categories and filter problems without dropping the last successful reading', async () => {
  backend.load.mockResolvedValueOnce({ capacity: 256, records: [
    { event: 'application_started', error: null, workflow: null, phase: null, processSessionId: 'synthetic', operationId: null, at: 1761930000000 },
    { event: 'storage_failure', error: 'storage_read_only', workflow: null, phase: null, processSessionId: 'synthetic', operationId: 'operation', at: 0 },
  ] });
  render(<DiagnosticsPanel />);
  await screen.findByText(/La unidad está en modo de solo lectura/);
  expect(screen.getByText('Se inició Panita Client.')).toBeTruthy();
  fireEvent.click(screen.getByLabelText('Solo problemas'));
  expect(screen.queryByText('Se inició Panita Client.')).toBeNull();
  expect(screen.getByText(/La unidad está en modo de solo lectura/)).toBeTruthy();
  backend.load.mockRejectedValueOnce(new Error('untrusted native response'));
  fireEvent.click(screen.getByRole('button', { name: 'Actualizar registro' }));
  await screen.findByRole('alert');
  expect(screen.getByText(/La unidad está en modo de solo lectura/)).toBeTruthy();
  expect(screen.queryByText('untrusted native response')).toBeNull();
});

test('clearing previous logs asks first, then reports what was removed and what stayed', async () => {
  backend.load.mockResolvedValue({ capacity: 256, records: [] });
  render(<DiagnosticsPanel />);
  await screen.findByText('Todavía no hay eventos disponibles.');
  fireEvent.click(screen.getByRole('button', { name: 'Borrar registros anteriores' }));
  expect(backend.clear).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: 'Cancelar' }));
  expect(screen.queryByRole('group', { name: 'Confirmar borrado de registros anteriores' })).toBeNull();
  backend.clear.mockResolvedValueOnce({ removed: 3, failed: 1 });
  fireEvent.click(screen.getByRole('button', { name: 'Borrar registros anteriores' }));
  fireEvent.click(screen.getByRole('button', { name: 'Borrar' }));
  const alert = await screen.findByRole('alert');
  expect(alert.textContent).toBe('Se borraron 3 registros anteriores. No pudimos borrar 1 archivo; puede estar en uso.');
  expect(backend.clear).toHaveBeenCalledTimes(1);
  backend.clear.mockResolvedValueOnce({ removed: 0, failed: 0 });
  fireEvent.click(screen.getByRole('button', { name: 'Borrar registros anteriores' }));
  fireEvent.click(screen.getByRole('button', { name: 'Borrar' }));
  expect(await screen.findByText('No hay registros anteriores que borrar.')).toBeTruthy();
});
