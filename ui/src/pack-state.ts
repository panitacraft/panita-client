export type PackFile = {
  path: string;
  name: string;
  fileName: string;
  folder: string;
  projectId: number;
  fileId: number;
  size: number;
  reason: 'manual' | 'withdrawn';
};
export type PackList = { files: PackFile[]; complete: boolean; cancelled?: boolean };
export type PackProgress = { filesDone: number; filesTotal: number; bytesDone: number; bytesTotal: number; reusedFiles: number };
export type CacheDetail = { directory: string; reason: string };
export type CacheReport = { removedEntries: number; removedBytes: number; retainedEntries: number; details: CacheDetail[] };

const number = new Intl.NumberFormat('es', { maximumFractionDigits: 1 });

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '0 B';
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${number.format(bytes / 1024)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${number.format(bytes / (1024 * 1024))} MB`;
  return `${number.format(bytes / (1024 * 1024 * 1024))} GB`;
}

export function packProgressFraction(progress: PackProgress | null | undefined): number {
  if (!progress || progress.bytesTotal <= 0) return 0;
  return Math.min(1, Math.max(0, progress.bytesDone / progress.bytesTotal));
}

export function packProgressText(progress: PackProgress | null | undefined): string {
  if (!progress) return 'Preparando los archivos del modpack…';
  const reused = progress.reusedFiles > 0 ? ` ${progress.reusedFiles} ya estaban guardados.` : '';
  return `${progress.filesDone} de ${progress.filesTotal} archivos (${formatBytes(progress.bytesDone)} de ${formatBytes(progress.bytesTotal)}).${reused}`;
}

export function manualHeading(count: number): string {
  return count === 1 ? 'Falta 1 archivo que debes descargar tú mismo' : `Faltan ${count} archivos que debes descargar tú mismo`;
}

export function reasonText(reason: PackFile['reason']): string {
  return reason === 'withdrawn'
    ? 'Su autor retiró la descarga automática.'
    : 'Su autor no permite que otras aplicaciones lo descarguen.';
}

export function folderHint(file: PackFile): string {
  return `Descárgalo desde CurseForge (archivo ${file.fileId}) y guárdalo en la carpeta ${file.folder} de esta instancia.`;
}

/** Text for the two errors a manual import adds to the general table; null otherwise. */
export function importErrorText(cause: unknown): string | null {
  if (cause === 'artifact_mismatch') return 'El archivo no es la versión esperada.';
  if (cause === 'pack_conflict') return 'Ya hay otro archivo en esa ubicación. No lo reemplazamos: bórralo o muévelo y vuelve a intentarlo.';
  return null;
}

export function skippedSummary(paths: string[]): string {
  return paths.length === 1
    ? 'Conservamos 1 archivo de configuración que ya existía, sin cambiarlo.'
    : `Conservamos ${paths.length} archivos de configuración que ya existían, sin cambiarlos.`;
}

export function conflictSummary(paths: string[]): string {
  return paths.length === 1
    ? 'Hay 1 archivo tuyo donde este modpack necesita instalar el suyo. No cambiamos nada.'
    : `Hay ${paths.length} archivos tuyos donde este modpack necesita instalar los suyos. No cambiamos nada.`;
}

export function cacheSummary(bytes: number | null): string {
  if (bytes === null) return 'Calculando el tamaño…';
  return bytes === 0 ? 'La caché de archivos de modpacks está vacía.' : `La caché de archivos de modpacks ocupa ${formatBytes(bytes)}.`;
}

const retained: Record<string, string> = {
  inUse: 'Se está usando en este momento.',
  unsafePath: 'Es un enlace o un archivo no esperado, así que no lo tocamos.',
  accessFailure: 'El sistema no permitió eliminarlo.',
  unknown: 'No reconocemos esta carpeta, así que la conservamos.',
};

export function cacheRetainedReason(reason: string): string {
  return Object.hasOwn(retained, reason) ? retained[reason] : 'No pudimos eliminarlo.';
}

export function clearSummary(report: CacheReport): string {
  const removed = report.removedEntries === 1 ? 'Eliminamos 1 archivo' : `Eliminamos ${report.removedEntries} archivos`;
  const kept = report.retainedEntries > 0
    ? ` Conservamos ${report.retainedEntries} que no pudimos eliminar.`
    : '';
  return `${removed} (${formatBytes(report.removedBytes)}). Tus instancias y mundos no cambian; los archivos se descargarán de nuevo cuando hagan falta.${kept}`;
}
