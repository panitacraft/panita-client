import type { LauncherErrorCode } from './errors';

export type Workflow = 'play' | 'install' | 'update' | 'repair' | 'uninstall';
export type DiagnosticPhase = 'preparing' | 'installing' | 'installed' | 'uninstalled' | 'warning' | 'starting' | 'running' | 'stopping' | 'cancelling' | 'exited' | 'exited_with_error' | 'failed' | 'cancelled';
export type DiagnosticRecord = {
  processSessionId: string;
  operationId: string | null;
  event: 'application_started' | 'operation_cancelled' | 'storage_failure' | 'catalog_failure' | 'workflow_changed';
  workflow: Workflow | null;
  phase: DiagnosticPhase | null;
  error: LauncherErrorCode | null;
};
export type DiagnosticSnapshot = { capacity: number; records: DiagnosticRecord[] };

const workflows: Record<Workflow, string> = {
  play: 'Inicio de Minecraft', install: 'Instalación', update: 'Actualización', repair: 'Reparación', uninstall: 'Desinstalación',
};
const phases: Record<DiagnosticPhase, string> = {
  preparing: 'comprobando archivos', installing: 'preparando archivos', installed: 'completada',
  uninstalled: 'completada conservando datos', warning: 'esperando confirmación', starting: 'iniciando Minecraft',
  running: 'Minecraft en ejecución', stopping: 'esperando el cierre', cancelling: 'cancelando',
  exited: 'Minecraft se cerró', exited_with_error: 'Minecraft se cerró de forma inesperada',
  failed: 'falló', cancelled: 'cancelada',
};

export function diagnosticText(record: DiagnosticRecord): string {
  switch (record.event) {
    case 'application_started': return 'Se inició Panita Client.';
    case 'operation_cancelled': return 'Se canceló una operación.';
    case 'storage_failure': return 'No pudimos acceder o guardar los datos del cliente.';
    case 'catalog_failure': return 'Hubo un problema al actualizar el catálogo o sus datos locales.';
    case 'workflow_changed':
      if (record.workflow && Object.hasOwn(workflows, record.workflow) && record.phase && Object.hasOwn(phases, record.phase)) {
        return `${workflows[record.workflow]}: ${phases[record.phase]}.`;
      }
  }
  // Do not render unknown backend strings, even if a future build adds events.
  return 'Evento del cliente no reconocido por esta versión.';
}

export function visibleDiagnosticRecords(snapshot: DiagnosticSnapshot): DiagnosticRecord[] {
  return snapshot.records.slice(-256).reverse();
}
