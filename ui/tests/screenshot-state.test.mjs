import assert from 'node:assert/strict';
import test from 'node:test';
import {
  SCREENSHOTS_PER_PAGE, adjacentScreenshot, clampScreenshotPage, indexAfterRemoval, screenshotActionError, screenshotChoices, screenshotDateText, screenshotPage,
  screenshotPageCount, screenshotPageOf, screenshotSizeText, sameScreenshotList, groupScreenshotsByMonth,
} from '../src/screenshot-state.ts';

const shots = count => Array.from({ length: count }, (_, index) => ({ name: `s${index}.png`, size: 1, modifiedMs: index }));

test('pages hold a fixed number of captures and an empty list still has one page', () => {
  assert.equal(screenshotPageCount(0), 1);
  assert.equal(screenshotPageCount(SCREENSHOTS_PER_PAGE), 1);
  assert.equal(screenshotPageCount(SCREENSHOTS_PER_PAGE + 1), 2);
  const list = shots(SCREENSHOTS_PER_PAGE * 2 + 3);
  assert.equal(screenshotPage(list, 0).length, SCREENSHOTS_PER_PAGE);
  assert.deepEqual(screenshotPage(list, 2).map(item => item.name), ['s24.png', 's25.png', 's26.png']);
  assert.deepEqual(screenshotPage([], 0), []);
});

test('a remembered page is clamped when the list shrinks', () => {
  assert.equal(clampScreenshotPage(5, 13), 1);
  assert.equal(clampScreenshotPage(-2, 13), 0);
  assert.equal(clampScreenshotPage(3, 0), 0);
  assert.deepEqual(screenshotPage(shots(3), 9).map(item => item.name), ['s0.png', 's1.png', 's2.png']);
});

test('sizes and dates are shown in Spanish and tolerate missing values', () => {
  assert.equal(screenshotSizeText(2.5 * 1024 * 1024), '2,5 MB');
  assert.equal(screenshotSizeText(512 * 1024), '512 KB');
  assert.equal(screenshotSizeText(0), '1 KB');
  assert.equal(screenshotDateText(0), '');
  assert.equal(screenshotDateText(Number.NaN), '');
  assert.notEqual(screenshotDateText(1_700_000_000_000), '');
});

test('a rejected name asks for a refresh while other failures name the action', () => {
  assert.match(screenshotActionError('copy', 'invalid_data'), /ya no está disponible/);
  assert.match(screenshotActionError('copy', 'storage_unavailable'), /portapapeles/);
  assert.match(screenshotActionError('open', 'storage_unavailable'), /visor de imágenes/);
  assert.match(screenshotActionError('reveal', undefined), /explorador de archivos/);
});

test('the viewer stays inside the list, goes to the next capture after a removal and finds its page', () => {
  assert.equal(adjacentScreenshot(0, -1, 3), 0);
  assert.equal(adjacentScreenshot(0, 1, 3), 1);
  assert.equal(adjacentScreenshot(2, 1, 3), 2);
  assert.equal(adjacentScreenshot(0, 1, 0), 0);
  assert.equal(indexAfterRemoval(1, 2), 1);
  assert.equal(indexAfterRemoval(2, 2), 1);
  assert.equal(indexAfterRemoval(0, 0), null);
  assert.equal(screenshotPageOf(0), 0);
  assert.equal(screenshotPageOf(SCREENSHOTS_PER_PAGE - 1), 0);
  assert.equal(screenshotPageOf(SCREENSHOTS_PER_PAGE), 1);
});

test('the menu lists the actions in order with Eliminar last, and a deletion failure is explained', () => {
  assert.deepEqual(screenshotChoices.map(item => item.choice), ['view', 'open', 'reveal', 'copy', 'delete']);
  assert.deepEqual(screenshotChoices.map(item => item.label), ['Ver', 'Abrir en fotos', 'Mostrar en la carpeta', 'Copiar imagen', 'Eliminar']);
  assert.match(screenshotActionError('delete', 'storage_busy'), /papelera/);
  assert.match(screenshotActionError('delete', 'invalid_data'), /ya no está disponible/);
});

test('a listing that says nothing new is recognized so the screen is left alone', () => {
  const base = { screenshots: shots(2), truncated: false };
  assert.equal(sameScreenshotList(null, base), false);
  assert.equal(sameScreenshotList(base, { screenshots: shots(2), truncated: false }), true);
  assert.equal(sameScreenshotList(base, { screenshots: shots(3), truncated: false }), false);
  assert.equal(sameScreenshotList(base, { screenshots: shots(2), truncated: true }), false);
  assert.equal(sameScreenshotList(base, { screenshots: [{ ...base.screenshots[0], modifiedMs: 99 }, base.screenshots[1]], truncated: false }), false);
  assert.equal(sameScreenshotList({ screenshots: [], truncated: false }, { screenshots: [], truncated: false }), true);
});

test('screenshots are grouped into month runs with Spanish headings, keeping their order', () => {
  const at = (year, month, day, hour = 12) => ({ name: `${year}-${month}-${day}-${hour}.png`, size: 1, modifiedMs: new Date(year, month - 1, day, hour).getTime() });
  const items = [at(2026, 11, 23, 18), at(2026, 11, 2), at(2026, 10, 31, 23), at(2026, 5, 1), at(2025, 5, 1)];
  const groups = groupScreenshotsByMonth(items);
  assert.deepEqual(groups.map(group => group.label), ['Noviembre 2026', 'Octubre 2026', 'Mayo 2026', 'Mayo 2025']);
  assert.deepEqual(groups.map(group => group.items.length), [2, 1, 1, 1]);
  assert.deepEqual(groups.flatMap(group => group.items), items);
  assert.equal(new Set(groups.map(group => group.key)).size, 4);
  assert.deepEqual(groupScreenshotsByMonth([]), []);
  const undated = groupScreenshotsByMonth([{ name: 'a.png', size: 1, modifiedMs: 0 }, { name: 'b.png', size: 1, modifiedMs: 0 }]);
  assert.deepEqual(undated.map(group => [group.label, group.items.length]), [['Sin fecha', 2]]);
});
