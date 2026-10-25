export type NativeCleanupReport = {
  cleanedGenerations: number;
  retainedGenerations: number;
  untrackedGenerations: number;
  details: { directory: string; reason: string }[];
};

const reasons: Record<string, string> = {
  active: 'En uso por la instalación actual.',
  missingOwnership: 'Sin registro de propiedad. No podemos determinar su origen de forma segura.',
  invalidOwnership: 'El registro de propiedad está dañado, no coincide o usa un formato no compatible.',
  accessFailure: 'No pudimos acceder o verificar los permisos. No se cambiaron los permisos.',
  unverifiedLibraries: 'No pudimos verificar las bibliotecas originales necesarias para identificar estos archivos.',
  changedFiles: 'Contiene archivos que cambiaron desde su registro o que no pudieron eliminarse.',
  unsafeFiles: 'Contiene enlaces o entradas que no se pueden eliminar de forma segura.',
  unknownFiles: 'Conserva archivos que no pertenecen al inventario verificado, incluidos archivos anteriores al seguimiento.',
};

export function nativeRetentionReason(reason: string): string {
  return Object.hasOwn(reasons, reason) ? reasons[reason] : 'No pudimos comprobar que sea seguro eliminar esta carpeta.';
}
