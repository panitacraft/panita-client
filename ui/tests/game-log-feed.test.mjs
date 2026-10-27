import assert from 'node:assert/strict';
import test from 'node:test';
import { setTimeout as delay } from 'node:timers/promises';
import { startGameLogFeed, applyLogNotice, mergeLogPage } from '../src/game-log-feed.ts';

const page = (offset, total, running = true) => ({ offset, nextOffset: Math.min(offset + 500, total), total, pageSize: 500, running, exitCode: running ? null : 1,
  lines: Array.from({ length: Math.min(500, total - offset) }, (_, index) => ({ sequence: offset + index, stream: 'stdout', text: String(offset + index) })) });
async function until(condition) {
  for (let i = 0; i < 100 && !condition(); i++) await delay(5);
  assert.ok(condition(), 'feed completed within test timeout');
}
test('live bursts consume every cursor in order and retain a bounded visible page', async () => {
  let total = 0, changed, stopped = false;
  const cursors = [], pages = [], errors = [];
  const stop = startGameLogFeed({ offset: 0, follow: true,
    subscribe: async callback => { changed = callback; return () => { stopped = true; }; },
    load: async cursor => { cursors.push(cursor); return page(cursor, total); },
    receive: value => pages.push(value), fail: error => errors.push(error),
  });
  await until(() => pages.length === 1);
  total = 1503;
  changed({ total, running: true, exitCode: null });
  changed({ total, running: false, exitCode: 1 });
  await until(() => pages.at(-1)?.nextOffset === total);
  assert.deepEqual(cursors, [0, 0, 500, 1000, 1500]);
  assert.equal(pages.at(-1).lines.length, 500);
  assert.deepEqual(pages.at(-1).lines.map(line => line.sequence), Array.from({ length: 500 }, (_, i) => i + 1003));
  assert.equal(pages.at(-1).running, false);
  assert.deepEqual(errors, []);
  stop(); assert.equal(stopped, true);
});
test('notifications racing the initial read trigger catch-up without skipping a page', async () => {
  let changed, resolveInitial;
  const cursors = [], pages = [];
  const stop = startGameLogFeed({ offset: 0, follow: true,
    subscribe: async callback => { changed = callback; return () => {}; },
    load: async cursor => { cursors.push(cursor); return cursors.length === 1 ? new Promise(resolve => { resolveInitial = resolve; }) : page(cursor, 800); },
    receive: value => pages.push(value), fail: error => { throw error; },
  });
  await until(() => resolveInitial);
  changed({ total: 800, running: true, exitCode: null });
  resolveInitial(page(0, 100));
  await until(() => pages.at(-1)?.nextOffset === 800);
  assert.deepEqual(cursors, [0, 100, 600]);
  stop();
});
test('paused browsing receives final status without reloading or jumping to the tail', async () => {
  let changed, calls = 0;
  const pages = [];
  const stop = startGameLogFeed({ offset: 0, follow: false,
    subscribe: async callback => { changed = callback; return () => {}; },
    load: async () => { calls++; return page(0, 1000); },
    receive: value => pages.push(value), fail: error => { throw error; },
  });
  await until(() => pages.length);
  changed({ total: 1200, running: false, exitCode: 1 });
  changed({ total: Number.NaN, running: true, exitCode: null });
  assert.equal(calls, 1); assert.equal(pages.at(-1).offset, 0);
  assert.equal(pages.at(-1).running, false); assert.equal(pages.at(-1).total, 1200);
  assert.equal(applyLogNotice(pages.at(-1), { total: 1000, running: true, exitCode: null }).running, false);
  stop();
});
test('closing a window unsubscribes late subscriptions', async () => {
  let finishSubscribe, stopped = false, received = false;
  const stop = startGameLogFeed({ offset: null, follow: true,
    subscribe: () => new Promise(resolve => { finishSubscribe = resolve; }),
    load: async () => page(0, 10), receive: () => { received = true; }, fail: error => { throw error; },
  });
  stop(); finishSubscribe(() => { stopped = true; });
  await until(() => stopped);
  assert.equal(received, false);
  assert.throws(() => mergeLogPage(page(0, 500), page(499, 1000)), /cursor/);
});
test('closing a window ignores an in-flight read and failures do not start a retry loop', async () => {
  let finishRead, received = false, unsubscribed = false;
  const stop = startGameLogFeed({ offset: 0, follow: true,
    subscribe: async () => () => { unsubscribed = true; },
    load: () => new Promise(resolve => { finishRead = resolve; }),
    receive: () => { received = true; }, fail: error => { throw error; },
  });
  await until(() => finishRead);
  stop(); finishRead(page(0, 10));
  await delay(10);
  assert.equal(received, false); assert.equal(unsubscribed, true);
  let failures = 0, reads = 0;
  const end = startGameLogFeed({ offset: null, follow: true,
    subscribe: async () => () => {},
    load: async () => { reads++; throw new Error('unavailable'); },
    receive: () => assert.fail('failed read'), fail: () => { failures++; },
  });
  await until(() => failures === 1);
  await delay(10); assert.equal(reads, 1);
  end();
});
