import assert from 'node:assert/strict';
import test from 'node:test';
import {
  SCREENSHOTS_PER_PAGE, adjacentScreenshot, clampScreenshotPage, indexAfterRemoval, screenshotActionError, screenshotChoices, screenshotDateText, screenshotPage,
  screenshotPageCount, screenshotPageOf, screenshotSizeText,
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
  assert.deepEqual(screenshotChoices.map(item => item.label), ['Ver', 'Abrir en otra app', 'Mostrar en la carpeta', 'Copiar imagen', 'Eliminar']);
  assert.match(screenshotActionError('delete', 'storage_busy'), /papelera/);
  assert.match(screenshotActionError('delete', 'invalid_data'), /ya no está disponible/);
});
