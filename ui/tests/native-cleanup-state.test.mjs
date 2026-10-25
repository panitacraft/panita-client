import assert from 'node:assert/strict';
import test from 'node:test';
import { nativeRetentionReason } from '../src/native-cleanup-state.ts';

test('native cleanup distinguishes ownership, changed files and access without exposing raw errors', () => {
  assert.match(nativeRetentionReason('missingOwnership'), /Sin registro de propiedad/);
  assert.match(nativeRetentionReason('changedFiles'), /cambiaron/);
  assert.match(nativeRetentionReason('accessFailure'), /permisos/);
  assert.match(nativeRetentionReason('unverifiedLibraries'), /bibliotecas originales/);
  assert.match(nativeRetentionReason('unknownFiles'), /anteriores al seguimiento/);
  assert.doesNotMatch(nativeRetentionReason('secret raw error'), /secret/);
  assert.equal(typeof nativeRetentionReason('__proto__'), 'string');
});
