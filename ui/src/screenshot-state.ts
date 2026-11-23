// Pure helpers for the screenshot gallery of an instance's settings.
export type Screenshot = { name: string; size: number; modifiedMs: number };

/** Newest first, as the backend lists them. */
export type ScreenshotList = { screenshots: Screenshot[]; truncated: boolean };

export type ScreenshotAction = 'open' | 'reveal' | 'copy';

/** Full-size images are fetched per card, so one page stays small. */
export const SCREENSHOTS_PER_PAGE = 12;

export function screenshotPageCount(total: number): number {
  return Math.max(1, Math.ceil(total / SCREENSHOTS_PER_PAGE));
}

/** Keep a remembered page valid after the list shrinks. */
export function clampScreenshotPage(page: number, total: number): number {
  return Math.min(Math.max(0, page), screenshotPageCount(total) - 1);
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

export const screenshotCopied = 'Imagen copiada al portapapeles.';

const failures: Record<ScreenshotAction, string> = {
  open: 'No pudimos abrir la captura con el visor de imágenes de tu sistema.',
  reveal: 'No pudimos mostrar la captura en el explorador de archivos.',
  copy: 'No pudimos copiar la imagen al portapapeles. Inténtalo de nuevo.',
};

/** A rejected name means the file changed or vanished since the list was read. */
export function screenshotActionError(action: ScreenshotAction, error: unknown): string {
  return error === 'invalid_data' ? 'Esa captura ya no está disponible o no se puede usar. Actualizamos la lista.' : failures[action];
}
