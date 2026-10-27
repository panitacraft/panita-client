import assert from 'node:assert/strict';
import test from 'node:test';
import { accountFeedback, initialAccountFeedback } from '../src/account-state.ts';

const snapshot = (revision, selected = 'first', overrides = {}) => ({
  revision, selected_account_id: selected, login_pending: false, login_error: null,
  accounts: [{ metadata: { account_id: selected, kind: 'microsoft', username: 'Player', profile_id: 'profile', oauth_client_id: 'public-client' }, status: 'ready', error: null, persisted: true }],
  ...overrides,
});
const ready = () => accountFeedback(initialAccountFeedback, { type: 'snapshot', view: snapshot(1) });
const start = (state, operation = 1, command = 'validate_account') => accountFeedback(state, { type: 'start', operation, command });
const finish = (state, operation = 1, extra = {}) => accountFeedback(state, { type: 'finish', operation, ...extra });

test('late initial snapshot cannot restore an account removed by a newer event', () => {
  const removed = accountFeedback(ready(), { type: 'snapshot', view: snapshot(4, null, { accounts: [] }) });
  assert.equal(accountFeedback(removed, { type: 'snapshot', view: snapshot(2) }), removed);
  assert.equal(removed.view.accounts.length, 0);
});

test('late login completion cannot replace cancellation feedback or clear newer pending work', () => {
  const cancelled = start(start(ready(), 1, 'start_microsoft_login'), 2, 'cancel_microsoft_login');
  assert.equal(finish(cancelled, 1, { notice: 'Abre el navegador.' }), cancelled);
  const next = finish(cancelled, 2);
  assert.equal(next.busy, null);
  assert.equal(next.notice, null);
});

test('late validation success and failure belong only to the account that requested them', () => {
  for (const result of [{ notice: 'Minecraft autorizó la sesión.' }, { error: 'No pudimos conectar.' }]) {
    const pending = start(ready());
    const switched = accountFeedback(pending, { type: 'snapshot', view: snapshot(2, 'second') });
    const next = finish(switched, 1, { ...result, selectedAccountId: 'first' });
    assert.equal(next.notice, null);
    assert.equal(next.error, null);
    assert.equal(next.busy, null);
    assert.equal(next.view.selected_account_id, 'second');
  }
});

test('a new login and a callback error clear earlier validation notices', () => {
  const validated = finish(start(ready()), 1, { notice: 'Minecraft autorizó la sesión.', selectedAccountId: 'first' });
  for (const overrides of [{ login_pending: true }, { login_error: 'authentication_denied' }]) {
    const next = accountFeedback(validated, { type: 'snapshot', view: snapshot(2, 'first', overrides) });
    assert.equal(next.notice, null);
    assert.deepEqual(next.view, snapshot(2, 'first', overrides));
  }
});

test('retrying initialization clears loading and error state after a valid snapshot', () => {
  const failed = accountFeedback(initialAccountFeedback, { type: 'initialization_failed', error: 'No disponible.', blocked: false });
  const retrying = start(failed, 1, 'initialize');
  const recovered = accountFeedback(retrying, { type: 'snapshot', view: snapshot(3) });
  assert.equal(recovered.error, null);
  assert.equal(recovered.busy, null);
  assert.equal(recovered.blocked, false);
});

test('corrupt account storage remains a blocked error without fabricating an empty account list', () => {
  const failed = accountFeedback(initialAccountFeedback, { type: 'initialization_failed', error: 'Archivo dañado.', blocked: true });
  assert.equal(failed.view, null);
  assert.equal(failed.blocked, true);
  assert.equal(failed.error, 'Archivo dañado.');
});

test('incomplete sign-out cleanup remains visible after the removed account changes selection', () => {
  const removing = start(ready(), 1, 'remove_account');
  const removed = accountFeedback(removing, { type: 'snapshot', view: snapshot(2, 'second') });
  const selecting = start(removed, 2, 'select_account');
  const warned = accountFeedback(selecting, { type: 'cleanup_warning', warning: 'La eliminación del archivo quedó pendiente.' });
  const next = finish(warned, 2);
  assert.equal(next.cleanupWarning, 'La eliminación del archivo quedó pendiente.');
  assert.equal(next.view.selected_account_id, 'second');
  assert.equal(start(next, 3, 'validate_account').cleanupWarning, next.cleanupWarning);
});

test('renewal failure remains inline and explicit retry can transition back to ready', () => {
  const pending = start(ready(), 1, 'renew_account');
  const failure = snapshot(2);
  failure.accounts[0].status = 'error';
  failure.accounts[0].error = 'network_unavailable';
  const failed = finish(accountFeedback(pending, { type: 'snapshot', view: failure }), 1, { selectedAccountId: 'first' });
  assert.equal(failed.error, null);
  assert.equal(failed.view.accounts[0].error, 'network_unavailable');
  const retrying = start(failed, 2, 'renew_account');
  const recovered = finish(accountFeedback(retrying, { type: 'snapshot', view: snapshot(3) }), 2, { selectedAccountId: 'first' });
  assert.equal(recovered.view.accounts[0].status, 'ready');
  assert.equal(recovered.view.accounts[0].error, null);
});

test('an unsaved offline account remains explicitly offline until a saved snapshot arrives', () => {
  const view = snapshot(2);
  view.accounts[0].metadata.kind = 'offline';
  view.accounts[0].metadata.oauth_client_id = null;
  view.accounts[0].status = 'offline';
  view.accounts[0].persisted = false;
  const unsaved = accountFeedback(ready(), { type: 'snapshot', view });
  assert.equal(unsaved.view.accounts[0].persisted, false);
  const savedView = structuredClone(view);
  savedView.revision = 3;
  savedView.accounts[0].persisted = true;
  const saved = finish(accountFeedback(start(unsaved, 1, 'retry_account_save'), { type: 'snapshot', view: savedView }), 1, { notice: 'La cuenta se guardó correctamente.' });
  assert.equal(saved.view.accounts[0].metadata.kind, 'offline');
  assert.equal(saved.view.accounts[0].persisted, true);
  assert.equal(saved.notice, 'La cuenta se guardó correctamente.');
});
