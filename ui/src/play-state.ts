export type PlayPhase = 'preparing' | 'warning' | 'starting' | 'running' | 'stopping' | 'cancelling' | 'exited' | 'failed' | 'cancelled';
export type PlayEntry = {
  instanceId: string;
  requestId: string;
  phase: PlayPhase;
  confirmationId: string | null;
  error: string | null;
  exitCode: number | null;
  exitSuccess: boolean | null;
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
  return phase !== 'exited' && phase !== 'failed' && phase !== 'cancelled';
}
