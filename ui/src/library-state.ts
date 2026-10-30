import type { CatalogInstance, PackSource } from './preview-catalog';

/** A custom or Modrinth pack, as opposed to a vanilla version. */
export function isPack(instance: CatalogInstance): boolean {
  return instance.source?.type === 'custom' || instance.source?.type === 'modrinth';
}

/** The exact pinned version of a pack source: the custom version id or Modrinth version id. */
export function packPin(source: PackSource | null | undefined): string | null {
  if (!source) return null;
  if (source.type === 'custom') return source.current_version_id ?? null;
  if (source.type === 'modrinth') return source.pack_version_id ?? null;
  return null;
}

export function canInstall(instance: CatalogInstance, stale: boolean): boolean {
  return !instance.removalPending && !stale && instance.availability === 'enabled' && instance.compatible && instance.supported
    && ['not_installed', 'failed', 'cancelled', 'needs_repair', 'installing'].includes(instance.localState)
    // A recorded pin can exist before an installation finishes. Retrying that
    // pin is safe; changing it belongs to the explicit update workflow.
    && (instance.installedVersion === null || instance.installedVersion === instance.minecraftVersion);
}

export type UpdateTarget = { fromVersion: string; toVersion: string; pack?: boolean };
export function updateTarget(instance: CatalogInstance, stale: boolean): UpdateTarget | null {
  if (instance.removalPending) return null;
  if (isPack(instance)) {
    // A newer published pack version replaces Play with Update until it is applied.
    const from = packPin(instance.installedSource);
    const to = packPin(instance.source);
    if (stale || instance.availability !== 'enabled' || !instance.compatible || !instance.supported
      || instance.localState !== 'update_available' || !from || !to || from === to) return null;
    return { fromVersion: from, toVersion: to, pack: true };
  }
  if (stale || instance.availability !== 'enabled' || !instance.compatible || !instance.supported
    || !['update_available', 'not_installed', 'failed', 'cancelled', 'needs_repair'].includes(instance.localState) || !instance.installedVersion || !instance.minecraftVersion
    || instance.installedVersion === instance.minecraftVersion) return null;
  return { fromVersion: instance.installedVersion, toVersion: instance.minecraftVersion };
}

/** A pack counts as installed when its recorded source names a pin. */
function installedPack(instance: CatalogInstance): boolean {
  return isPack(instance) && packPin(instance.installedSource) !== null;
}

/** Keep maintenance reachable for an existing installation that needs repair. */
export function canOpenInstanceOptions(instance: CatalogInstance): boolean {
  return instance.removalPending || instance.hasInstallationManifest
    && (instance.installedVersion !== null || installedPack(instance))
    && ['installed', 'update_available', 'needs_repair', 'running', 'failed', 'cancelled'].includes(instance.localState);
}

export function canRepair(instance: CatalogInstance, stale: boolean): boolean {
  return !instance.removalPending && instance.hasInstallationManifest && !stale && instance.availability === 'enabled' && instance.compatible && instance.supported
    && (instance.installedVersion !== null || installedPack(instance))
    && ['installed', 'update_available', 'needs_repair', 'failed', 'cancelled'].includes(instance.localState);
}

export function canUninstall(instance: CatalogInstance): boolean {
  return instance.removalPending || instance.hasInstallationManifest && (instance.installedVersion !== null || installedPack(instance))
    && instance.localState !== 'not_installed' && instance.localState !== 'running';
}
