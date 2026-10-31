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

export type UpdateNotice = { tone: 'status' | 'alert'; text: string; action: UpdateAction | null; actionLabel: string | null };

/** A newer release is known, so new game launches are blocked until it is applied. */
export function blocksPlay(view: LauncherUpdateView | null): boolean {
  return view?.pendingVersion != null;
}

/** `describeError` turns a backend error code into player-facing Spanish text. */
export function updateNotice(view: LauncherUpdateView | null, describeError: (code: string | null) => string): UpdateNotice | null {
  if (!view) return null;
  const version = view.pendingVersion;
  const required = version !== null;
  const named = version ? `Panita Client ${version}` : 'Panita Client';
  const status = (text: string, action: UpdateAction | null = null, actionLabel: string | null = null): UpdateNotice => ({ tone: 'status', text, action, actionLabel });
  const alert = (text: string, action: UpdateAction, actionLabel: string): UpdateNotice => ({ tone: 'alert', text, action, actionLabel });
  switch (view.phase) {
    case 'checking':
      return required ? status(`Comprobando la actualización obligatoria ${named}…`) : null;
    case 'idle':
      return required ? alert(`Hay una actualización obligatoria: ${named}. Necesitas aplicarla antes de jugar.`, 'retry', 'Buscar actualización') : null;
    case 'downloading':
      return status(`Descargando ${named}${view.progress === null ? '' : ` (${view.progress} %)`}…`);
    case 'ready':
      return view.applyRequested
        ? status('La actualización se aplicará cuando terminen tus partidas y operaciones en curso.', 'cancel', 'Cancelar')
        : alert(`${named} está listo para instalarse. Es obligatorio para poder jugar.`, 'apply', 'Actualizar ahora');
    case 'applying':
      return status('Aplicando la actualización. Panita Client se reiniciará.');
    case 'manual':
      return alert(`Hay una versión nueva: ${named}. Esta instalación no puede actualizarse sola; instálala desde la página de la versión.`, 'release_page', 'Ver nueva versión');
    case 'failed':
      return required
        ? alert(`${describeError(view.error)} Es obligatorio actualizar para poder jugar.`, 'retry', 'Reintentar')
        : status('No pudimos comprobar si hay actualizaciones. Puedes seguir jugando.', 'retry', 'Reintentar');
  }
}

/** The control that replaces Play while an update is required. */
export function primaryUpdateControl(view: LauncherUpdateView | null): { label: string; action: UpdateAction | null } | null {
  if (!blocksPlay(view) || !view) return null;
  switch (view.phase) {
    case 'ready': return view.applyRequested ? { label: 'Actualización pendiente…', action: null } : { label: 'Actualizar Panita Client', action: 'apply' };
    case 'downloading': return { label: 'Descargando actualización…', action: null };
    case 'checking': return { label: 'Comprobando actualización…', action: null };
    case 'applying': return { label: 'Actualizando…', action: null };
    case 'manual': return { label: 'Ver nueva versión', action: 'release_page' };
    default: return { label: 'Buscar actualización', action: 'retry' };
  }
}

/** Switching channels is refused while a download or an application is in progress. */
export function channelLocked(view: LauncherUpdateView | null): boolean {
  return view === null || view.phase === 'downloading' || view.phase === 'applying';
}
