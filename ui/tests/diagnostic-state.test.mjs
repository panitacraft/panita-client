import assert from 'node:assert/strict';
import test from 'node:test';
import { diagnosticText, visibleDiagnosticRecords } from '../src/diagnostic-state.ts';

test('diagnostics render only fixed Spanish messages and ignore unknown raw strings', () => {
  assert.match(diagnosticText({ event: 'application_started' }), /Se inició/);
  assert.match(diagnosticText({ event: 'workflow_changed', workflow: 'repair', phase: 'failed' }), /Reparación: falló/);
  assert.match(diagnosticText({ event: 'workflow_changed', workflow: 'play', phase: 'exited_with_error' }), /inesperada/);
  for (const record of [
    { event: 'fixture-sensitive-response' },
    { event: 'workflow_changed', workflow: 'fixture-sensitive-response', phase: 'failed' },
    { event: 'workflow_changed', workflow: 'play', phase: 'fixture-sensitive-response' },
    { event: 'workflow_changed', workflow: 'constructor', phase: 'failed' },
  ]) {
    assert.equal(diagnosticText(record), 'Evento del cliente no reconocido por esta versión.');
    assert.equal(diagnosticText(record).includes('fixture-sensitive-response'), false);
  }
});

test('diagnostic rendering stays bounded and newest first without mutating the snapshot', () => {
  const records = Array.from({ length: 300 }, (_, index) => ({ operationId: String(index) }));
  const visible = visibleDiagnosticRecords({ capacity: 999, records });
  assert.equal(visible.length, 256);
  assert.equal(visible[0].operationId, '299');
  assert.equal(visible.at(-1).operationId, '44');
  assert.equal(records[0].operationId, '0');
  assert.deepEqual(visibleDiagnosticRecords({ capacity: 256, records: [] }), []);
});
