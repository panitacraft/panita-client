import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, test, vi } from 'vitest';
import { TransientNotice, useTransientNotice } from '../src/components/transient-notice';

function Fixture() {
  const toast = useTransientNotice();
  return <><button onClick={() => toast.show('Código copiado.')}>Copiar</button><TransientNotice notice={toast.notice} /></>;
}
afterEach(() => { cleanup(); vi.useRealTimers(); });

function ActionFixture({ onDetails }: { onDetails: () => void }) {
  const toast = useTransientNotice();
  return <><button onClick={() => toast.show('Hay archivos tuyos.', 'error', { label: 'Ver detalles', onClick: onDetails })}>Avisar</button><TransientNotice notice={toast.notice} /></>;
}

test('a toast with an action stays until dismissed and runs the action once per click', () => {
  vi.useFakeTimers();
  const onDetails = vi.fn();
  render(<ActionFixture onDetails={onDetails} />);
  fireEvent.click(screen.getByRole('button', { name: 'Avisar' }));
  act(() => { vi.advanceTimersByTime(60_000); });
  expect(screen.getByText('Hay archivos tuyos.')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'Ver detalles' }));
  expect(onDetails).toHaveBeenCalledTimes(1);
  // Opening the details does not dismiss it: the player can come back to it.
  expect(screen.getByText('Hay archivos tuyos.')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'Cerrar aviso' }));
  expect(screen.queryByText('Hay archivos tuyos.')).toBeNull();
});

test('copy notices expire, repeated copies restart the timer, and notices include an icon', () => {
  vi.useFakeTimers();
  render(<Fixture />);
  fireEvent.click(screen.getByRole('button', { name: 'Copiar' }));
  expect(screen.getByText('Código copiado.')).toBeTruthy();
  expect(screen.getByRole('status').querySelector('svg')).not.toBeNull();
  act(() => { vi.advanceTimersByTime(3000); });
  fireEvent.click(screen.getByRole('button', { name: 'Copiar' }));
  act(() => { vi.advanceTimersByTime(3000); });
  expect(screen.getByText('Código copiado.')).toBeTruthy();
  act(() => { vi.advanceTimersByTime(1000); });
  expect(screen.queryByText('Código copiado.')).toBeNull();
});
