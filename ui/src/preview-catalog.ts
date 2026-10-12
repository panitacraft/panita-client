export interface CatalogInstance {
  id: string;
  name: string;
  description: string;
  minecraftVersion: string;
  enabled: boolean;
}

const previewInstances: CatalogInstance[] = [
  {
    id: '62d7e99d-fb4e-4167-8732-ce3a334d4a58',
    name: 'Vanilla actual',
    description: 'La última versión estable de Minecraft, sin mods.',
    minecraftVersion: '26.3',
    enabled: true,
  },
  {
    id: '65ee13c6-bc34-4914-aa09-41205f3691a0',
    name: 'Vanilla clásica',
    description: 'Una versión anterior, con mundos y ajustes independientes.',
    minecraftVersion: '1.21.1',
    enabled: true,
  },
];

export async function loadPreviewCatalog(): Promise<CatalogInstance[]> {
  if (!import.meta.env.DEV) return [];
  return previewInstances.map(instance => ({ ...instance }));
}
