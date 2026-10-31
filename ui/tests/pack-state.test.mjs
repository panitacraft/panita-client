import assert from 'node:assert/strict';
import test from 'node:test';
import {
  cacheRetainedReason, cacheSummary, clearSummary, conflictSummary, folderHint, formatBytes, manualHeading,
  importErrorText, packProgressFraction, packProgressText, reasonText, skippedSummary,
} from '../src/pack-state.ts';
import { acceptPlaySnapshot, installationRefreshKey, playActive } from '../src/play-state.ts';

const file = { path: 'mods/crazyphone.jar', name: 'CrazyPhone', fileName: 'crazyphone.jar', folder: 'mods', projectId: 1646289, fileId: 9089413, size: 2300000, reason: 'manual' };

test('sizes and progress are readable in Spanish and bounded', () => {
  assert.equal(formatBytes(0), '0 B');
  assert.equal(formatBytes(512), '512 B');
  assert.match(formatBytes(693629982), /^661,5 MB$/);
  assert.match(formatBytes(3 * 1024 ** 3), /^3 GB$/);
  assert.equal(formatBytes(-1), '0 B');
  assert.equal(formatBytes(Number.NaN), '0 B');
  assert.equal(packProgressFraction(null), 0);
  assert.equal(packProgressFraction({ filesDone: 0, filesTotal: 0, bytesDone: 5, bytesTotal: 0, reusedFiles: 0 }), 0);
  assert.equal(packProgressFraction({ filesDone: 1, filesTotal: 2, bytesDone: 50, bytesTotal: 100, reusedFiles: 0 }), 0.5);
  assert.equal(packProgressFraction({ filesDone: 1, filesTotal: 2, bytesDone: 500, bytesTotal: 100, reusedFiles: 0 }), 1);
  assert.match(packProgressText(null), /Preparando/);
  assert.match(packProgressText({ filesDone: 12, filesTotal: 81, bytesDone: 1048576, bytesTotal: 2097152, reusedFiles: 0 }), /12 de 81 archivos \(1 MB de 2 MB\)\./);
  assert.match(packProgressText({ filesDone: 81, filesTotal: 81, bytesDone: 2, bytesTotal: 2, reusedFiles: 3 }), /3 ya estaban guardados/);
});

test('the manual file list explains each file and where it goes', () => {
  assert.equal(manualHeading(1), 'Falta 1 archivo que debes descargar tú mismo');
  assert.equal(manualHeading(2), 'Faltan 2 archivos que debes descargar tú mismo');
  assert.match(folderHint(file), /CurseForge \(archivo 9089413\).*carpeta mods/);
  assert.match(reasonText('manual'), /no permite/);
  assert.match(reasonText('withdrawn'), /retiró/);
});

test('import errors say the file is not the expected version without leaking details', () => {
  assert.equal(importErrorText('artifact_mismatch'), 'El archivo no es la versión esperada.');
  assert.match(importErrorText('pack_conflict'), /No lo reemplazamos/);
  // Everything else falls through to the general Spanish table.
  assert.equal(importErrorText('operation_busy'), null);
  assert.equal(importErrorText({ message: 'raw' }), null);
});

test('kept configuration and conflicts are summarized with exact counts', () => {
  assert.match(skippedSummary(['options.txt']), /^Conservamos 1 archivo de configuración/);
  assert.match(skippedSummary(['a', 'b']), /^Conservamos 2 archivos de configuración/);
  assert.match(conflictSummary(['mods/a.jar']), /1 archivo tuyo.*No cambiamos nada/);
  assert.match(conflictSummary(['a', 'b']), /2 archivos tuyos/);
});

test('the cache summary and clearing report are honest about what was and was not removed', () => {
  assert.equal(cacheSummary(null), 'Calculando el tamaño…');
  assert.match(cacheSummary(0), /vacía/);
  assert.match(cacheSummary(2 * 1024 * 1024), /2 MB/);
  const report = { removedEntries: 4, removedBytes: 14, retainedEntries: 0, details: [] };
  assert.match(clearSummary(report), /Eliminamos 4 archivos \(14 B\)\..*instancias y mundos no cambian/);
  assert.match(clearSummary({ ...report, removedEntries: 1, retainedEntries: 2 }), /Eliminamos 1 archivo .*Conservamos 2 que no pudimos eliminar/);
  assert.match(cacheRetainedReason('inUse'), /usando/);
  assert.match(cacheRetainedReason('unsafePath'), /enlace/);
  assert.match(cacheRetainedReason('__proto__'), /No pudimos eliminarlo/);
});

test('a pack waiting for manual files is not an active operation and triggers a catalog refresh', () => {
  assert.equal(playActive('awaiting_files'), false);
  assert.equal(playActive('installing'), true);
  const entry = (phase, requestId) => ({ instanceId: 'a', requestId, phase, operation: 'install' });
  const snapshot = (instances, revision = 1) => ({ revision, processSessionId: 's', closeBlocked: false, gracefulStopSupported: false, instances });
  assert.equal(installationRefreshKey(snapshot([entry('installing', 'r1')])), '');
  assert.equal(installationRefreshKey(snapshot([entry('awaiting_files', 'r1')])), 'a:r1:awaiting_files');
  // A stale snapshot cannot hide the waiting state.
  const newer = snapshot([entry('awaiting_files', 'r1')], 5);
  assert.equal(acceptPlaySnapshot(newer, snapshot([entry('installing', 'r1')], 4)), newer);
});

test('update conflicts explain each reason and only offer to move files aside when that can work', async () => {
  const state = await import('../src/pack-state.ts');
  for (const reason of ['edited', 'occupied', 'blocked']) assert.ok(state.conflictReasonText(reason).length > 10);
  assert.equal(state.conflictReasonText('something else'), 'Este archivo estorba.');
  assert.equal(state.conflictsResolvable([]), false);
  assert.equal(state.conflictsResolvable([{ path: 'mods/a.jar', reason: 'edited' }, { path: 'mods/b.jar', reason: 'occupied' }]), true);
  assert.equal(state.conflictsResolvable([{ path: 'mods/a.jar', reason: 'edited' }, { path: 'mods/b.jar', reason: 'blocked' }]), false);
  assert.match(state.updateConflictHeading(1), /1 archivo tuyo/);
  assert.match(state.updateConflictHeading(3), /3 archivos tuyos/);
});

test('version titles name both versions and shorten raw identifiers', async () => {
  const state = await import('../src/pack-state.ts');
  assert.equal(state.versionTitle({ installed: '0.1', available: '0.2' }, 'a', 'b'), 'Actualizar Modpack (v0.1 → v0.2)');
  assert.equal(state.versionTitle(null, 'v1', 'v2'), 'Actualizar Modpack (v1 → v2)');
  assert.equal(
    state.versionTitle({ installed: null, available: null }, '04ac0877-11a5-4de2-9a41-a41dbb44f1ee', '9f3c1d2e-aaaa-bbbb-cccc-111122223333'),
    'Actualizar Modpack (04ac0877… → 9f3c1d2e…)',
  );
});

test('summaries of backed up and released files use the right number', async () => {
  const state = await import('../src/pack-state.ts');
  assert.match(state.backedUpSummary(['mods/a.jar']), /1 archivo tuyo/);
  assert.match(state.backedUpSummary(['a', 'b']), /2 archivos tuyos/);
  assert.match(state.releasedSummary(['a']), /1 archivo que/);
  assert.match(state.releasedSummary(['a', 'b']), /2 archivos que/);
});
