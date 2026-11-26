import assert from 'node:assert/strict';
import test from 'node:test';
import { settingsDraft, globalSettingsPayload, instanceSettingsPayload, updateSettingsDraft } from '../src/settings-state.ts';

function snapshot(instance = null) {
  return { instanceId: instance ? 'instance' : null, instance, windowWarning: null,
    global: { schema_version: 2, ram_mib: 4096, extra_jvm_args: ['-Dlabel=value with spaces'], java_path_override: null, show_console_on_start: false,
      window_size: { width: 1280, height: 720 }, launcher: { selected_instance_id: 'selected', window: { width: 1000, height: 600, maximized: true } } } };
}
const inherited = () => ({ schema_version: 1, ram_mib: null, ram_auto: null, extra_jvm_args: null, java_path_override: null, window_size: null, show_console_on_start: null });

test('automatic console settings preserve inheritance and explicit on/off overrides', () => {
  const global = snapshot();
  assert.equal(settingsDraft(global).showConsole, false);
  const enabled = updateSettingsDraft(settingsDraft(global), { showConsole: true }, global);
  assert.equal(globalSettingsPayload(enabled, global).show_console_on_start, true);
  const view = snapshot(inherited());
  view.global.show_console_on_start = true;
  let draft = settingsDraft(view);
  assert.equal(draft.showConsole, true);
  assert.equal(instanceSettingsPayload(draft, view).show_console_on_start, null);
  draft = updateSettingsDraft(draft, { inheritConsole: false, showConsole: false }, view);
  assert.equal(instanceSettingsPayload(draft, view).show_console_on_start, false);
  draft = updateSettingsDraft(draft, { inheritConsole: true }, view);
  assert.equal(draft.showConsole, true);
  assert.equal(instanceSettingsPayload(draft, view).show_console_on_start, null);
  const explicit = snapshot({ ...inherited(), show_console_on_start: true });
  assert.equal(settingsDraft(explicit).inheritConsole, false);
  assert.equal(settingsDraft(explicit).showConsole, true);
});

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
  assert.equal(draft.jvm, '-Xmx4096M\n-Dlabel=value with spaces');
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
  Object.assign(draft, updateSettingsDraft(draft, { ram: '8192' }, view));
  assert.deepEqual(instanceSettingsPayload(draft, view), { ...view.instance, ram_mib: 8192, ram_auto: false });
  Object.assign(draft, updateSettingsDraft(draft, { inheritRam: true }, view));
  draft.ram = 'invalid unused input';
  assert.equal(instanceSettingsPayload(draft, view).ram_mib, null);
});

test('RAM input and JVM edits share one memory value without duplicate launch arguments', () => {
  const view = snapshot();
  let draft = settingsDraft(view);
  draft = updateSettingsDraft(draft, { ram: '6144' }, view);
  assert.equal(draft.jvm, '-Xmx6144M\n-Dlabel=value with spaces');
  draft = updateSettingsDraft(draft, { jvm: '-Xmx8G\n-Dlabel=value with spaces' }, view);
  assert.equal(draft.ram, '8192');
  assert.equal(globalSettingsPayload(draft, view).ram_mib, 8192);
  assert.deepEqual(globalSettingsPayload(draft, view).extra_jvm_args, ['-Dlabel=value with spaces']);
  for (const jvm of ['-Xmx', '-Xmx0G', '-Xmx4G\n-Xmx8G', '-Xmx4T']) {
    const bad = updateSettingsDraft(draft, { jvm }, view);
    assert.throws(() => globalSettingsPayload(bad, view), /invalid_data/);
  }
});

