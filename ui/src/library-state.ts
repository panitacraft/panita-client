import type { CatalogInstance } from './preview-catalog';

export function canInstall(instance: CatalogInstance, stale: boolean): boolean {
  return !stale && instance.availability === 'enabled' && instance.compatible && instance.supported
    && ['not_installed', 'failed', 'cancelled', 'needs_repair', 'installing'].includes(instance.localState)
    // A recorded pin can exist before an installation finishes. Retrying that
    // pin is safe; changing it belongs to the explicit update workflow.
    && (instance.installedVersion === null || instance.installedVersion === instance.minecraftVersion);
}
