import assert from 'node:assert/strict';
import test from 'node:test';
import {
  automaticVariantSwitch, defaultVariant, hasVariantChoice, installedVariant, instanceVariants, isVariant, offeredVariants,
  optionSizeText, recommendVariant, resolveVariant, selectableOptions, variantChange, variantForMemory, withdrawnVariant, withdrawnVariantText,
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
  // After an uninstall the catalog still remembers the pin, but nothing is installed.
  assert.equal(installedVariant({ ...instance, localState: 'not_installed' }), null);
  assert.equal(installedVariant({ ...instance, installedVariant: null }), null);
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
  assert.match(withdrawnVariantText('lite'), /Ligero ya no se ofrece.*Estándar/);
});

test('cards exist before the sizes do, and a pick that is no longer shown falls back to the default', async () => {
  const { variantCards, resolveVariant, previewOptions } = await import('../src/variant-state.ts');
  const loading = variantCards(['lite', 'standard', 'deluxe'], null, null);
  assert.deepEqual(loading.map(card => [card.id, card.pending]), [['lite', true], ['standard', true], ['deluxe', true]]);
  assert.deepEqual(variantCards(['lite', 'standard'], null, 'network_unavailable'), []);
  const loaded = variantCards(['lite', 'standard', 'deluxe'], [
    { id: 'lite', fileCount: 1, size: 1, sameAsPrevious: false },
    { id: 'standard', fileCount: 2, size: 2, sameAsPrevious: false },
    { id: 'deluxe', fileCount: 2, size: 2, sameAsPrevious: true },
  ], null);
  assert.deepEqual(loaded.map(card => card.id), ['lite', 'standard']);
  assert.equal(resolveVariant(loaded, null), 'standard');
  assert.equal(resolveVariant(loaded, 'lite'), 'lite');
  assert.equal(resolveVariant(loaded, 'deluxe'), 'standard');
  assert.equal(resolveVariant(loaded, null, 'lite'), 'lite');
  assert.deepEqual(previewOptions(['lite', 'standard']).map(option => option.id), ['lite', 'standard']);
});

const GIB = 1024;
const cards = [
  { id: 'lite', fileCount: 12, size: 500 * 1024 * 1024, sameAsPrevious: false },
  { id: 'standard', fileCount: 28, size: 2 * GIB * 1024 * 1024, sameAsPrevious: false },
  { id: 'deluxe', fileCount: 35, size: 6 * GIB * 1024 * 1024, sameAsPrevious: false },
];

test('memory decides the variant: up to 4 GB Lite, over 4 and under 12 Standard, 12 or more Deluxe', () => {
  assert.equal(variantForMemory(2 * GIB), 'lite');
  assert.equal(variantForMemory(4 * GIB), 'lite');
  // The system reports a little under the nominal amount; whole gigabytes decide.
  assert.equal(variantForMemory(3.9 * GIB), 'lite');
  assert.equal(variantForMemory(5 * GIB), 'standard');
  assert.equal(variantForMemory(7.8 * GIB), 'standard');
  assert.equal(variantForMemory(11 * GIB), 'standard');
  assert.equal(variantForMemory(11.9 * GIB), 'deluxe');
  assert.equal(variantForMemory(12 * GIB), 'deluxe');
  assert.equal(variantForMemory(15.9 * GIB), 'deluxe');
  for (const unknown of [null, undefined, 0, -1, Number.NaN]) assert.equal(variantForMemory(unknown), null);
});

