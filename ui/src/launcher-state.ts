import type { CatalogInstance } from './preview-catalog';
import type { PlayEntry } from './play-state';

export type LauncherPage = 'library' | 'accounts' | 'settings';

/** A cached selection may name a catalog entry no longer visible to the member. */
export function visibleSelection(instances: CatalogInstance[], selected: string | null): string | null {
  return instances.find(instance => instance.id === selected)?.id ?? instances[0]?.id ?? null;
}

export const localStateText: Record<CatalogInstance['localState'], string> = {
  not_installed: 'Sin instalar', installing: 'Instalando', installed: 'Lista para jugar',
  awaiting_files: 'Faltan archivos', update_available: 'Actualización disponible',
  needs_repair: 'Necesita reparación', running: 'En ejecución', failed: 'La operación falló', cancelled: 'Operación cancelada',
};

/** Live operations take precedence over the last catalog snapshot in navigation. */
export function instanceStatus(instance: CatalogInstance, entry?: PlayEntry): string {
  if (instance.availability === 'disabled') return 'Deshabilitada';
  if (entry) {
    switch (entry.phase) {
      case 'preparing': case 'starting': return 'Preparando inicio';
      case 'installing': return entry.operation === 'update' ? 'Actualizando' : entry.operation === 'repair' ? 'Reparando' : entry.operation === 'uninstall' ? 'Desinstalando' : 'Instalando';
      case 'warning': return 'Confirmación pendiente';
      case 'running': return 'En ejecución';
      case 'stopping': return 'Cerrando Minecraft';
      case 'cancelling': return 'Cancelando';
    }
  }
  return localStateText[instance.localState];
}

export function safeArtwork(url: string | null | undefined): string | null {
  if (!url || url.length > 8192) return null;
  try {
    const parsed = new URL(url);
    return parsed.protocol === 'https:' && !parsed.username && !parsed.password ? parsed.href : null;
  } catch { return null; }
}
