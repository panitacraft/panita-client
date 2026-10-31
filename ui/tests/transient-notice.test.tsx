import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, test, vi } from 'vitest';
import { TransientNotice, useTransientNotice } from '../src/components/transient-notice';

function Fixture() {
  const toast = useTransientNotice();
  return <><button onClick={() => toast.show('Código copiado.')}>Copiar</button><TransientNotice notice={toast.notice} /></>;
}
afterEach(() => { cleanup(); vi.useRealTimers(); });

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
