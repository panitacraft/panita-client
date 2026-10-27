import assert from 'node:assert/strict';
import test from 'node:test';
import { logWindowIdentity, previousLogPage, unacknowledgedCrashes } from '../src/game-output-state.ts';
import { readableGameLines } from '../src/game-log-format.ts';

const instanceId = '65ee13c6-bc34-4914-aa09-41205f3691a0';
const requestId = '7238fb86-89bf-44a1-bd6c-96362be534bb';

test('Minecraft console XML is readable plain text while stderr and ordinary output remain intact', () => {
  const lines = [
    { stream: 'stdout', text: '<log4j:Event logger="game" level="INFO" thread="Render thread">' },
    { stream: 'stdout', text: '<log4j:Message><![CDATA[Game ready <script>text only</script>]]></log4j:Message>' },
    { stream: 'stdout', text: '</log4j:Event>' },
    { stream: 'stderr', text: 'Error: example failure' },
    { stream: 'stdout', text: 'Ordinary console message' },
  ];
  assert.deepEqual(readableGameLines(lines), [
    { stream: 'stdout', text: '[INFO/Render thread] Game ready <script>text only</script>' },
    lines[3], lines[4],
  ]);
  const history = [lines[3], ...Array(10_000).fill(lines[4]), { stream: 'stderr', text: 'last' }];
  const readable = readableGameLines(history);
  assert.equal(readable.length, 10_002);
  assert.deepEqual(readable[0], lines[3]);
  assert.deepEqual(readable.at(-1), history.at(-1));
});

test('console history pages navigate to the beginning without imposing a retention cap', () => {
  assert.equal(previousLogPage({ offset: 1234, pageSize: 500 }), 734);
  assert.equal(previousLogPage({ offset: 234, pageSize: 500 }), 0);
  assert.equal(previousLogPage({ offset: 0, pageSize: 500 }), 0);
});

test('log windows bind their identity to one instance and execution', () => {
  assert.deepEqual(logWindowIdentity(`game-log-${instanceId}-${requestId}`), { instanceId, requestId });
  for (const label of ['main', `game-log-${instanceId}`, `game-log-${instanceId}-${requestId}/other`]) assert.equal(logWindowIdentity(label), null);
});

test('crash notifications exclude normal exits and requested stops and dismiss only the current execution', () => {
  const entry = { instanceId, requestId, operation: 'play', phase: 'exited', exitSuccess: false, exitCode: 1, exitExpected: false };
  const snapshot = { processSessionId: 'process', instances: [entry] };
  assert.equal(unacknowledgedCrashes(snapshot, new Set()).length, 1);
  const dismissed = new Set([`process:${requestId}`]);
  assert.equal(unacknowledgedCrashes(snapshot, dismissed).length, 0);
  assert.equal(unacknowledgedCrashes({ ...snapshot, processSessionId: 'other' }, dismissed).length, 1);
  for (const change of [{ exitSuccess: true }, { exitExpected: true }, { operation: 'repair' }, { phase: 'running' }]) {
    assert.equal(unacknowledgedCrashes({ ...snapshot, instances: [{ ...entry, ...change }] }, new Set()).length, 0);
  }
});
