import assert from 'node:assert/strict';
import test from 'node:test';
import { settingsDraft, globalSettingsPayload, instanceSettingsPayload } from '../src/settings-state.ts';

function snapshot(instance = null) {
  return { instanceId: instance ? 'instance' : null, instance, windowWarning: null,
    global: { schema_version: 2, ram_mib: 4096, extra_jvm_args: ['-Dlabel=value with spaces'], java_path_override: null,
      window_size: { width: 1280, height: 720 }, launcher: { selected_instance_id: 'selected', window: { width: 1000, height: 600, maximized: true } } } };
}
const inherited = () => ({ schema_version: 1, ram_mib: null, extra_jvm_args: null, java_path_override: null, window_size: null });

test('global edits keep literal JVM arguments and exclude launcher preferences from the editable payload', () => {
  const view = snapshot();
  const draft = settingsDraft(view);
  draft.ram = '8192';
  draft.jvm = '-Dlabel=value with spaces\r\n-XX:+UseG1GC\r\n';
  const payload = globalSettingsPayload(draft, view);
  assert.equal(payload.ram_mib, 8192);
  assert.deepEqual(payload.extra_jvm_args, ['-Dlabel=value with spaces', '-XX:+UseG1GC']);
  assert.equal(Object.hasOwn(payload, 'launcher'), false);
  assert.equal(view.global.launcher.selected_instance_id, 'selected');
});

test('inherited instance values remain null while explicit empty JVM options override global options', () => {
  const view = snapshot(inherited());
  const draft = settingsDraft(view);
  assert.equal(draft.ram, '4096');
  assert.equal(draft.jvm, '-Dlabel=value with spaces');
  assert.deepEqual(instanceSettingsPayload(draft, view), inherited());
  draft.inheritJvm = false;
  draft.jvm = '';
  assert.deepEqual(instanceSettingsPayload(draft, view).extra_jvm_args, []);
});

test('instance overrides keep their own RAM and window without copying inherited fields', () => {
  const view = snapshot({ ...inherited(), ram_mib: 6144, window_size: { width: 1024, height: 768 } });
  const draft = settingsDraft(view);
  assert.equal(draft.inheritRam, false);
  assert.equal(draft.inheritWindow, false);
  assert.equal(draft.inheritJvm, true);
  draft.ram = '8192';
  assert.deepEqual(instanceSettingsPayload(draft, view), { ...view.instance, ram_mib: 8192 });
  draft.inheritRam = true;
  draft.ram = 'invalid unused input';
  assert.equal(instanceSettingsPayload(draft, view).ram_mib, null);
});

test('invalid numeric input is rejected instead of truncating or coercing it', () => {
  const view = snapshot();
  for (const ram of ['', '512.5', '1e3', '-1', '511', '262145', 'Infinity', '9007199254740993']) {
    assert.throws(() => globalSettingsPayload({ ...settingsDraft(view), ram }, view), /invalid_data/);
  }
  for (const patch of [{ width: '319' }, { height: '239' }, { width: '16385' }, { height: 'NaN' }]) {
    assert.throws(() => globalSettingsPayload({ ...settingsDraft(view), ...patch }, view), /invalid_data/);
  }
});

test('JVM control characters, argument count and length limits fail before IPC', () => {
  const view = snapshot();
  for (const jvm of ['-Dtest=bad\0value', '-Dtest=bad\tvalue', '-Dtest=bad\u007fvalue', 'x'.repeat(4097), Array(65).fill('-Dtest=value').join('\n')]) {
    assert.throws(() => globalSettingsPayload({ ...settingsDraft(view), jvm }, view), /invalid_data/);
  }
});

test('a stored custom Java override is removed only by an explicit edit in its own scope', () => {
  const globalView = snapshot();
  globalView.global.java_path_override = 'C:/custom/java.exe';
  const globalDraft = settingsDraft(globalView);
  assert.equal(globalSettingsPayload(globalDraft, globalView).java_path_override, 'C:/custom/java.exe');
  globalDraft.clearJava = true;
  assert.equal(globalSettingsPayload(globalDraft, globalView).java_path_override, null);
  const instanceView = snapshot({ ...inherited(), java_path_override: '/custom/bin/java' });
  const instanceDraft = settingsDraft(instanceView);
  assert.equal(instanceSettingsPayload(instanceDraft, instanceView).java_path_override, '/custom/bin/java');
  instanceDraft.clearJava = true;
  assert.equal(instanceSettingsPayload(instanceDraft, instanceView).java_path_override, null);
});
