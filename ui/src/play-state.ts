export type PlayPhase = 'preparing' | 'installing' | 'installed' | 'awaiting_files' | 'uninstalled' | 'warning' | 'starting' | 'running' | 'stopping' | 'cancelling' | 'exited' | 'failed' | 'cancelled';
export type PlayEntry = {
  instanceId: string;
  requestId: string;
  phase: PlayPhase;
  confirmationId: string | null;
  error: string | null;
  exitCode: number | null;
  exitSuccess: boolean | null;
  exitExpected?: boolean;
  consoleError?: string | null;
  operation: 'install' | 'update' | 'repair' | 'uninstall' | 'play';
  backupRequested: boolean;
  backupOperation: string | null;
  receivedFiles: number;
  reusedFiles: number;
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
  return snapshot?.instances.filter(entry => entry.operation !== 'play' && !playActive(entry.phase))
    .map(entry => `${entry.instanceId}:${entry.requestId}:${entry.phase}`).sort().join('|') ?? '';
}
