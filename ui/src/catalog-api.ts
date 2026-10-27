import { invoke, isTauri } from '@tauri-apps/api/core';
import { loadPreviewCatalog, type CatalogOverview } from './preview-catalog';

export async function loadCatalog(): Promise<CatalogOverview> {
  if (isTauri()) {
    await invoke<void>('initialize');
    return invoke<CatalogOverview>('catalog_snapshot');
  }
  return loadPreviewCatalog();
}
