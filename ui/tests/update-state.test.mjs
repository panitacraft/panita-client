import assert from 'node:assert/strict';
import test from 'node:test';
import { badgeModel, blocksPlay, channelLocked, previewUpdateView } from '../src/update-state.ts';

const describe = code => `Mensaje de ${code}.`;
const badge = value => badgeModel(value, describe);
const view = (overrides = {}) => ({
  currentVersion: '0.2.0-beta.3', channel: 'stable', phase: 'idle', pendingVersion: null, delivery: 'in_place',
  releaseUrl: null, progress: null, applyRequested: false, error: null, ...overrides,
});
const pending = { pendingVersion: '0.2.0-beta.4' };

test('only a known newer release blocks play', () => {
  assert.equal(blocksPlay(null), false);
  assert.equal(blocksPlay(view()), false);
  assert.equal(blocksPlay(view({ phase: 'failed', error: 'network_unavailable' })), false);
  assert.equal(blocksPlay(view(pending)), true);
});

test('with nothing pending the version stays plain, except after a failed check', () => {
  assert.deepEqual(badge(null), { kind: 'version' });
  assert.deepEqual(badge(view()), { kind: 'version' });
  assert.deepEqual(badge(view({ phase: 'checking' })), { kind: 'version' });
  const failed = badge(view({ phase: 'failed', error: 'network_unavailable' }));
  assert.equal(failed.kind, 'retry');
  assert.match(failed.tooltip, /No se pudieron buscar actualizaciones/);
});

test('a ready update is a button, then waits for idle and can be cancelled', () => {
  const ready = badge(view({ phase: 'ready', ...pending }));
  assert.equal(ready.kind, 'update');
  assert.equal(ready.action, 'apply');
  assert.equal(ready.label, 'Actualizar');
  assert.match(ready.tooltip, /0\.2\.0-beta\.4/);
  const waiting = badge(view({ phase: 'ready', ...pending, applyRequested: true }));
  assert.equal(waiting.action, 'cancel');
  assert.match(waiting.tooltip, /terminar tus partidas/);
});

test('working states have no action and fit a short label', () => {
  const downloading = badge(view({ phase: 'downloading', ...pending, progress: 42 }));
  assert.equal(downloading.action, null);
  assert.equal(downloading.label, '42 %');
  assert.equal(badge(view({ phase: 'downloading', ...pending })).label, 'Bajando');
  assert.equal(badge(view({ phase: 'applying', ...pending })).action, null);
  assert.equal(badge(view({ phase: 'checking', ...pending })).action, null);
  for (const phase of ['ready', 'downloading', 'checking', 'applying', 'manual', 'failed', 'idle']) {
    assert.ok(badge(view({ phase, ...pending })).label.length <= 10, phase);
  }
});

test('manual packages open the release page and failures of a required update are red', () => {
  const manual = badge(view({ phase: 'manual', ...pending, delivery: 'manual' }));
  assert.equal(manual.action, 'release_page');
  assert.match(manual.tooltip, /no puede actualizarse sola/);
  const failed = badge(view({ phase: 'failed', ...pending, error: 'launcher_update_failed' }));
  assert.equal(failed.error, true);
  assert.equal(failed.action, 'retry');
  assert.match(failed.tooltip, /Mensaje de launcher_update_failed\./);
  assert.match(failed.tooltip, /obligatoria/);
  const stale = badge(view({ phase: 'idle', ...pending }));
  assert.equal(stale.action, 'retry');
  assert.equal(stale.error, false);
});

test('the channel cannot change while an update is downloading or applying', () => {
  assert.equal(channelLocked(null), true);
  assert.equal(channelLocked(view({ phase: 'downloading' })), true);
  assert.equal(channelLocked(view({ phase: 'applying' })), true);
  assert.equal(channelLocked(view({ phase: 'ready' })), false);
  assert.equal(channelLocked(view()), false);
});

test('development previews map to states and unknown names to none', () => {
  assert.equal(previewUpdateView(null), null);
  assert.equal(previewUpdateView('unknown'), null);
  assert.equal(previewUpdateView('ready').phase, 'ready');
  assert.equal(previewUpdateView('required-failed').pendingVersion, '0.2.0-beta.4');
  assert.equal(previewUpdateView('failed').pendingVersion, null);
});
