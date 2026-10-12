import { invoke, isTauri } from '@tauri-apps/api/core';
import { loadPreviewCatalog, type CatalogInstance } from './preview-catalog';

export async function loadCatalog(): Promise<CatalogInstance[]> {
  if (isTauri()) {
    await invoke<void>('initialize');
    return invoke<CatalogInstance[]>('catalog_snapshot');
  }
  return loadPreviewCatalog();
}
