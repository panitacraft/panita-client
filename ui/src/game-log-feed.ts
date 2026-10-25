import type { GameLogSnapshot } from './game-output-state';

export type GameLogNotice = {
  instanceId: string; requestId: string; total: number; running: boolean; exitCode: number | null;
};

export function applyLogNotice(page: GameLogSnapshot, notice: GameLogNotice | null): GameLogSnapshot {
  if (!notice) return page;
  return { ...page, total: Math.max(page.total, notice.total), running: page.running && notice.running,
    exitCode: notice.running ? page.exitCode : notice.exitCode ?? page.exitCode };
}

/** Keep rendering bounded while consuming every new cursor in order. */
export function mergeLogPage(previous: GameLogSnapshot | null, next: GameLogSnapshot): GameLogSnapshot {
  if (!previous) return next;
  if (next.offset !== previous.nextOffset) throw new Error('Invalid console cursor');
  const offset = Math.max(previous.offset, next.nextOffset - next.pageSize);
  return { ...next, offset, running: previous.running && next.running,
    exitCode: next.exitCode ?? previous.exitCode,
    lines: [...previous.lines, ...next.lines].filter(line => (line.sequence ?? 0) >= offset) };
}

type FeedOptions = {
  offset: number | null; follow: boolean;
  load: (offset: number | null) => Promise<GameLogSnapshot>;
  subscribe: (changed: (notice: GameLogNotice) => void) => Promise<() => void>;
  receive: (page: GameLogSnapshot) => void;
  fail: (error: unknown) => void;
};

/** Subscribe before reading; coalesced notices never replace the read cursor. */
export function startGameLogFeed(options: FeedOptions): () => void {
  let active = true;
  let unlisten: (() => void) | undefined;
  let reading = false;
  let dirty = false;
  let page: GameLogSnapshot | null = null;
  let notice: GameLogNotice | null = null;
  async function refresh() {
    dirty = true;
    if (reading || !active) return;
    reading = true;
    try {
      while (active && dirty) {
        dirty = false;
        const cursor = page && options.follow ? page.nextOffset : options.offset;
        const next = await options.load(cursor);
        if (!active) return;
        if (next.nextOffset < next.offset || next.nextOffset > next.total
          || (cursor !== null && next.offset !== cursor)) throw new Error('Invalid console page');
        page = applyLogNotice(options.follow ? mergeLogPage(page, next) : next, notice);
        options.receive(page);
        if (options.follow && page.nextOffset < page.total) {
          if (next.nextOffset === next.offset) throw new Error('Console cursor did not advance');
          dirty = true;
          await new Promise(resolve => setTimeout(resolve, 0));
        }
      }
    } catch (error) { if (active) options.fail(error); }
    finally { reading = false; }
  }
  void options.subscribe(next => {
    if (!active) return;
    if (!Number.isSafeInteger(next.total) || next.total < 0 || typeof next.running !== 'boolean') return;
    notice = next;
    if (options.follow) void refresh();
    else if (page) { page = applyLogNotice(page, notice); options.receive(page); }
  }).then(stop => {
    if (!active) { stop(); return; }
    unlisten = stop;
    void refresh();
  }).catch(error => { if (active) options.fail(error); });
  return () => { active = false; unlisten?.(); };
}
