export type PlayPhase = 'preparing' | 'installing' | 'installed' | 'awaiting_files' | 'uninstalled' | 'warning' | 'starting' | 'running' | 'stopping' | 'cancelling' | 'exited' | 'failed' | 'cancelled';
/**
 * What the core knows about a failure beyond its code. Every field is a fixed token or
 * a game-relative path, never a message from the operating system or a service.
 */
export type PlayErrorDetail = {
  /** For example `published_state_invalid`, `archive_invalid`, `checksum_mismatch`, `file_in_use`. */
  cause: string | null;
  /** Game-directory relative file the failure concerns, `/`-separated. */
  path: string | null;
  /** Raw operating-system error number, such as 32 for a Windows sharing violation. */
  osError: number | null;
};
export type PlayEntry = {
  instanceId: string;
  requestId: string;
  phase: PlayPhase;
  confirmationId: string | null;
  error: string | null;
  /** Present only while `error` is set and the core could say more about it. */
  errorDetail?: PlayErrorDetail | null;
  exitCode: number | null;
  exitSuccess: boolean | null;
  exitExpected?: boolean;
  consoleError?: string | null;
  operation: 'install' | 'update' | 'repair' | 'uninstall' | 'play';
  backupRequested: boolean;
  eraseRequested?: boolean;
  backupOperation: string | null;
  receivedFiles: number;
  reusedFiles: number;
  /** Latest reported artifact transfer, never a whole-install byte total. */
  artifactProgress?: { bytes: number; total: number } | null;
  retainedFiles: number;
  nativeCleanup?: import('./native-cleanup-state').NativeCleanupReport | null;
  packProgress?: import('./pack-state').PackProgress | null;
  packAwaiting?: import('./pack-state').PackFile[];
  packSkipped?: string[] | null;
  packConflicts?: import('./pack-state').PackConflict[] | null;
  packBackedUp?: string[] | null;
  packReleased?: string[] | null;
};
export type PlaySnapshot = {
  revision: number;
  processSessionId: string;
  closeBlocked: boolean;
  gracefulStopSupported: boolean;
  instances: PlayEntry[];
};

export function acceptPlaySnapshot(previous: PlaySnapshot | null, next: PlaySnapshot): PlaySnapshot {
  if (previous?.processSessionId === next.processSessionId && previous.revision > next.revision) return previous;
  return next;
}

export function playActive(phase: PlayPhase): boolean {
  return phase !== 'installed' && phase !== 'awaiting_files' && phase !== 'uninstalled' && phase !== 'exited' && phase !== 'failed' && phase !== 'cancelled';
}

export function installationRefreshKey(snapshot: PlaySnapshot | null): string {
  return snapshot?.instances.filter(entry => (entry.operation !== 'play' || (entry.error === 'update_required' || entry.error === 'variant_unavailable')) && !playActive(entry.phase))
    .map(entry => `${entry.instanceId}:${entry.requestId}:${entry.phase}`).sort().join('|') ?? '';
}
