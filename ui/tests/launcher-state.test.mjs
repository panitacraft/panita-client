import assert from 'node:assert/strict';
import test from 'node:test';
import { instanceStatus, visibleSelection, safeArtwork } from '../src/launcher-state.ts';

test('selection survives a reordered catalog and falls back only when the saved entry disappears', () => {
  assert.equal(visibleSelection([{ id: 'b' }, { id: 'a' }], 'a'), 'a');
  assert.equal(visibleSelection([{ id: 'b' }], 'a'), 'b');
  assert.equal(visibleSelection([], 'a'), null);
});

test('catalog artwork rejects active content, local paths and embedded credentials', () => {
  for (const url of ['javascript:alert(1)', 'data:image/svg+xml,bad', 'file:///accounts.json', 'http://example.com/image', 'https://user:password@example.com/image', 'invalid', '', null]) assert.equal(safeArtwork(url), null);
  assert.equal(safeArtwork('https://cdn.example.com/banner.png'), 'https://cdn.example.com/banner.png');
});

test('navigation reports live preparation, running and consent states ahead of cached installation state', () => {
  const instance = { localState: 'installed', availability: 'enabled' };
  assert.equal(instanceStatus(instance, { phase: 'warning' }), 'Confirmación pendiente');
  assert.equal(instanceStatus(instance, { phase: 'running' }), 'En ejecución');
  assert.equal(instanceStatus(instance, { phase: 'installing', operation: 'repair' }), 'Reparando');
  assert.equal(instanceStatus(instance, { phase: 'failed' }), 'Lista para jugar');
  assert.equal(instanceStatus({ ...instance, availability: 'disabled' }, { phase: 'running' }), 'Deshabilitada');
});
