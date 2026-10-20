import assert from 'node:assert/strict';
import test from 'node:test';
import { acceptPlaySnapshot, installationRefreshKey, playActive } from '../src/play-state.ts';

const snapshot = (revision, processSessionId = 'current') => ({ revision, processSessionId, closeBlocked: false, gracefulStopSupported: false, instances: [] });

test('a late initial snapshot cannot replace a newer process event', () => {
  const current = snapshot(8);
  assert.equal(acceptPlaySnapshot(current, snapshot(7)), current);
  const next = snapshot(9);
  assert.equal(acceptPlaySnapshot(current, next), next);
});

test('a restarted desktop process starts its own revision sequence', () => {
  const next = snapshot(0, 'new-process');
  assert.equal(acceptPlaySnapshot(snapshot(30), next), next);
});

test('only terminal states allow a new Play request', () => {
  for (const phase of ['preparing', 'installing', 'warning', 'starting', 'running', 'stopping', 'cancelling']) assert.equal(playActive(phase), true);
  for (const phase of ['installed', 'failed', 'exited', 'cancelled']) assert.equal(playActive(phase), false);
});

test('catalog reconciliation waits for terminal installation results and ignores download ticks', () => {
  const current = snapshot(1);
  current.instances = [{ instanceId: 'first', requestId: 'install-1', operation: 'install', phase: 'installing' }];
  assert.equal(installationRefreshKey(current), '');
  current.revision = 50;
  assert.equal(installationRefreshKey(current), '');
  current.instances[0].phase = 'installed';
  const key = installationRefreshKey(current);
  assert.notEqual(key, '');
  current.revision = 51;
  assert.equal(installationRefreshKey(current), key);
  current.instances.push({ instanceId: 'second', requestId: 'play-1', operation: 'play', phase: 'exited' });
  assert.equal(installationRefreshKey(current), key);
  current.instances[0].requestId = 'install-2';
  current.instances[0].phase = 'cancelled';
  assert.notEqual(installationRefreshKey(current), key);
});

test('failed installation refreshes local state and out-of-order progress cannot revive it', () => {
  const progress = { ...snapshot(3), instances: [{ instanceId: 'first', requestId: 'install-1', operation: 'install', phase: 'installing' }] };
  const failure = { ...snapshot(4), instances: [{ ...progress.instances[0], phase: 'failed' }] };
  assert.notEqual(installationRefreshKey(failure), installationRefreshKey(progress));
  assert.equal(acceptPlaySnapshot(failure, progress), failure);
});
