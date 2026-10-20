import assert from 'node:assert/strict';
import test from 'node:test';
import { canInstall } from '../src/library-state.ts';

const instance = { availability: 'enabled', compatible: true, supported: true, localState: 'not_installed', installedVersion: null, minecraftVersion: '1.21.1' };

test('fresh and interrupted installations can install or resume the same recorded pin', () => {
  for (const localState of ['not_installed', 'installing', 'failed', 'cancelled', 'needs_repair']) {
    for (const installedVersion of [null, '1.21.1']) {
      assert.equal(canInstall({ ...instance, localState, installedVersion }, false), true);
    }
  }
});

test('installation cannot bypass catalog restrictions, offline mode or explicit updates', () => {
  assert.equal(canInstall(instance, true), false);
  for (const availability of ['disabled', 'missing', 'unverified']) {
    assert.equal(canInstall({ ...instance, availability }, false), false);
  }
  for (const localState of ['installed', 'update_available', 'running']) {
    assert.equal(canInstall({ ...instance, localState }, false), false);
  }
  assert.equal(canInstall({ ...instance, compatible: false }, false), false);
  assert.equal(canInstall({ ...instance, supported: false }, false), false);
  assert.equal(canInstall({ ...instance, installedVersion: '1.20.1' }, false), false);
});
