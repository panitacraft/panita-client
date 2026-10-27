import type { GameLogSnapshot } from './game-output-state';

export type LogMetadata = { timestamp: string | null; level: string | null; thread: string | null; logger: string | null };
export type FormattedLogLine = GameLogSnapshot['lines'][number] & { metadata: LogMetadata | null };

function xmlText(text: string): string {
  return text.replace(/&(?:amp|lt|gt|quot|apos|#x[0-9a-f]+|#[0-9]+);/gi, entity => {
    const named: Record<string, string> = { '&amp;': '&', '&lt;': '<', '&gt;': '>', '&quot;': '"', '&apos;': "'" };
    if (named[entity]) return named[entity];
    const code = entity[2]?.toLowerCase() === 'x' ? parseInt(entity.slice(3, -1), 16) : Number(entity.slice(2, -1));
    return Number.isInteger(code) && code >= 32 && code <= 0x10ffff && !(code >= 0xd800 && code <= 0xdfff)
      ? String.fromCodePoint(code) : entity;
  });
}

export function logMetadata(header: string | null | undefined): LogMetadata | null {
  if (!header || !/^<log4j:Event(?:\s|>)/.test(header)) return null;
  const attributes: Record<string, string> = {};
  for (const match of header.matchAll(/([a-zA-Z][\w:.-]*)\s*=\s*(?:"([^"]*)"|'([^']*)')/g))
    attributes[match[1]] = xmlText(match[2] ?? match[3]);
  return { timestamp: attributes.timestamp ?? null, level: attributes.level?.toUpperCase() ?? null,
    thread: attributes.thread ?? null, logger: attributes.logger ?? null };
}

export function displayLogTime(timestamp: string | null): string {
  if (timestamp === null || !/^\d{1,16}$/.test(timestamp)) return '';
  const date = new Date(Number(timestamp));
  if (!Number.isFinite(date.getTime())) return '';
  return [date.getHours(), date.getMinutes(), date.getSeconds()].map(value => String(value).padStart(2, '0')).join(':')
    + '.' + String(date.getMilliseconds()).padStart(3, '0');
}

export function logPrefix(metadata: LogMetadata | null): string {
  if (!metadata) return '';
  return [displayLogTime(metadata.timestamp), [metadata.thread, metadata.level].filter(Boolean).join('/'), metadata.logger]
    .filter(Boolean).map(value => '[' + value + ']').join(' ');
}

/** Parse known wrappers only. CDATA/unknown markup stays inert text, never HTML. */
export function formattedGameLines(lines: GameLogSnapshot['lines']): FormattedLogLine[] {
  const states: Record<string, { header: string | null; cdata: boolean }> = {};
  return lines.flatMap(line => {
    if (line.stream === 'notice') return [{ ...line, metadata: null }];
    const state = states[line.stream] ??= { header: null, cdata: false };
    if (line.context !== undefined) state.header = line.context;
    if (line.cdata !== undefined) state.cdata = line.cdata;
    let metadata = logMetadata(state.header);
    let first: LogMetadata | null | undefined;
    const parts: string[] = [];
    const structured = state.header !== null || state.cdata || /^\s*<\/?log4j:/.test(line.text);
    if (!structured) return line.text.trim() ? [{ ...line, metadata: null }] : [];
    const append = (text: string, literal: boolean) => {
      if (!text) return;
      if (first === undefined && text.trim()) first = metadata;
      parts.push(literal ? text : xmlText(text));
    };
    let offset = 0;
    while (offset < line.text.length) {
      const rest = line.text.slice(offset);
      if (state.cdata) {
        const end = rest.indexOf(']]>');
        if (end < 0) { append(rest, true); break; }
        append(rest.slice(0, end), true);
        state.cdata = false; offset += end + 3; continue;
      }
      if (rest.startsWith('<![CDATA[')) { state.cdata = true; offset += 9; continue; }
      const tag = /^<\/?log4j:(Event|Message|Throwable)(?=[\s/>])(?:[^>"']|"[^"]*"|'[^']*')*>/.exec(rest)?.[0];
      if (tag) {
        if (/^<log4j:Event(?:\s|>)/.test(tag)) {
          state.header = tag; metadata = logMetadata(tag);
          if (parts.some(part => part.trim())) parts.push('\n' + logPrefix(metadata) + ' ');
        } else if (/^<\/log4j:Event\s*>$/.test(tag)) {
          state.header = null; metadata = null;
        } else if (/^<log4j:(?:Message|Throwable)/.test(tag) && parts.some(part => part.trim())) parts.push('\n');
        offset += tag.length; continue;
      }
      const next = rest.indexOf('<', 1);
      const end = next < 0 ? rest.length : next;
      append(rest.slice(0, end), false); offset += end;
    }
    const text = parts.join('');
    return text.trim() ? [{ ...line, text, metadata: first ?? null }] : [];
  });
}

export function readableGameLines(lines: GameLogSnapshot['lines']): GameLogSnapshot['lines'] {
  return formattedGameLines(lines).map(line => {
    const prefix = [line.metadata?.level, line.metadata?.thread].filter(Boolean).join('/');
    return { stream: line.stream, text: prefix ? '[' + prefix + '] ' + line.text : line.text };
  });
}
