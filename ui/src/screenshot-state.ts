// Pure helpers for the screenshot gallery of an instance's settings.
export type Screenshot = { name: string; size: number; modifiedMs: number };

/** Newest first, as the backend lists them. */
export type ScreenshotList = { screenshots: Screenshot[]; truncated: boolean };

/** The actions the backend performs on one screenshot. */
export type ScreenshotAction = 'open' | 'reveal' | 'copy' | 'delete';

/** What the player can choose for a screenshot: the backend actions plus the in-app viewer. */
export type ScreenshotChoice = 'view' | ScreenshotAction;

/** The menu entries, in order, in simple Spanish. Deleting only moves the file to the trash. */
export const screenshotChoices: readonly { choice: ScreenshotChoice; label: string }[] = [
  { choice: 'view', label: 'Ver' },
  { choice: 'open', label: 'Abrir en fotos' },
  { choice: 'reveal', label: 'Mostrar en la carpeta' },
  { choice: 'copy', label: 'Copiar imagen' },
  { choice: 'delete', label: 'Eliminar' },
];

/** Full-size images are fetched per card, so one page stays small. */
export const SCREENSHOTS_PER_PAGE = 12;

/** How often the open gallery looks for new or removed screenshots. */
export const SCREENSHOT_REFRESH_MS = 3000;

/** Whether a fresh listing says nothing new, so the screen can keep what it already shows. */
export function sameScreenshotList(previous: ScreenshotList | null, next: ScreenshotList): boolean {
  return previous !== null && previous.truncated === next.truncated && previous.screenshots.length === next.screenshots.length
    && previous.screenshots.every((item, index) => {
      const other = next.screenshots[index];
      return item.name === other.name && item.size === other.size && item.modifiedMs === other.modifiedMs;
    });
}

export function screenshotPageCount(total: number): number {
  return Math.max(1, Math.ceil(total / SCREENSHOTS_PER_PAGE));
}

/** Keep a remembered page valid after the list shrinks. */
export function clampScreenshotPage(page: number, total: number): number {
  return Math.min(Math.max(0, page), screenshotPageCount(total) - 1);
}

export function screenshotPageOf(index: number): number {
  return Math.max(0, Math.floor(index / SCREENSHOTS_PER_PAGE));
}

/** The neighbour in the viewer; the ends do not wrap around. */
export function adjacentScreenshot(index: number, step: -1 | 1, length: number): number {
  return Math.min(Math.max(0, index + step), Math.max(0, length - 1));
}

/** Where the viewer goes after a removal, given how many remain: the next one, else the previous. */
export function indexAfterRemoval(index: number, remaining: number): number | null {
  return remaining <= 0 ? null : Math.min(index, remaining - 1);
}

export function screenshotPage(list: readonly Screenshot[], page: number): Screenshot[] {
  const start = clampScreenshotPage(page, list.length) * SCREENSHOTS_PER_PAGE;
  return list.slice(start, start + SCREENSHOTS_PER_PAGE);
}

export function screenshotSizeText(bytes: number): string {
  const format = (value: number) => new Intl.NumberFormat('es', { maximumFractionDigits: 1 }).format(value);
  return bytes >= 1024 * 1024 ? `${format(bytes / (1024 * 1024))} MB` : `${format(Math.max(1, bytes / 1024))} KB`;
}

/** The capture time, or an empty text when the file system reported none. */
export function screenshotDateText(modifiedMs: number): string {
  if (!Number.isFinite(modifiedMs) || modifiedMs <= 0) return '';
  return new Intl.DateTimeFormat('es', { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(modifiedMs));
}

export type ScreenshotMonth = { key: string; label: string; items: Screenshot[] };

/**
 * Split a newest-first list into runs that share a month, each with a Spanish heading such
 * as "Noviembre 2026". A file without a date goes under "Sin fecha". Order is kept.
 */
export function groupScreenshotsByMonth(items: readonly Screenshot[]): ScreenshotMonth[] {
  const months: ScreenshotMonth[] = [];
  for (const item of items) {
    const dated = Number.isFinite(item.modifiedMs) && item.modifiedMs > 0;
    const date = new Date(item.modifiedMs);
    const key = dated ? `${date.getFullYear()}-${date.getMonth()}` : 'unknown';
    const last = months[months.length - 1];
    if (last?.key === key) { last.items.push(item); continue; }
    const name = dated ? new Intl.DateTimeFormat('es', { month: 'long' }).format(date) : '';
    months.push({ key, label: dated ? `${name.charAt(0).toUpperCase()}${name.slice(1)} ${date.getFullYear()}` : 'Sin fecha', items: [item] });
  }
  return months;
}

export const screenshotCopied = 'Imagen copiada al portapapeles.';
export const screenshotCopying = 'Copiando imagen…';
export const screenshotTrashed = 'Captura movida a la papelera.';

const failures: Record<ScreenshotAction, string> = {
  open: 'No pudimos abrir la captura con el visor de imágenes de tu sistema.',
  reveal: 'No pudimos mostrar la captura en el explorador de archivos.',
  copy: 'No pudimos copiar la imagen al portapapeles. Inténtalo de nuevo.',
  delete: 'No pudimos mover la captura a la papelera. Puede estar en uso por otro programa.',
};

/** A rejected name means the file changed or vanished since the list was read. */
export function screenshotActionError(action: ScreenshotAction, error: unknown): string {
  return error === 'invalid_data' ? 'Esa captura ya no está disponible o no se puede usar. Actualizamos la lista.' : failures[action];
}
