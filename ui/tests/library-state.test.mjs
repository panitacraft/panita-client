import assert from 'node:assert/strict';
import test from 'node:test';
import { canInstall, updateTarget } from '../src/library-state.ts';

const instance = { availability: 'enabled', compatible: true, supported: true, localState: 'not_installed', installedVersion: null, minecraftVersion: '1.21.1' };

test('fresh and interrupted installations can install or resume the same recorded pin', () => {
  for (const localState of ['not_installed', 'installing', 'failed', 'cancelled', 'needs_repair']) {
    for (const installedVersion of [null, '1.21.1']) {
      assert.equal(canInstall({ ...instance, localState, installedVersion }, false), true);
    }
  }
});

test('update offers a specific installed-to-catalog transition without replacing Play or permitting implicit installation', () => {
  const updating = { ...instance, localState: 'update_available', installedVersion: '1.20.1', minecraftVersion: '1.21.1' };
  assert.deepEqual(updateTarget(updating, false), { fromVersion: '1.20.1', toVersion: '1.21.1' });
  assert.equal(canInstall(updating, false), false);
  assert.equal(updateTarget(updating, true), null);
  for (const availability of ['disabled', 'missing', 'unverified']) assert.equal(updateTarget({ ...updating, availability }, false), null);
  for (const patch of [{ compatible: false }, { supported: false }, { installedVersion: null }, { minecraftVersion: null }, { installedVersion: '1.21.1' }, { localState: 'installed' }, { localState: 'needs_repair' }]) {
    assert.equal(updateTarget({ ...updating, ...patch }, false), null);
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
