import assert from 'node:assert/strict';
import test from 'node:test';
import {
  activationFeedback, countdownText, gateStage, initialActivationFeedback, memberName,
  offlineNameSuggestion, registrationHint, remainingSeconds, retryAfterText, signOutNotice,
} from '../src/activation-state.ts';

const user = (overrides = {}) => ({
  id: 42, ign: 'Javithor360', discord_name: 'javithor360', avatar_url: null, minecraft_status: 'premium',
  capabilities: { authenticated: true }, ...overrides,
});
const view = (revision, state = 'no_session', overrides = {}) => ({
  revision, state, user: state.startsWith('active') ? user() : null, verified_at: null, serial: null,
  expires_at: null, persisted: true, error: null, retry_after: null, ...overrides,
});

test('a late first snapshot cannot undo a newer event', () => {
  const active = activationFeedback(initialActivationFeedback, { type: 'snapshot', view: view(5, 'active') });
  assert.equal(activationFeedback(active, { type: 'snapshot', view: view(2, 'no_session') }), active);
  assert.equal(active.view.state, 'active');
});

test('a new serial or a finished activation clears earlier feedback, other changes keep it', () => {
  const failed = { ...initialActivationFeedback, error: 'activation_unknown_account', notice: 'Cerraste la sesión.' };
  const waiting = activationFeedback(failed, { type: 'snapshot', view: view(1, 'pending', { serial: '5V3J-A5TD-YE4L' }) });
  assert.equal(waiting.error, null);
  assert.equal(waiting.notice, null);
  const kept = activationFeedback(
    { ...failed, view: view(1, 'no_session') },
    { type: 'snapshot', view: view(2, 'no_session', { error: 'session_rejected' }) },
  );
  assert.equal(kept.error, 'activation_unknown_account');
  assert.equal(kept.notice, 'Cerraste la sesión.');
  const done = activationFeedback({ ...waiting, error: 'x', view: view(2, 'pending') }, { type: 'snapshot', view: view(3, 'active') });
  assert.equal(done.error, null);
});

test('late command results never replace newer feedback or clear newer work', () => {
  const started = activationFeedback(
    activationFeedback(initialActivationFeedback, { type: 'start', operation: 1, command: 'start_activation' }),
    { type: 'start', operation: 2, command: 'cancel_activation' },
  );
  assert.equal(activationFeedback(started, { type: 'finish', operation: 1, error: 'network_unavailable' }), started);
  const done = activationFeedback(started, { type: 'finish', operation: 2, notice: 'Listo.' });
  assert.equal(done.busy, null);
  assert.equal(done.notice, 'Listo.');
  assert.equal(done.error, null);
});

test('initialization failure is kept until a retry or a snapshot arrives', () => {
  const failed = activationFeedback(initialActivationFeedback, { type: 'initialization_failed', error: 'storage_unavailable' });
  assert.equal(failed.initializationError, 'storage_unavailable');
  assert.equal(activationFeedback(failed, { type: 'retry_initialization' }).initializationError, null);
  assert.equal(activationFeedback(failed, { type: 'snapshot', view: view(1) }).initializationError, null);
});

test('the window shows activation, onboarding or the launcher in that order', () => {
  const base = { desktop: true, view: null, accountCount: null, accountsFailed: false, initializationError: null };
  assert.equal(gateStage({ ...base, desktop: false }), 'launcher');
  assert.equal(gateStage(base), 'loading');
  assert.equal(gateStage({ ...base, initializationError: 'storage_unavailable' }), 'activation');
  for (const state of ['no_session', 'creating', 'pending']) {
    // A saved Minecraft account never skips Panita activation.
    assert.equal(gateStage({ ...base, view: view(1, state), accountCount: 3 }), 'activation');
  }
  for (const state of ['active', 'active_offline']) {
    assert.equal(gateStage({ ...base, view: view(1, state) }), 'loading');
    assert.equal(gateStage({ ...base, view: view(1, state), accountCount: 0 }), 'onboarding');
    assert.equal(gateStage({ ...base, view: view(1, state), accountCount: 2 }), 'launcher');
    // The launcher explains an accounts file problem, so it must stay reachable.
    assert.equal(gateStage({ ...base, view: view(1, state), accountsFailed: true }), 'launcher');
  }
});

test('the countdown never goes negative and rounds up', () => {
  assert.equal(remainingSeconds(null, 0), 0);
  assert.equal(remainingSeconds(100, 99_500), 1);
  assert.equal(remainingSeconds(100, 100_000), 0);
  assert.equal(remainingSeconds(100, 200_000), 0);
  assert.equal(countdownText(179), '2:59');
  assert.equal(countdownText(60), '1:00');
  assert.equal(countdownText(5), '0:05');
  assert.equal(countdownText(-3), '0:00');
});

test('the rate limit message names the wait with the right plural', () => {
  assert.equal(retryAfterText(null), '');
  assert.equal(retryAfterText(0), '');
  assert.match(retryAfterText(1), /1 segundo antes/);
  assert.match(retryAfterText(42), /42 segundos antes/);
});

test('Minecraft status and nickname are hints, never requirements', () => {
  assert.equal(memberName(user()), 'Javithor360');
  assert.equal(memberName(user({ ign: null })), 'javithor360');
  assert.equal(registrationHint(user({ minecraft_status: null })).includes('/register'), true);
  assert.equal(registrationHint(user({ minecraft_status: 'cracked' })), null);
  assert.equal(offlineNameSuggestion(user()), 'Javithor360');
  assert.equal(offlineNameSuggestion(user({ ign: null })), '');
  assert.equal(offlineNameSuggestion(user({ ign: 'has space' })), '');
  assert.equal(offlineNameSuggestion(user({ ign: 'ab' })), '');
});

test('signing out offline says the session stays open on the server', () => {
  assert.doesNotMatch(signOutNotice(true), /sigue abierta/);
  assert.match(signOutNotice(false), /sigue abierta/);
  assert.match(signOutNotice(false), /\/sessions/);
});
