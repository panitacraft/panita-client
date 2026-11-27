import assert from 'node:assert/strict';
import test from 'node:test';
import { FRONT_HEIGHT, FRONT_WIDTH, capeSource, frontParts, isLegacyLayout } from '../src/skin-render.ts';

const inside = part => part.dx >= 0 && part.dy >= 0 && part.dx + part.w <= FRONT_WIDTH && part.dy + part.h <= FRONT_HEIGHT;
const key = part => `${part.dx},${part.dy}`;

test('every part fits the frame and the skin, for every model and layout', () => {
  for (const model of ['classic', 'slim']) {
    for (const legacy of [false, true]) {
      const parts = frontParts(model, legacy);
      assert.ok(parts.every(inside), `${model} ${legacy}`);
      assert.ok(parts.every(part => part.sx >= 0 && part.sy >= 0 && part.sx + part.w <= 64 && part.sy + part.h <= (legacy ? 32 : 64)), `${model} ${legacy} source`);
    }
  }
});

test('slim arms are one pixel narrower and sit one pixel in from the edge', () => {
  const arms = model => frontParts(model, false).filter(part => part.dy === 8 && part.sy === 20 && part.w < 8 && part.dx !== 4);
  assert.deepEqual(arms('classic').map(part => [part.dx, part.w]), [[0, 4]]);
  assert.deepEqual(arms('slim').map(part => [part.dx, part.w]), [[1, 3]]);
  const left = model => frontParts(model, false).find(part => part.sx === 36 && part.sy === 52);
  assert.equal(left('classic').w, 4);
  assert.equal(left('slim').w, 3);
});

test('the outer layer is drawn after the base layer, and old skins have only a hat', () => {
  const modern = frontParts('classic', false);
  const firstOuter = modern.findIndex(part => part.sy === 36 || part.sx === 40);
  assert.ok(firstOuter > 0);
  assert.ok(modern.slice(0, firstOuter).every(part => part.sy !== 36 && part.sx !== 40));
  const old = frontParts('classic', true);
  assert.equal(old.filter(part => part.sx === 40 && part.sy === 8).length, 1);
  assert.equal(old.length, 7);
  // Without left limbs of their own, the right ones are mirrored into place.
  assert.deepEqual(old.filter(part => part.flip).map(key).sort(), ['12,8', '8,20']);
  assert.ok(frontParts('classic', false).every(part => !part.flip));
});

test('a skin that is twice as wide as it is tall has the old layout', () => {
  assert.equal(isLegacyLayout(64, 32), true);
  assert.equal(isLegacyLayout(128, 64), true);
  assert.equal(isLegacyLayout(64, 64), false);
});

test('the cape face scales with the image, including the old small layout', () => {
  assert.deepEqual(capeSource({ width: 64, height: 32 }), { x: 1, y: 1, w: 10, h: 16 });
  assert.deepEqual(capeSource({ width: 128, height: 64 }), { x: 2, y: 2, w: 20, h: 32 });
  assert.deepEqual(capeSource({ width: 22, height: 17 }), { x: 1, y: 1, w: 10, h: 16 });
  assert.deepEqual(capeSource({ width: 44, height: 34 }), { x: 2, y: 2, w: 20, h: 32 });
});
