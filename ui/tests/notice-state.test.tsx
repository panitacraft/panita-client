import { expect, test } from 'vitest';
import { completionDetails, completionNote, entryNotice } from '../src/notice-state';
import { conflictHeading, unknownVersionNames } from '../src/pack-state';
import type { PlayEntry } from '../src/play-state';

const entry = (patch: Record<string, unknown>) => ({ instanceId: 'a', requestId: 'r', phase: 'failed', operation: 'update', error: null, ...patch }) as unknown as PlayEntry;

test('a required update is a plain toast with nothing to open', () => {
  expect(entryNotice(entry({ operation: 'play', error: 'update_required' }))).toEqual({ message: 'Actualiza esta instancia antes de jugar.', details: null });
});

test('conflicts and missing manual files are toasts with a details list, and only while the operation is over', () => {
  const conflicts = [{ path: 'mods/a.jar', reason: 'occupied' }];
  expect(entryNotice(entry({ error: 'pack_conflict', packConflicts: conflicts }))).toEqual({ message: 'Hay 1 archivo tuyo que estorba a la actualización.', details: 'conflicts' });
  expect(entryNotice(entry({ operation: 'install', error: 'pack_conflict', packConflicts: [...conflicts, ...conflicts] }))?.message).toBe('Hay 2 archivos tuyos que estorban a la instalación.');
  const manual = entryNotice(entry({ error: 'manual_files_required', packAwaiting: [{ path: 'mods/m.jar' }] }));
  expect(manual).toEqual({ message: 'Falta 1 archivo que debes descargar tú mismo.', details: 'manual' });
  // Without a list there is nothing to open, so the general message keeps showing in the page.
  expect(entryNotice(entry({ error: 'pack_conflict', packConflicts: [] }))).toBeNull();
  expect(entryNotice(entry({ error: 'manual_files_required' }))).toBeNull();
  expect(entryNotice(entry({ operation: 'install', error: 'manual_files_required', packAwaiting: [{ path: 'x' }] }))).toBeNull();
  expect(entryNotice(entry({ phase: 'installing', error: 'pack_conflict', packConflicts: conflicts }))).toBeNull();
  expect(entryNotice(entry({ error: 'artifact_mismatch' }))).toBeNull();
});

test('a completed operation offers details only when it kept or left something behind', () => {
  const done = (patch: Record<string, unknown>) => entry({ phase: 'installed', ...patch });
  expect(completionDetails(done({}))).toBeNull();
  expect(completionDetails(done({ packSkipped: [], packReleased: null, packBackedUp: [] }))).toBeNull();
  // A failed operation is not a completion.
  expect(completionDetails(entry({ packSkipped: ['options.txt'] }))).toBeNull();
  const natives = { cleanedGenerations: 1, retainedGenerations: 0, untrackedGenerations: 0, details: [] };
  expect(completionDetails(done({ nativeCleanup: natives }))).toBeNull();
  const kept = completionDetails(done({ packSkipped: ['options.txt'], nativeCleanup: { ...natives, retainedGenerations: 1 } }));
  expect(kept?.skipped).toEqual(['options.txt']);
  expect(kept?.natives?.retainedGenerations).toBe(1);
  expect(completionNote(kept!)).toBe('Algunos de tus archivos se conservaron. Quedaron carpetas nativas antiguas sin eliminar.');
  expect(completionNote(completionDetails(done({ packBackedUp: ['mods/a.jar'] }))!)).toBe('Algunos de tus archivos se conservaron.');
});

test('the conflict heading names what the files are in the way of', () => {
  expect(conflictHeading(1, 'update')).toBe('Hay 1 archivo tuyo que estorba a la actualización');
  expect(conflictHeading(3, 'repair')).toBe('Hay 3 archivos tuyos que estorban a la instalación');
});

test('unknown version names carry no notes', () => {
  expect(unknownVersionNames).toEqual({ installed: null, available: null, changelog: null });
});
