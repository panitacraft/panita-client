import assert from 'node:assert/strict';
import test from 'node:test';
import { acceptPlaySnapshot, playActive } from '../src/play-state.ts';

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
  for (const phase of ['preparing', 'warning', 'starting', 'running', 'stopping', 'cancelling']) assert.equal(playActive(phase), true);
  for (const phase of ['failed', 'exited', 'cancelled']) assert.equal(playActive(phase), false);
});
