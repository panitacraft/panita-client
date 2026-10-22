import assert from 'node:assert/strict';
import test from 'node:test';
import { canInstall, canRepair, canUninstall, updateTarget } from '../src/library-state.ts';

const instance = { availability: 'enabled', compatible: true, supported: true, localState: 'not_installed', installedVersion: null, hasInstallationManifest: false, minecraftVersion: '1.21.1' };

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
  for (const patch of [{ compatible: false }, { supported: false }, { installedVersion: null }, { minecraftVersion: null }, { installedVersion: '1.21.1' }, { localState: 'installed' }]) {
    assert.equal(updateTarget({ ...updating, ...patch }, false), null);
  }
});

test('uninstalled and interrupted reinstalls require an explicit version transition when the catalog pin changed', () => {
  for (const localState of ['not_installed', 'failed', 'cancelled', 'needs_repair']) {
    const local = { ...instance, localState, installedVersion: '1.20.1' };
    assert.equal(canInstall(local, false), false);
    assert.deepEqual(updateTarget(local, false), { fromVersion: '1.20.1', toVersion: '1.21.1' });
  }
});

test('repair requires current catalog permission while preserving uninstall remains available offline or disabled', () => {
  const local = { ...instance, localState: 'installed', installedVersion: '1.20.1', hasInstallationManifest: true };
  assert.equal(canRepair(local, false), true);
  assert.equal(canRepair(local, true), false);
  for (const availability of ['disabled', 'missing', 'unverified']) {
    assert.equal(canRepair({ ...local, availability }, false), false);
    assert.equal(canUninstall({ ...local, availability }), true);
  }
  assert.equal(canUninstall({ ...local, localState: 'not_installed' }), false);
  assert.equal(canRepair({ ...local, localState: 'not_installed' }, false), false);
  assert.equal(canUninstall({ ...local, localState: 'running' }), false);
  assert.equal(canRepair({ ...local, installedVersion: null }, false), false);
});

test('an interrupted first installation offers retry but not maintenance requiring a published plan', () => {
  for (const localState of ['installing', 'failed', 'cancelled', 'needs_repair']) {
    const interrupted = { ...instance, localState, installedVersion: '1.21.1' };
    assert.equal(canInstall(interrupted, false), true);
    assert.equal(canRepair(interrupted, false), false);
    assert.equal(canUninstall(interrupted), false);
  }
  const damaged = { ...instance, localState: 'needs_repair', installedVersion: '1.21.1', hasInstallationManifest: true };
  assert.equal(canRepair(damaged, false), true);
  assert.equal(canUninstall(damaged), true);
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
