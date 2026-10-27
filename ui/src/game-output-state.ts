import type { PlaySnapshot } from './play-state';

export type GameLogSnapshot = {
  lines: { stream: 'stdout' | 'stderr' | 'notice'; text: string; sequence?: number; context?: string | null; cdata?: boolean }[];
  running: boolean; exitCode: number | null;
  offset: number; nextOffset: number; total: number; pageSize: number;
};

export function previousLogPage(snapshot: GameLogSnapshot): number {
  return Math.max(0, snapshot.offset - snapshot.pageSize);
}

export function logWindowIdentity(label: string): { instanceId: string; requestId: string } | null {
  const uuid = '[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}';
  const match = new RegExp(`^game-log-(${uuid})-(${uuid})$`).exec(label);
  return match ? { instanceId: match[1], requestId: match[2] } : null;
}

export function unacknowledgedCrashes(snapshot: PlaySnapshot | null, dismissed: Set<string>) {
  return snapshot?.instances.filter(entry => entry.operation === 'play' && entry.phase === 'exited' && entry.exitSuccess === false && !entry.exitExpected)
    .map(entry => ({ ...entry, key: `${snapshot.processSessionId}:${entry.requestId}` }))
    .filter(entry => !dismissed.has(entry.key)) ?? [];
}
