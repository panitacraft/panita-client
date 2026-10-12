export type LauncherErrorCode =
  | 'invalid_data'
  | 'permission_denied'
  | 'storage_unavailable'
  | 'storage_commit_uncertain';

const messages: Record<LauncherErrorCode, string> = {
  invalid_data: 'No pudimos leer los ajustes guardados. Tus datos no se han reemplazado.',
  permission_denied: 'No pudimos acceder a los datos locales. Revisa los permisos e inténtalo de nuevo.',
  storage_unavailable: 'Los datos locales no están disponibles. Inténtalo de nuevo.',
  storage_commit_uncertain: 'No pudimos confirmar si se guardaron los datos. Reinicia el cliente y compruébalo.',
};

export function messageForError(error: unknown): string {
  if (typeof error === 'string' && Object.hasOwn(messages, error)) {
    return messages[error as LauncherErrorCode];
  }
  return 'No pudimos cargar las instancias. Inténtalo de nuevo.';
}
