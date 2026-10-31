import assert from 'node:assert/strict';
import test from 'node:test';
import { blocksPlay, channelLocked, primaryUpdateControl, updateNotice } from '../src/update-state.ts';

const describe = code => `Mensaje de ${code}.`;
const notice = value => updateNotice(value, describe);
const view = (overrides = {}) => ({
  currentVersion: '0.2.0-beta.3', channel: 'stable', phase: 'idle', pendingVersion: null, delivery: 'in_place',
  releaseUrl: null, progress: null, applyRequested: false, error: null, ...overrides,
});

test('only a known newer release blocks play', () => {
  assert.equal(blocksPlay(null), false);
  assert.equal(blocksPlay(view()), false);
  assert.equal(blocksPlay(view({ phase: 'failed', error: 'network_unavailable' })), false);
  assert.equal(blocksPlay(view({ pendingVersion: '0.2.0-beta.4' })), true);
});

test('with nothing pending the notices stay quiet except for an unavailable check', () => {
  assert.equal(notice(null), null);
  assert.equal(notice(view()), null);
  assert.equal(notice(view({ phase: 'checking' })), null);
  const failed = notice(view({ phase: 'failed', error: 'network_unavailable' }));
  assert.equal(failed.tone, 'status');
  assert.match(failed.text, /seguir jugando/);
  assert.equal(failed.action, 'retry');
});

test('a ready update asks for confirmation, then waits for idle and can be cancelled', () => {
  const ready = notice(view({ phase: 'ready', pendingVersion: '0.2.0-beta.4' }));
  assert.equal(ready.tone, 'alert');
  assert.match(ready.text, /0\.2\.0-beta\.4/);
  assert.equal(ready.action, 'apply');
  const waiting = notice(view({ phase: 'ready', pendingVersion: '0.2.0-beta.4', applyRequested: true }));
  assert.match(waiting.text, /terminen tus partidas/);
  assert.equal(waiting.action, 'cancel');
  assert.equal(notice(view({ phase: 'applying', pendingVersion: '0.2.0-beta.4' })).action, null);
});

test('download progress, manual packages and failures of a required update are explained', () => {
  const downloading = notice(view({ phase: 'downloading', pendingVersion: '0.2.0-beta.4', progress: 40 }));
  assert.match(downloading.text, /40 %/);
  assert.equal(notice(view({ phase: 'downloading', pendingVersion: '0.2.0-beta.4' })).text.includes('%'), false);
  const manual = notice(view({ phase: 'manual', pendingVersion: '0.2.0-beta.4', delivery: 'manual' }));
  assert.equal(manual.action, 'release_page');
  assert.match(manual.text, /no puede actualizarse sola/);
  const failed = notice(view({ phase: 'failed', pendingVersion: '0.2.0-beta.4', error: 'launcher_update_failed' }));
  assert.equal(failed.tone, 'alert');
  assert.match(failed.text, /Mensaje de launcher_update_failed./);
  assert.match(failed.text, /obligatorio actualizar/);
  assert.equal(failed.action, 'retry');
  const stale = notice(view({ phase: 'idle', pendingVersion: '0.2.0-beta.4' }));
  assert.equal(stale.action, 'retry');
});

test('the play replacement offers an action only when one can be taken', () => {
  assert.equal(primaryUpdateControl(null), null);
  assert.equal(primaryUpdateControl(view()), null);
  const pending = { pendingVersion: '0.2.0-beta.4' };
  assert.deepEqual(primaryUpdateControl(view({ ...pending, phase: 'ready' })), { label: 'Actualizar Panita Client', action: 'apply' });
  assert.equal(primaryUpdateControl(view({ ...pending, phase: 'ready', applyRequested: true })).action, null);
  assert.equal(primaryUpdateControl(view({ ...pending, phase: 'downloading' })).action, null);
  assert.equal(primaryUpdateControl(view({ ...pending, phase: 'applying' })).action, null);
  assert.equal(primaryUpdateControl(view({ ...pending, phase: 'manual' })).action, 'release_page');
  assert.equal(primaryUpdateControl(view({ ...pending, phase: 'failed' })).action, 'retry');
  assert.equal(primaryUpdateControl(view({ ...pending, phase: 'idle' })).action, 'retry');
});

test('the channel cannot change while an update is downloading or applying', () => {
  assert.equal(channelLocked(null), true);
  assert.equal(channelLocked(view({ phase: 'downloading' })), true);
  assert.equal(channelLocked(view({ phase: 'applying' })), true);
  assert.equal(channelLocked(view({ phase: 'ready' })), false);
  assert.equal(channelLocked(view()), false);
});
