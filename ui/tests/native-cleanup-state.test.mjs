import assert from 'node:assert/strict';
import test from 'node:test';
import { nativeRetentionReason, nativeCleanupSummary } from '../src/native-cleanup-state.ts';
import { messageForError } from '../src/errors.ts';

test('native cleanup reports active, redirected and blocked folders without ownership gates', () => {
  assert.match(nativeRetentionReason('active'), /instalación actual/);
  assert.match(nativeRetentionReason('accessFailure'), /en uso/);
  assert.match(nativeRetentionReason('unsafePath'), /no sigue enlaces/);
  assert.doesNotMatch(nativeRetentionReason('secret raw error'), /secret/);
  assert.equal(typeof nativeRetentionReason('__proto__'), 'string');
});

test('cleanup completion and pending recovery are explicit', () => {
  const report = { cleanedGenerations: 2, retainedGenerations: 0, untrackedGenerations: 0, details: [] };
  assert.match(nativeCleanupSummary(report), /eliminadas: 2.*completada/);
  assert.match(nativeCleanupSummary({ ...report, retainedGenerations: 1 }), /pendiente/);
  assert.doesNotMatch(nativeCleanupSummary({ ...report, retainedGenerations: 1 }), /completada/);
  assert.match(messageForError('native_cleanup_incomplete'), /desinstalada.*faltan archivos nativos/);
});
