import assert from 'node:assert/strict';
import test from 'node:test';
import { createImageCache } from '../src/screenshot-images.ts';

const shot = (name, modifiedMs = 1) => ({ name, size: 1, modifiedMs });

test('a loaded image is reused and the least recently used one is dropped past the limit', async () => {
  const loads = [];
  const cache = createImageCache(async item => { loads.push(item.name); return `data:${item.name}`; }, 2);
  assert.equal(await cache.get(shot('a.png')), 'data:a.png');
  await cache.get(shot('b.png'));
  await cache.get(shot('a.png'));
  assert.deepEqual(loads, ['a.png', 'b.png']);
  await cache.get(shot('c.png'));
  await cache.get(shot('b.png'));
  assert.deepEqual(loads, ['a.png', 'b.png', 'c.png', 'b.png']);
});

test('a changed file is loaded again and a failure is not remembered', async () => {
  let attempts = 0;
  const cache = createImageCache(async item => { attempts += 1; if (attempts === 1) throw new Error('busy'); return `data:${item.modifiedMs}`; });
  await assert.rejects(cache.get(shot('a.png', 1)));
  assert.equal(await cache.get(shot('a.png', 1)), 'data:1');
  assert.equal(await cache.get(shot('a.png', 2)), 'data:2');
  assert.equal(attempts, 3);
});