test('the recommendation follows the memory and says why', () => {
  const plenty = 500 * GIB;
  assert.equal(recommendVariant(cards, { memoryMib: 4 * GIB, freeDiskMib: plenty })?.variant, 'lite');
  assert.equal(recommendVariant(cards, { memoryMib: 8 * GIB, freeDiskMib: plenty })?.variant, 'standard');
  assert.equal(recommendVariant(cards, { memoryMib: 16 * GIB, freeDiskMib: plenty })?.variant, 'deluxe');
  assert.match(recommendVariant(cards, { memoryMib: 16 * GIB, freeDiskMib: plenty })?.reason ?? '', /^Te recomendamos el Perfil de Rendimiento Mejorado porque tu equipo tiene 16 GB de RAM.$/);
  // Without a reading, Standard is recommended and nothing is claimed about the machine.
  assert.deepEqual(recommendVariant(cards, null), { variant: 'standard', reason: null });
  assert.deepEqual(recommendVariant(cards, { memoryMib: null, freeDiskMib: null }), { variant: 'standard', reason: null });
  assert.equal(recommendVariant([], { memoryMib: 16 * GIB, freeDiskMib: plenty }), null);
});

test('a variant the instance does not show gives way to the closest one below, or the lowest', () => {
  const plenty = 500 * GIB;
  const noDeluxe = cards.slice(0, 2);
  assert.equal(recommendVariant(noDeluxe, { memoryMib: 16 * GIB, freeDiskMib: plenty })?.variant, 'standard');
  const onlyUpper = cards.slice(1);
  assert.equal(recommendVariant(onlyUpper, { memoryMib: 2 * GIB, freeDiskMib: plenty })?.variant, 'standard');
  assert.equal(recommendVariant(onlyUpper, { memoryMib: 16 * GIB, freeDiskMib: plenty })?.variant, 'deluxe');
});

test('a variant that would not fit on the disk gives way to a lighter one', () => {
  // Deluxe: twice 6 GB plus 2 GB for the game is 14 GB. Standard needs 6 GB; Lite a little over 3 GB.
  const machine = (freeGib) => ({ memoryMib: 16 * GIB, freeDiskMib: freeGib * GIB });
  assert.equal(recommendVariant(cards, machine(14))?.variant, 'deluxe');
  const standard = recommendVariant(cards, machine(13));
  assert.equal(standard?.variant, 'standard');
  assert.match(standard?.reason ?? '', /^Te recomendamos el Perfil de Rendimiento Estándar: .*16 GB de RAM.*13 GB/);
  assert.equal(recommendVariant(cards, machine(6))?.variant, 'standard');
  assert.equal(recommendVariant(cards, machine(5))?.variant, 'lite');
  // Even the lightest one is tight: it is still the one recommended, with a warning.
  const tight = recommendVariant(cards, machine(1));
  assert.equal(tight?.variant, 'lite');
  assert.match(tight?.reason ?? '', /espacio libre en disco/);
  // Sizes still being fetched, or a disk that could not be read, never block a recommendation.
  const pending = cards.map(card => ({ ...card, size: 0, pending: true }));
  assert.equal(recommendVariant(pending, machine(1))?.variant, 'deluxe');
  assert.equal(recommendVariant(cards, { memoryMib: 16 * GIB, freeDiskMib: null })?.variant, 'deluxe');
});

test('when changing, only what is added to the installed files needs room', () => {
  // Standard is installed. Deluxe adds 4 GB, which is 8 GB with the copy; the game is already there.
  const machine = (freeGib) => ({ memoryMib: 16 * GIB, freeDiskMib: freeGib * GIB });
  assert.equal(recommendVariant(cards, machine(8), 'standard')?.variant, 'deluxe');
  assert.equal(recommendVariant(cards, machine(7), 'standard')?.variant, 'standard');
});

test('the default is the installed variant, then the recommended one, then Standard', () => {
  assert.equal(defaultVariant(cards, null, 'deluxe'), 'deluxe');
  assert.equal(defaultVariant(cards, 'lite', 'deluxe'), 'lite');
  assert.equal(defaultVariant(cards, null, null), 'standard');
  assert.equal(defaultVariant(cards.slice(0, 2), null, 'deluxe'), 'standard');
  assert.equal(resolveVariant(cards, null, null, 'lite'), 'lite');
  assert.equal(resolveVariant(cards, 'deluxe', null, 'lite'), 'deluxe');
});
