import type { PlaySnapshot } from './play-state';

export type GameLogSnapshot = { lines: { stream: 'stdout' | 'stderr' | 'notice'; text: string }[]; running: boolean; exitCode: number | null };

/** Render Mojang's console XML as plain text, without evaluating markup. */
export function readableGameLines(lines: GameLogSnapshot['lines']): GameLogSnapshot['lines'] {
  let prefix = '';
  return lines.slice(-256).flatMap(line => {
    if (line.stream !== 'stdout') return [line];
    if (/^\s*<log4j:Event\b/.test(line.text)) {
      const level = /\blevel="([^"]*)"/.exec(line.text)?.[1];
      const thread = /\bthread="([^"]*)"/.exec(line.text)?.[1];
      prefix = [level, thread].filter(Boolean).join('/');
      return [];
    }
    if (/^\s*<\/log4j:Event>\s*$/.test(line.text)) { prefix = ''; return []; }
    const text = line.text.replace(/^\s*<log4j:(?:Message|Throwable)>(?:<!\[CDATA\[)?/, '')
      .replace(/(?:\]\]>)?<\/log4j:(?:Message|Throwable)>\s*$/, '');
    if (!text.trim()) return [];
    return [{ ...line, text: prefix ? `[${prefix}] ${text}` : text }];
  });
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
