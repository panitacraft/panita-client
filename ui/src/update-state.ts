export type UpdateChannel = 'stable' | 'beta';
export type UpdatePhase = 'idle' | 'checking' | 'downloading' | 'ready' | 'applying' | 'manual' | 'failed';
export type UpdateAction = 'apply' | 'cancel' | 'retry' | 'release_page';

/** The backend's launcher update view. The pending version is the durable requirement. */
export type LauncherUpdateView = {
  currentVersion: string;
  channel: UpdateChannel;
  phase: UpdatePhase;
  pendingVersion: string | null;
  delivery: 'in_place' | 'manual';
  releaseUrl: string | null;
  progress: number | null;
  applyRequested: boolean;
  error: string | null;
};

/** A newer release is known, so new game launches are blocked until it is applied. */
export function blocksPlay(view: LauncherUpdateView | null): boolean {
  return view?.pendingVersion != null;
}

/** Switching channels is refused while a download or an application is in progress. */
export function channelLocked(view: LauncherUpdateView | null): boolean {
  return view === null || view.phase === 'downloading' || view.phase === 'applying';
}

export type BadgeIcon = 'update' | 'retry' | 'external' | 'busy';
/**
 * What the sidebar version element shows. `version` is the plain label, `retry` is the
 * version with a red retry icon (the check failed and nothing is required), and `update`
 * is a button that takes the box of the label.
 */
export type BadgeModel =
  | { kind: 'version' }
  | { kind: 'retry'; tooltip: string }
  | { kind: 'update'; icon: BadgeIcon; label: string; action: UpdateAction | null; tooltip: string; error: boolean };

/** `describeError` turns a backend error code into player-facing Spanish text. */
export function badgeModel(view: LauncherUpdateView | null, describeError: (code: string | null) => string): BadgeModel {
  const pending = view?.pendingVersion ?? null;
  if (!view || pending === null) {
    return view?.phase === 'failed'
      ? { kind: 'retry', tooltip: 'No se pudieron buscar actualizaciones. Haz clic para reintentar.' }
      : { kind: 'version' };
  }
  const named = `Panita Client ${pending}`;
  const update = (icon: BadgeIcon, label: string, action: UpdateAction | null, tooltip: string, error = false): BadgeModel =>
    ({ kind: 'update', icon, label, action, tooltip, error });
  switch (view.phase) {
    case 'ready':
      return view.applyRequested
        ? update('busy', 'Pendiente', 'cancel', 'Se aplicará al terminar tus partidas y operaciones en curso. Haz clic para cancelar.')
        : update('update', 'Actualizar', 'apply', `Actualizar a ${named}. Es obligatorio para poder jugar.`);
    case 'downloading':
      return update('busy', view.progress === null ? 'Bajando' : `${view.progress} %`, null, `Descargando ${named}…`);
    case 'checking':
      return update('busy', 'Buscando', null, `Comprobando la actualización obligatoria ${named}…`);
    case 'applying':
      return update('busy', 'Aplicando', null, 'Aplicando la actualización. Panita Client se reiniciará.');
    case 'manual':
      return update('external', 'Ver nueva', 'release_page', `Hay una versión nueva: ${named}. Esta instalación no puede actualizarse sola; instálala desde la página de la versión.`);
    case 'failed':
      return update('retry', 'Reintentar', 'retry', `${describeError(view.error)} La actualización es obligatoria para poder jugar. Haz clic para reintentar.`, true);
    case 'idle':
      return update('update', 'Actualizar', 'retry', `Hay una actualización obligatoria: ${named}. Haz clic para descargarla.`);
  }
}
