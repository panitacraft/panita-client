import { messageForError } from './errors';
import type { NativeCleanupReport } from './native-cleanup-state';
import { conflictHeading, manualHeading, tagVersion } from './pack-state';
import { playActive, type PlayEntry } from './play-state';
import type { CatalogInstance } from './preview-catalog';

/** The version an instance has installed, as its completion prompt shows it: a custom pack's label, or a vanilla Minecraft version. */
export function installedVersionLabel(instance: CatalogInstance): string | null {
  const source = instance.installedSource;
  if (source?.type === 'custom') {
    const label = source.label?.trim();
    return label ? tagVersion(label) : null;
  }
  return instance.installedVersion;
}

/**
 * Outcomes that are announced with a toast instead of a block in the page. A toast holds one
 * sentence; anything with a list behind it carries a "Ver detalles" button that opens the list.
 */
export type NoticeDetails = 'conflicts' | 'manual';
export type EntryNotice = { message: string; details: NoticeDetails | null };

/** The toast for a finished operation that stopped before changing anything, or null when it has none. */
export function entryNotice(entry: PlayEntry): EntryNotice | null {
  if (playActive(entry.phase)) return null;
  if (entry.error === 'update_required') return { message: messageForError('update_required'), details: null };
  const conflicts = entry.packConflicts?.length ?? 0;
  if (entry.error === 'pack_conflict' && conflicts > 0) return { message: `${conflictHeading(conflicts, entry.operation)}.`, details: 'conflicts' };
  const awaiting = entry.packAwaiting?.length ?? 0;
  if (entry.operation === 'update' && entry.error === 'manual_files_required' && awaiting > 0) return { message: `${manualHeading(awaiting)}.`, details: 'manual' };
  return null;
}

/** What a successful operation left for the player to know about, beyond "completed". */
export type CompletionDetails = { skipped: string[]; released: string[]; backedUp: string[]; natives: NativeCleanupReport | null };

/** Lists behind a completed operation, or null when it finished without anything to explain. */
export function completionDetails(entry: PlayEntry): CompletionDetails | null {
  if (entry.phase !== 'installed' && entry.phase !== 'uninstalled') return null;
  const details: CompletionDetails = {
    skipped: entry.packSkipped ?? [],
    released: entry.packReleased ?? [],
    backedUp: entry.packBackedUp ?? [],
    natives: entry.nativeCleanup && entry.nativeCleanup.retainedGenerations > 0 ? entry.nativeCleanup : null,
  };
  return details.skipped.length + details.released.length + details.backedUp.length > 0 || details.natives ? details : null;
}

/** One short sentence for the completion toast: what to look at, not the list itself. */
export function completionNote(details: CompletionDetails): string {
  const notes: string[] = [];
  if (details.skipped.length + details.released.length + details.backedUp.length > 0) notes.push('Algunos de tus archivos se conservaron.');
  if (details.natives) notes.push('Quedaron carpetas nativas antiguas sin eliminar.');
  return notes.join(' ');
}
