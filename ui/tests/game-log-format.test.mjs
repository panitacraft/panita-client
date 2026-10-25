import assert from 'node:assert/strict';
import test from 'node:test';
import { formattedGameLines, logMetadata, displayLogTime, logPrefix } from '../src/game-log-format.ts';

const header = '<log4j:Event timestamp="1720000000123" level="warn" thread="Render thread" logger="a&gt;b">';
test('inline Log4j events expose metadata and preserve inert CDATA', () => {
  const [line] = formattedGameLines([{ stream: 'stdout', sequence: 9, text: header + '<log4j:Message><![CDATA[ready <script> &amp;]]></log4j:Message></log4j:Event>' }]);
  assert.equal(line.text, 'ready <script> &amp;');
  assert.equal(line.sequence, 9);
  assert.deepEqual(line.metadata, { timestamp: '1720000000123', level: 'WARN', thread: 'Render thread', logger: 'a>b' });
  assert.match(logPrefix(line.metadata), /\[\d\d:\d\d:\d\d\.123\] \[Render thread\/WARN\] \[a>b\]/);
});
test('a page starting inside CDATA uses backend context and keeps streams independent', () => {
  const lines = formattedGameLines([
    { stream: 'stdout', text: '<log4j:Event literal &amp;', context: header, cdata: true },
    { stream: 'stderr', text: 'plain error', context: null, cdata: false },
    { stream: 'stdout', text: 'end]]></log4j:Message></log4j:Event>', context: header, cdata: true },
    { stream: 'stdout', text: 'plain output', context: null, cdata: false },
  ]);
  assert.equal(lines[0].text, '<log4j:Event literal &amp;');
  assert.equal(lines[0].metadata.level, 'WARN');
  assert.equal(lines[1].metadata, null);
  assert.equal(lines[2].text, 'end');
  assert.equal(lines[3].metadata, null);
});
test('known XML entities decode safely while malformed and unknown markup remains visible', () => {
  const lines = formattedGameLines([
    { stream: 'stdout', text: header + '<log4j:Message>&lt;ok&gt; &#65; &#x1f600; &#0; <unknown>text</unknown></log4j:Message></log4j:Event>' },
    { stream: 'stdout', text: '<log4j:Event level="unterminated' },
  ]);
  assert.equal(lines[0].text, '<ok> A 😀 &#0; <unknown>text</unknown>');
  assert.equal(lines[1].text, '<log4j:Event level="unterminated');
  assert.equal(displayLogTime('invalid'), '');
  assert.equal(displayLogTime('9999999999999999'), '');
  assert.equal(logMetadata(null), null);
});
test('many inline XML events never expand a single source row into unbounded DOM rows', () => {
  const text = (header + '<log4j:Message>x</log4j:Message></log4j:Event>').repeat(50);
  const lines = formattedGameLines([{ stream: 'stdout', text }]);
  assert.equal(lines.length, 1);
  assert.equal(lines[0].text.match(/x/g).length, 50);
});
