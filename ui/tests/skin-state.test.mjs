import assert from 'node:assert/strict';
import test from 'node:test';
import {
  MAX_LOOK_NAME, MAX_SKIN_FILE_BYTES, applyRequestOf, canSaveCurrent, isDirty, lookCape, lookNameProblem, noSelection, pickCape, pickLook, previewOf,
  skinFileProblem, suggestedLookName, syncedText, validSelection,
} from '../src/skin-state.ts';

const CAPE_A = 'cape-a';
const CAPE_B = 'cape-b';
const hash = letter => letter.repeat(64);
const look = (id, overrides = {}) => ({ id, name: `Look ${id}`, model: 'classic', texture: hash(id), cape: null, createdMs: 1, ...overrides });
function view(overrides = {}) {
  return {
    profileId: 'p1', profileName: 'PlayerOne', syncedMs: 1_000, incomplete: false, matchingLook: null,
    current: { custom: true, texture: hash('a'), model: 'classic', cape: CAPE_A },
    capes: [{ id: CAPE_A, alias: 'Una', cached: true, active: true }, { id: CAPE_B, alias: 'Dos', cached: true, active: false }],
    library: [look('b'), look('c', { model: 'slim', cape: CAPE_B }), look('d', { cape: 'foreign' }), look('e', { cape: null })],
    ...overrides,
  };
}

test('with nothing selected the preview is the current look and nothing is pending', () => {
  const v = view();
  assert.deepEqual(previewOf(v, noSelection), { texture: hash('a'), model: 'classic', cape: CAPE_A, custom: true });
  assert.equal(isDirty(v, noSelection), false);
  assert.equal(applyRequestOf(v, noSelection), null);
});

test('previewing a saved look shows its skin, model and its own cape', () => {
  const v = view();
  assert.deepEqual(previewOf(v, { lookId: 'c', cape: null }), { texture: hash('c'), model: 'slim', cape: CAPE_B, custom: true });
});

test("a look's cape follows what the account owns, as the backend decides it", () => {
  const v = view();
  const [, owned, foreign, bare] = v.library;
  assert.equal(lookCape(v, owned), CAPE_B);
  // A cape the account lacks cannot be worn: the cape already worn stays.
  assert.equal(lookCape(v, foreign), CAPE_A);
  // A look without a cape takes the cape off.
  assert.equal(lookCape(v, bare), null);
  assert.equal(previewOf(v, { lookId: 'e', cape: null }).cape, null);
});

test('applying a look sends the look and only the cape change that differs from the account', () => {
  const v = view();
  assert.deepEqual(applyRequestOf(v, { lookId: 'c', cape: null }), { lookId: 'c', capeChange: { id: CAPE_B } });
  assert.deepEqual(applyRequestOf(v, { lookId: 'b', cape: null }), { lookId: 'b', capeChange: { id: null } });
  // The cape the account wears already matches, so it is not sent again.
  assert.deepEqual(applyRequestOf(v, { lookId: 'd', cape: null }), { lookId: 'd' });
});

test('a cape choice alone changes only the cape, and the same choice as before is no change', () => {
  const v = view();
  assert.deepEqual(applyRequestOf(v, { lookId: null, cape: { id: CAPE_B } }), { lookId: null, capeChange: { id: CAPE_B } });
  assert.deepEqual(applyRequestOf(v, { lookId: null, cape: { id: null } }), { lookId: null, capeChange: { id: null } });
  assert.equal(applyRequestOf(v, { lookId: null, cape: { id: CAPE_A } }), null);
});

test('a look with the same skin and model as the account only needs its cape', () => {
  const v = view({ library: [look('a', { texture: hash('a'), cape: CAPE_B })] });
  assert.deepEqual(applyRequestOf(v, { lookId: 'a', cape: null }), { lookId: null, capeChange: { id: CAPE_B } });
  assert.equal(applyRequestOf(view({ library: [look('a', { cape: CAPE_A })] }), { lookId: 'a', cape: null }), null);
});