test('editing inherited RAM in the text creates only a RAM override and rejects incomplete edits', () => {
  const view = snapshot(inherited());
  const draft = updateSettingsDraft(settingsDraft(view), { jvm: '-Xmx6G\n-Dlabel=value with spaces' }, view);
  assert.equal(draft.inheritRam, false);
  assert.equal(draft.inheritJvm, true);
  assert.equal(instanceSettingsPayload(draft, view).ram_mib, 6144);
  assert.equal(instanceSettingsPayload(draft, view).extra_jvm_args, null);
  const bad = updateSettingsDraft(settingsDraft(view), { jvm: '-Xmx\n-Dlabel=value with spaces' }, view);
  assert.throws(() => instanceSettingsPayload(bad, view), /invalid_data/);
  const reset = updateSettingsDraft(draft, { inheritRam: true }, view);
  assert.equal(reset.ram, '4096');
  assert.match(reset.jvm, /^-Xmx4096M/);
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

test('automatic RAM: a saved global value stays manual, switching keeps the number and saves both', () => {
  const view = snapshot();
  assert.equal(settingsDraft(view).ramAuto, false, 'data without the option is manual');
  let draft = updateSettingsDraft(settingsDraft(view), { ramAuto: true }, view);
  assert.equal(draft.ram, '4096', 'the manual value is kept');
  assert.equal(draft.jvm, '-Dlabel=value with spaces', 'no heap argument while automatic');
  let payload = globalSettingsPayload(draft, view);
  assert.equal(payload.ram_auto, true);
  assert.equal(payload.ram_mib, 4096);
  draft = updateSettingsDraft(draft, { ramAuto: false }, view);
  assert.equal(draft.jvm, '-Xmx4096M\n-Dlabel=value with spaces');
  payload = globalSettingsPayload(draft, view);
  assert.equal(payload.ram_auto, false);
  // A global value that was already automatic opens automatic.
  const auto = snapshot();
  auto.global.ram_auto = true;
  assert.equal(settingsDraft(auto).ramAuto, true);
});

test('automatic RAM: typing a heap argument while automatic chooses manual with that value', () => {
  const view = snapshot();
  view.global.ram_auto = true;
  const draft = updateSettingsDraft(settingsDraft(view), { jvm: '-Xmx6G\n-Dlabel=value with spaces' }, view);
  assert.equal(draft.ramAuto, false);
  assert.equal(draft.ram, '6144');
  assert.equal(globalSettingsPayload(draft, view).ram_mib, 6144);
  // A heap argument that survived in automatic mode is refused instead of silently ignored.
  assert.throws(() => globalSettingsPayload({ ...settingsDraft(view), jvm: '-Xmx6G' }, view), /invalid_data/);
});

test('automatic RAM: an instance choice beats the global one in every combination and legacy values stay manual', () => {
  const globalAuto = (instance) => { const view = snapshot(instance); view.global.ram_auto = true; return view; };
  // Inheriting follows the global choice.
  assert.equal(settingsDraft(globalAuto(inherited())).ramAuto, true);
  assert.equal(settingsDraft(globalAuto(inherited())).inheritRam, true);
  assert.equal(settingsDraft(snapshot(inherited())).ramAuto, false);
  // A manual value saved before the option existed is not turned automatic by the global choice.
  const legacy = globalAuto({ ...inherited(), ram_auto: undefined, ram_mib: 6144 });
  assert.equal(settingsDraft(legacy).ramAuto, false);
  assert.equal(settingsDraft(legacy).inheritRam, false);
  assert.equal(instanceSettingsPayload(settingsDraft(legacy), legacy).ram_auto, false);
  // Explicit choices, with or without a stored value.
  const explicitManual = globalAuto({ ...inherited(), ram_mib: 6144, ram_auto: false });
  assert.equal(settingsDraft(explicitManual).ramAuto, false);
  const explicitAuto = snapshot({ ...inherited(), ram_mib: 6144, ram_auto: true });
  const draft = settingsDraft(explicitAuto);
  assert.deepEqual([draft.ramAuto, draft.ram, draft.inheritRam], [true, '6144', false]);
  assert.deepEqual(instanceSettingsPayload(draft, explicitAuto), { ...explicitAuto.instance });
  // Choosing automatic for one instance writes an explicit choice and keeps the value.
  const view = snapshot(inherited());
  let next = updateSettingsDraft(settingsDraft(view), { inheritRam: false }, view);
  next = updateSettingsDraft(next, { ramAuto: true }, view);
  const payload = instanceSettingsPayload(next, view);
  assert.deepEqual([payload.ram_auto, payload.ram_mib], [true, 4096]);
  // Going back to inheriting clears both and follows the global choice again.
  const back = updateSettingsDraft(next, { inheritRam: true }, view);
  assert.deepEqual([instanceSettingsPayload(back, view).ram_auto, instanceSettingsPayload(back, view).ram_mib], [null, null]);
  assert.equal(back.ramAuto, false);
  const backAuto = updateSettingsDraft(next, { inheritRam: true }, globalAuto(inherited()));
  assert.equal(backAuto.ramAuto, true);
  assert.equal(backAuto.jvm, '-Dlabel=value with spaces');
});
