export type WindowSize = { width: number; height: number };
export type GlobalLaunchSettings = {
  schema_version: number; ram_mib: number; extra_jvm_args: string[];
  java_path_override: string | null; window_size: WindowSize;
};
export type GlobalSettings = GlobalLaunchSettings & {
  launcher: { selected_instance_id: string | null; window: (WindowSize & { maximized: boolean }) | null };
};
export type InstanceSettings = {
  schema_version: number; ram_mib: number | null; extra_jvm_args: string[] | null;
  java_path_override: string | null; window_size: WindowSize | null;
};
export type SettingsSnapshot = { global: GlobalSettings; instance: InstanceSettings | null; instanceId: string | null; windowWarning: string | null };
export type SettingsDraft = {
  ram: string; width: string; height: string; jvm: string;
  inheritRam: boolean; inheritWindow: boolean; inheritJvm: boolean;
  clearJava: boolean;
};

export function settingsDraft(snapshot: SettingsSnapshot): SettingsDraft {
  const { global, instance } = snapshot;
  return {
    ram: String(instance?.ram_mib ?? global.ram_mib),
    width: String(instance?.window_size?.width ?? global.window_size.width),
    height: String(instance?.window_size?.height ?? global.window_size.height),
    jvm: (instance?.extra_jvm_args ?? global.extra_jvm_args).join('\n'),
    inheritRam: instance?.ram_mib == null,
    inheritWindow: instance?.window_size == null,
    inheritJvm: instance?.extra_jvm_args == null,
    clearJava: false,
  };
}

function integer(value: string, min: number, max: number): number {
  if (!/^\d+$/.test(value)) throw new Error('invalid_data');
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < min || parsed > max) throw new Error('invalid_data');
  return parsed;
}

function jvmArguments(value: string): string[] {
  const args = value.split(/\r?\n/).filter(line => line.length > 0);
  if (args.length > 64 || args.some(arg => arg.length > 4096 || /[\x00-\x1f\x7f]/.test(arg))) throw new Error('invalid_data');
  return args;
}

export function globalSettingsPayload(draft: SettingsDraft, snapshot: SettingsSnapshot): GlobalLaunchSettings {
  return {
    schema_version: 2, ram_mib: integer(draft.ram, 512, 262144),
    window_size: { width: integer(draft.width, 320, 16384), height: integer(draft.height, 240, 16384) },
    extra_jvm_args: jvmArguments(draft.jvm),
    java_path_override: draft.clearJava ? null : snapshot.global.java_path_override,
  };
}

export function instanceSettingsPayload(draft: SettingsDraft, snapshot: SettingsSnapshot): InstanceSettings {
  return {
    schema_version: 1, ram_mib: draft.inheritRam ? null : integer(draft.ram, 512, 262144),
    window_size: draft.inheritWindow ? null : { width: integer(draft.width, 320, 16384), height: integer(draft.height, 240, 16384) },
    extra_jvm_args: draft.inheritJvm ? null : jvmArguments(draft.jvm),
    java_path_override: draft.clearJava ? null : snapshot.instance?.java_path_override ?? null,
  };
}