test('an explicit cape choice overrides the look and is dropped when it is what the look implies', () => {
  const v = view();
  assert.equal(previewOf(v, { lookId: 'c', cape: { id: null } }).cape, null);
  assert.deepEqual(pickCape(v, { lookId: 'c', cape: null }, CAPE_B), { lookId: 'c', cape: null });
  assert.deepEqual(pickCape(v, { lookId: 'c', cape: null }, null), { lookId: 'c', cape: { id: null } });
  assert.deepEqual(pickCape(v, noSelection, CAPE_A), noSelection);
  assert.deepEqual(pickCape(v, noSelection, CAPE_B), { lookId: null, cape: { id: CAPE_B } });
});

test('choosing a look again clears it, and choosing another starts from its own cape', () => {
  assert.deepEqual(pickLook({ lookId: 'b', cape: { id: CAPE_B } }, 'c'), { lookId: 'c', cape: null });
  assert.deepEqual(pickLook({ lookId: 'b', cape: null }, 'b'), noSelection);
});

test('nothing is pending before the profile was ever synchronised', () => {
  const v = view({ current: null });
  assert.equal(isDirty(v, { lookId: 'b', cape: null }), false);
  assert.equal(applyRequestOf(v, { lookId: 'b', cape: null }), null);
  assert.equal(canSaveCurrent(v), false);
});

test('the current look can be saved only when it is a real, cached skin that is not saved yet', () => {
  assert.equal(canSaveCurrent(view()), true);
  assert.equal(canSaveCurrent(view({ matchingLook: 'b' })), false);
  assert.equal(canSaveCurrent(view({ current: { custom: false, texture: null, model: 'classic', cape: null } })), false);
  assert.equal(canSaveCurrent(view({ current: { custom: true, texture: null, model: 'classic', cape: null } })), false);
});

test('a selection that names a vanished look or cape is dropped', () => {
  const v = view();
  assert.deepEqual(validSelection(v, { lookId: 'zzz', cape: null }), noSelection);
  assert.deepEqual(validSelection(v, { lookId: 'b', cape: { id: 'gone' } }), { lookId: 'b', cape: null });
  assert.deepEqual(validSelection(v, { lookId: 'b', cape: { id: null } }), { lookId: 'b', cape: { id: null } });
});

test('suggested names are unique and fit the limit', () => {
  assert.equal(suggestedLookName(view()), 'Skin de PlayerOne');
  const taken = view({ library: [look('b', { name: 'skin de playerone' }), look('c', { name: 'Skin de PlayerOne 2' })] });
  assert.equal(suggestedLookName(taken), 'Skin de PlayerOne 3');
  const long = suggestedLookName(view({ profileName: 'X'.repeat(40), library: [] }));
  assert.equal(long.length, MAX_LOOK_NAME);
});

test('names and files are checked before anything is sent', () => {
  assert.equal(lookNameProblem('Mi skin'), null);
  assert.ok(lookNameProblem('   '));
  assert.ok(lookNameProblem('x'.repeat(MAX_LOOK_NAME + 1)));
  assert.equal(lookNameProblem('x'.repeat(MAX_LOOK_NAME)), null);
  assert.equal(skinFileProblem({ name: 'a.png', type: 'image/png', size: 4000 }), null);
  assert.equal(skinFileProblem({ name: 'A.PNG', type: '', size: 4000 }), null);
  assert.ok(skinFileProblem({ name: 'a.jpg', type: 'image/jpeg', size: 4000 }));
  assert.ok(skinFileProblem({ name: 'a.png', type: 'image/png', size: 0 }));
  assert.ok(skinFileProblem({ name: 'a.png', type: 'image/png', size: MAX_SKIN_FILE_BYTES + 1 }));
});

test('the synchronisation age reads naturally', () => {
  const now = 10_000_000_000;
  assert.equal(syncedText(null, now), 'Aún no sincronizado');
  assert.equal(syncedText(now - 5_000, now), 'Sincronizado hace un momento');
  assert.equal(syncedText(now - 5 * 60_000, now), 'Sincronizado hace 5 min');
  assert.equal(syncedText(now - 3 * 3_600_000, now), 'Sincronizado hace 3 h');
  assert.match(syncedText(now - 3 * 86_400_000, now), /\d/);
});
