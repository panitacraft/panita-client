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
    jvm: [`-Xmx${instance?.ram_mib ?? global.ram_mib}M`, ...(instance?.extra_jvm_args ?? global.extra_jvm_args)].join('\n'),
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
  if (args.length > 65 || args.some(arg => arg.length > 4096 || /[\x00-\x1f\x7f]/.test(arg))) throw new Error('invalid_data');
  return args;
}

function memoryArgument(arg: string): number | null {
  const match = /^-Xmx(\d+)([mMgG])$/.exec(arg);
  if (!match) return null;
  return integer(String(Number(match[1]) * (match[2].toLowerCase() === 'g' ? 1024 : 1)), 512, 262144);
}

function extraArguments(draft: SettingsDraft): string[] {
  const args = jvmArguments(draft.jvm);
  const memory = args.filter(arg => arg.startsWith('-Xmx'));
  if (memory.length > 1 || (memory.length === 1 && memoryArgument(memory[0]) !== integer(draft.ram, 512, 262144))) throw new Error('invalid_data');
  const extras = args.filter(arg => !arg.startsWith('-Xmx'));
  if (extras.length > 64) throw new Error('invalid_data');
  return extras;
}

/** Keep a single RAM value; the editable memory argument never reaches extra JVM options. */
export function updateSettingsDraft(draft: SettingsDraft, patch: Partial<SettingsDraft>, snapshot: SettingsSnapshot): SettingsDraft {
  const next = { ...draft, ...patch };
  if (patch.inheritRam === true) next.ram = String(snapshot.global.ram_mib);
  if (patch.inheritJvm === true) next.jvm = [`-Xmx${next.ram}M`, ...snapshot.global.extra_jvm_args].join('\n');
  if (patch.ram !== undefined || patch.inheritRam === true) {
    next.jvm = [`-Xmx${next.ram}M`, ...next.jvm.split(/\r?\n/).filter(line => !line.startsWith('-Xmx') && line.length > 0)].join('\n');
  } else if (patch.jvm !== undefined) {
    const args = next.jvm.split(/\r?\n/).filter(Boolean);
    const memory = args.filter(arg => arg.startsWith('-Xmx'));
    if (memory.length === 1) {
      try {
        const ram = memoryArgument(memory[0]);
        if (ram !== null) { next.ram = String(ram); if (next.ram !== draft.ram) next.inheritRam = false; }
      } catch { /* Preserve an incomplete edit for correction; save will reject it. */ }
    }
    const extras = args.filter(arg => !arg.startsWith('-Xmx'));
    if (JSON.stringify(extras) !== JSON.stringify(snapshot.global.extra_jvm_args)) next.inheritJvm = false;
  }
  return next;
}

export function globalSettingsPayload(draft: SettingsDraft, snapshot: SettingsSnapshot): GlobalLaunchSettings {
  return {
    schema_version: 2, ram_mib: integer(draft.ram, 512, 262144),
    window_size: { width: integer(draft.width, 320, 16384), height: integer(draft.height, 240, 16384) },
    extra_jvm_args: extraArguments(draft),
    java_path_override: draft.clearJava ? null : snapshot.global.java_path_override,
  };
}

export function instanceSettingsPayload(draft: SettingsDraft, snapshot: SettingsSnapshot): InstanceSettings {
  const extras = extraArguments({ ...draft, ram: draft.inheritRam ? String(snapshot.global.ram_mib) : draft.ram });
  return {
    schema_version: 1, ram_mib: draft.inheritRam ? null : integer(draft.ram, 512, 262144),
    window_size: draft.inheritWindow ? null : { width: integer(draft.width, 320, 16384), height: integer(draft.height, 240, 16384) },
    extra_jvm_args: draft.inheritJvm ? null : extras,
    java_path_override: draft.clearJava ? null : snapshot.instance?.java_path_override ?? null,
  };
}
