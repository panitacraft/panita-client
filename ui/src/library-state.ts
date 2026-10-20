import type { CatalogInstance } from './preview-catalog';

export function canInstall(instance: CatalogInstance, stale: boolean): boolean {
  return !stale && instance.availability === 'enabled' && instance.compatible && instance.supported
    && ['not_installed', 'failed', 'cancelled', 'needs_repair', 'installing'].includes(instance.localState)
    // A recorded pin can exist before an installation finishes. Retrying that
    // pin is safe; changing it belongs to the explicit update workflow.
    && (instance.installedVersion === null || instance.installedVersion === instance.minecraftVersion);
}

export type UpdateTarget = { fromVersion: string; toVersion: string };
export function updateTarget(instance: CatalogInstance, stale: boolean): UpdateTarget | null {
  if (stale || instance.availability !== 'enabled' || !instance.compatible || !instance.supported
    || !['update_available', 'not_installed', 'failed', 'cancelled', 'needs_repair'].includes(instance.localState) || !instance.installedVersion || !instance.minecraftVersion
    || instance.installedVersion === instance.minecraftVersion) return null;
  return { fromVersion: instance.installedVersion, toVersion: instance.minecraftVersion };
}

export function canRepair(instance: CatalogInstance, stale: boolean): boolean {
  return !stale && instance.availability === 'enabled' && instance.compatible && instance.supported
    && instance.installedVersion !== null
    && ['installed', 'update_available', 'needs_repair', 'failed', 'cancelled'].includes(instance.localState);
}

export function canUninstall(instance: CatalogInstance): boolean {
  return instance.installedVersion !== null && instance.localState !== 'not_installed' && instance.localState !== 'running';
}
