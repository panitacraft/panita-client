export type NativeCleanupReport = {
  cleanedGenerations: number;
  retainedGenerations: number;
  untrackedGenerations: number;
  details: { directory: string; reason: string }[];
};

const reasons: Record<string, string> = {
  active: 'En uso por la instalación actual.',
  accessFailure: 'No se pudo eliminar. Puede haber archivos en uso o permisos que impidan el acceso. Cierra Minecraft y vuelve a intentarlo.',
  unsafePath: 'Se conservó una ruta redirigida o una estructura no segura. La limpieza no sigue enlaces a otras carpetas.',
};

export function nativeRetentionReason(reason: string): string {
  return Object.hasOwn(reasons, reason) ? reasons[reason] : 'No pudimos comprobar que sea seguro eliminar esta carpeta.';
}

export function nativeCleanupSummary(report: NativeCleanupReport): string {
  return 'Carpetas nativas eliminadas: ' + report.cleanedGenerations + '.'
    + (report.retainedGenerations > 0 ? ' No se pudieron eliminar ' + report.retainedGenerations + '; la limpieza está pendiente.' : ' Limpieza completada.');
}
