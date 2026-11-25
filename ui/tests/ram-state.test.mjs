import assert from 'node:assert/strict';
import test from 'node:test';
import { autoRamText, exceedsMachine, ramProfileOf, sliderMaxMib, sliderZones } from '../src/ram-state.ts';

const custom = { type: 'custom', current_version_id: 'v1' };
const pack = (extra = {}) => ({ localState: 'installed', source: custom, installedSource: custom, installedVariant: 'deluxe', ...extra });

test('the profile follows what is installed: vanilla, Modrinth as Standard, custom packs by variant', () => {
  assert.equal(ramProfileOf(null), null);
  assert.equal(ramProfileOf({ source: undefined }), 'vanilla');
  assert.equal(ramProfileOf({ source: { type: 'vanilla' } }), 'vanilla');
  assert.equal(ramProfileOf({ source: { type: 'modrinth', pack_version_id: 'x' } }), 'standard');
  assert.equal(ramProfileOf(pack()), 'deluxe');
  assert.equal(ramProfileOf(pack({ installedVariant: 'lite' })), 'lite');
  // Before installing, or after uninstalling, a custom pack is sized as Standard.
  assert.equal(ramProfileOf(pack({ localState: 'not_installed' })), 'standard');
  assert.equal(ramProfileOf(pack({ installedSource: null })), 'standard');
});

test('the slider ends at the machine RAM in whole gigabytes and keeps its old range when it is unknown', () => {
  assert.equal(sliderMaxMib(16318, 4096), 16384);
  assert.equal(sliderMaxMib(8000, 4096), 8192);
  assert.equal(sliderMaxMib(32000, 99999), 31744, "a known machine ignores the saved value");
  // A tiny or odd reading never collapses the slider.
  assert.equal(sliderMaxMib(300, 4096), 1024);
  for (const unreadable of [null, undefined, 0, -5, NaN]) {
    assert.equal(sliderMaxMib(unreadable, 4096), 32768);
    assert.equal(sliderMaxMib(unreadable, 65536), 65536);
  }
});

test('the color zones leave the system its room and never invert', () => {
  assert.deepEqual(sliderZones(null, 32768), { green: 0.6, yellow: 0.85 });
  for (const machine of [2048, 4096, 8192, 16384, 65536]) {
    const max = Math.round(machine / 1024) * 1024;
    const { green, yellow } = sliderZones(machine, max);
    assert.ok(green > 0 && green < yellow && yellow <= 0.85, String(machine));
  }
});

test('only a value above the machine RAM is flagged', () => {
  assert.equal(exceedsMachine(16384, 16318), false);
  assert.equal(exceedsMachine(17408, 16318), true);
  assert.equal(exceedsMachine(65536, null), false);
  assert.equal(exceedsMachine(65536, 0), false);
});

test('the automatic line names the value for an instance and every kind for the global settings', () => {
  const preview = { totalMib: 8192,
    vanilla: { mib: 4096, basis: 'target' }, lite: { mib: 4096, basis: 'target' },
    standard: { mib: 3840, basis: 'limited' }, deluxe: { mib: 3072, basis: 'floor' } };
  assert.equal(autoRamText(preview, 'vanilla'), 'Se calculará al jugar según tu equipo. Con la RAM de tu equipo serían 4 GiB.');
  assert.match(autoRamText(preview, 'standard'), /serían 3\.75 GiB\. Es menos de lo ideal porque tu equipo tiene poca RAM\.$/);
  assert.match(autoRamText(preview, 'deluxe'), /serían 3 GiB\. Tu equipo tiene muy poca RAM;/);
  const global = autoRamText(preview, null);
  assert.match(global, /Vanilla 4 GiB, Ligero 4 GiB, Estándar 3\.75 GiB, Mejorado 3 GiB\./);
  // The reading failed: nothing about the machine is claimed.
  assert.match(autoRamText(null, 'vanilla'), /No pudimos leer tu RAM ahora/);
  assert.match(autoRamText({ ...preview, vanilla: { mib: 4096, basis: 'unknown' } }, 'vanilla'), /valor prudente\.$/);
});
