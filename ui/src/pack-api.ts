import { invoke } from '@tauri-apps/api/core';
import type { CacheReport, PackList } from './pack-state';

// Every call names an entry only by its manifest path; the backend owns all file paths.
export const packApi = {
  files: (instanceId: string) => invoke<PackList>('pack_files', { instanceId }),
  verify: (instanceId: string) => invoke<PackList>('verify_pack_files', { instanceId }),
  openSource: (instanceId: string, path: string) => invoke<boolean>('open_pack_source', { instanceId, path }),
  openFolder: (instanceId: string, path: string) => invoke<boolean>('open_pack_folder', { instanceId, path }),
  importFile: (instanceId: string, path: string) => invoke<PackList>('import_pack_file', { instanceId, path }),
  cacheSize: () => invoke<number>('content_cache_size'),
  clearCache: () => invoke<CacheReport>('clear_content_cache'),
};
