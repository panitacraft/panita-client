import assert from 'node:assert/strict';
import test from 'node:test';
import {
  automaticVariantSwitch, defaultVariant, hasVariantChoice, installedVariant, instanceVariants, isVariant, offeredVariants,
  optionSizeText, selectableOptions, variantChange, withdrawnVariant, withdrawnVariantText,
} from '../src/variant-state.ts';

const custom = { type: 'custom', current_version_id: 'v2' };
const instance = {
  availability: 'enabled', compatible: true, supported: true, localState: 'installed', removalPending: false,
  source: custom, installedSource: custom, variants: ['lite', 'standard', 'deluxe'], installedVariant: 'standard',
};

test('offered variants are ordered, known and always include Standard when empty', () => {
  assert.deepEqual(offeredVariants(['deluxe', 'standard', 'lite']), ['lite', 'standard', 'deluxe']);
  assert.deepEqual(offeredVariants(['standard', 'platinum']), ['standard']);
  for (const missing of [undefined, null, [], ['platinum']]) assert.deepEqual(offeredVariants(missing), ['standard']);
  assert.equal(isVariant('lite'), true);
  assert.equal(isVariant('gold'), false);
  // Only a custom instance offers a choice.
  assert.deepEqual(instanceVariants({ ...instance, source: { type: 'vanilla' } }), ['standard']);
  assert.deepEqual(instanceVariants({ ...instance, source: { type: 'modrinth', pack_version_id: 'x' } }), ['standard']);
  assert.equal(hasVariantChoice(instance), true);
  assert.equal(hasVariantChoice({ ...instance, variants: ['standard'] }), false);
  assert.equal(hasVariantChoice({ ...instance, variants: undefined }), false);
});

test('a variant that adds nothing is hidden unless it is the installed one, and Standard is the default', () => {
  const options = [
    { id: 'lite', fileCount: 12, size: 1, sameAsPrevious: false },
    { id: 'standard', fileCount: 28, size: 2, sameAsPrevious: false },
    { id: 'deluxe', fileCount: 28, size: 2, sameAsPrevious: true },
  ];
  assert.deepEqual(selectableOptions(options).map(option => option.id), ['lite', 'standard']);
  assert.deepEqual(selectableOptions(options, 'deluxe').map(option => option.id), ['lite', 'standard', 'deluxe']);
  assert.equal(defaultVariant(selectableOptions(options)), 'standard');
  assert.equal(defaultVariant(selectableOptions(options), 'lite'), 'lite');
  assert.equal(defaultVariant([{ id: 'lite', fileCount: 1, size: 1, sameAsPrevious: false }]), 'lite');
  assert.equal(defaultVariant([]), 'standard');
  assert.match(optionSizeText({ fileCount: 28, size: 128_307_508 }), /^122,4 MB · 28 archivos$/);
  assert.equal(optionSizeText({ fileCount: 1, size: 2 * 1024 * 1024 * 1024 }), '2 GB · 1 archivo');
});

test('an installation recorded before variants is Standard and only custom packs have one', () => {
  assert.equal(installedVariant({ ...instance, installedVariant: undefined }), 'standard');
  assert.equal(installedVariant({ ...instance, installedVariant: 'lite' }), 'lite');
  assert.equal(installedVariant({ ...instance, installedVariant: 'gold' }), 'standard');
  assert.equal(installedVariant({ ...instance, installedSource: null }), null);
  assert.equal(installedVariant({ ...instance, source: { type: 'vanilla' }, installedSource: { type: 'vanilla' } }), null);
});

test('changing the variant needs the installed pin to be the current one and a real choice', () => {
  assert.deepEqual(variantChange(instance, false), { pin: 'v2', installed: 'standard' });
  assert.equal(variantChange(instance, true), null);
  assert.equal(variantChange({ ...instance, variants: ['standard'] }, false), null);
  assert.equal(variantChange({ ...instance, installedSource: { type: 'custom', current_version_id: 'v1' } }, false), null);
  for (const patch of [{ availability: 'disabled' }, { compatible: false }, { supported: false }, { removalPending: true }, { localState: 'update_available' }, { localState: 'running' }, { localState: 'not_installed' }]) {
    assert.equal(variantChange({ ...instance, ...patch }, false), null, JSON.stringify(patch));
  }
});

test('a withdrawn variant moves to Standard on its own only when nothing else is pending', () => {
  const withdrawn = { ...instance, variants: ['standard', 'deluxe'], installedVariant: 'lite' };
  assert.equal(withdrawnVariant(withdrawn, false), 'lite');
  assert.equal(withdrawnVariant(instance, false), null);
  assert.equal(withdrawnVariant(withdrawn, true), null);
  assert.deepEqual(automaticVariantSwitch(withdrawn, false), { pin: 'v2', installed: 'lite', to: 'standard' });
  assert.equal(automaticVariantSwitch(instance, false), null);
  assert.equal(automaticVariantSwitch(withdrawn, true), null);
  // A version change goes through the confirmed update instead.
  const newer = { ...withdrawn, installedSource: { type: 'custom', current_version_id: 'v1' }, localState: 'update_available' };
  assert.equal(withdrawnVariant(newer, false), 'lite');
  assert.equal(automaticVariantSwitch(newer, false), null);
  for (const patch of [{ availability: 'disabled' }, { removalPending: true }, { localState: 'running' }]) {
    assert.equal(automaticVariantSwitch({ ...withdrawn, ...patch }, false), null, JSON.stringify(patch));
  }
  assert.match(withdrawnVariantText('lite'), /Lite ya no se ofrece.*Standard/);
});
