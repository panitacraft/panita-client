export interface CatalogInstance {
  id: string;
  name: string;
  description: string;
  minecraftVersion: string | null;
  enabled: boolean;
  availability: 'enabled' | 'disabled' | 'missing' | 'unverified';
  localState: 'not_installed' | 'installing' | 'installed' | 'update_available' | 'needs_repair' | 'running' | 'failed' | 'cancelled';
  installedVersion: string | null;
  compatible: boolean;
  supported: boolean;
  minLauncherVersion: string | null;
}

export interface CatalogOverview {
  instances: CatalogInstance[];
  stale: boolean;
  cacheWarning: string | null;
  registryWarning: string | null;
}

const defaults = { availability: 'enabled' as const, localState: 'not_installed' as const, installedVersion: null, compatible: true, supported: true, minLauncherVersion: null };

const previewInstances: CatalogInstance[] = [
  {
    ...defaults,
    id: '62d7e99d-fb4e-4167-8732-ce3a334d4a58',
    name: 'Vanilla actual',
    description: 'La última versión estable de Minecraft, sin mods.',
    minecraftVersion: '26.3',
    enabled: true,
  },
  {
    ...defaults,
    id: '65ee13c6-bc34-4914-aa09-41205f3691a0',
    name: 'Vanilla clásica',
    description: 'Una versión anterior, con mundos y ajustes independientes.',
    minecraftVersion: '1.21.1',
    enabled: false,
    availability: 'disabled',
  },
  { ...defaults, id: '7fd3f5a0-876d-4ca6-98a2-c4e4bedbbec2', name: 'Modpack de ejemplo', description: 'Vista previa de un tipo de instancia aún no compatible.', minecraftVersion: '1.21.1', enabled: true, supported: false },
];

export async function loadPreviewCatalog(): Promise<CatalogOverview> {
  return { instances: import.meta.env.DEV ? previewInstances.map(instance => ({ ...instance })) : [], stale: false, cacheWarning: null, registryWarning: null };
}
