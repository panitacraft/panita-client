export type LauncherErrorCode =
  | 'invalid_data'
  | 'permission_denied'
  | 'storage_unavailable'
  | 'storage_commit_uncertain'
  | 'network_unavailable' | 'authentication_required' | 'account_not_found'
  | 'account_mismatch' | 'invalid_username' | 'authentication_denied'
  | 'authentication_timeout' | 'service_unavailable' | 'account_restricted'
  | 'minecraft_not_owned' | 'operation_busy' | 'operation_cancelled'
  | 'callback_registration_failed' | 'browser_unavailable' | 'authentication_configuration'
  | 'accounts_file_invalid' | 'callback_not_active' | 'unsafe_permissions'
  | 'instance_disabled' | 'instance_missing' | 'unsupported_instance'
  | 'artifact_mismatch' | 'runtime_unsupported';
// Backend error categories are translated here; raw service payloads are excluded.

const messages: Record<LauncherErrorCode, string> = {
  artifact_mismatch: 'Un archivo descargado no coincide con el original. Inténtalo de nuevo.',
  runtime_unsupported: 'No hay un Java compatible disponible para esta versión y plataforma.',
  instance_disabled: 'Esta instancia está deshabilitada. Conservamos tus datos locales.',
  instance_missing: 'Esta instancia ya no está disponible en el catálogo. Conservamos tus datos locales.',
  unsupported_instance: 'Esta instancia requiere otra versión o funciones que todavía no están disponibles.',
  invalid_data: 'No pudimos validar los datos recibidos o guardados. Inténtalo de nuevo.',
  permission_denied: 'No pudimos acceder a los datos locales. Revisa los permisos e inténtalo de nuevo.',
  storage_unavailable: 'Los datos locales no están disponibles. Inténtalo de nuevo.',
  storage_commit_uncertain: 'No pudimos confirmar si se guardaron los datos. Reinicia el cliente y compruébalo.',
  network_unavailable: 'No pudimos conectar con el servicio. Revisa tu conexión e inténtalo de nuevo.',
  authentication_required: 'Microsoft no autorizó la sesión. Vuelve a iniciar sesión para continuar.',
  account_not_found: 'La cuenta ya no está disponible. Selecciona otra cuenta.',
  account_mismatch: 'La cuenta autorizada no corresponde al perfil esperado. Inicia sesión con la cuenta correcta.',
  invalid_username: 'Usa un nombre de 3 a 16 caracteres: letras, números o guion bajo.',
  authentication_denied: 'No se autorizó el inicio de sesión. Puedes intentarlo de nuevo.',
  authentication_configuration: 'Microsoft rechazó la configuración de inicio de sesión del cliente. Contacta con el responsable de Panita Client.',
  authentication_timeout: 'El inicio de sesión tardó demasiado. Inténtalo de nuevo.',
  service_unavailable: 'El servicio no está disponible por ahora. Inténtalo más tarde.',
  account_restricted: 'El servicio rechazó el acceso de esta cuenta. Revisa los permisos y la configuración de Xbox.',
  minecraft_not_owned: 'Esta cuenta no tiene un perfil de Minecraft Java disponible. Comprueba tu licencia y crea el perfil.',
  operation_busy: 'Hay una operación de inicio de sesión en curso. Espera o cancélala.',
  operation_cancelled: 'La operación se canceló. Puedes intentarlo de nuevo.',
  accounts_file_invalid: 'El archivo de cuentas está dañado o pertenece a una versión no compatible. No lo modificamos para proteger tus datos. Cierra el cliente y restaura una copia de accounts.json en la carpeta PanitaClient de tus datos de aplicación, o elimínalo para empezar de nuevo.',
  unsafe_permissions: 'La carpeta de datos de Panita Client o uno de sus archivos tiene permisos que otros usuarios podrían leer, así que no lo usamos. Restaura los permisos para que solo tu usuario tenga acceso y vuelve a intentarlo.',
  callback_not_active: 'Se ignoró un regreso del navegador que ya no estaba activo. Si querías iniciar sesión, vuelve a intentarlo.',
  callback_registration_failed: 'No pudimos configurar el regreso desde el navegador. Reinicia el cliente e inténtalo de nuevo.',
  browser_unavailable: 'No pudimos abrir el navegador del sistema. Revisa tu navegador predeterminado e inténtalo de nuevo.',
};

export function messageForError(error: unknown): string {
  if (typeof error === 'string' && Object.hasOwn(messages, error)) {
    return messages[error as LauncherErrorCode];
  }
  return 'No pudimos cargar las instancias. Inténtalo de nuevo.';
}
