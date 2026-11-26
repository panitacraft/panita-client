import type { ReactNode } from 'react';
import { ExternalLink, FolderOpen, ShieldCheck, TriangleAlert, X } from 'lucide-react';
import { AccountDialog } from './account-dialog';
import { Button } from './ui/button';
import { nativeCleanupSummary, nativeRetentionReason, type NativeCleanupReport } from '../native-cleanup-state';
import type { CompletionDetails } from '../notice-state';
import {
  backedUpSummary, conflictHeading, conflictReasonText, folderHint, manualHeading, reasonText, releasedSummary, skippedSummary,
  type PackConflict, type PackFile,
} from '../pack-state';

/** The modal every "Ver detalles" toast opens, shaped like the update and uninstall prompts. */
export function DetailsDialog({ title, onClose, closeLabel = 'Cerrar', children, actions }: { title: ReactNode; onClose: () => void; closeLabel?: string; children: ReactNode; actions?: ReactNode }) {
  return <AccountDialog className="update-dialog" titleClassName="dialog-title" title={title} onCancel={onClose}>
    {children}
    <div className="account-buttons">
      {actions}
      <Button variant="secondary" onClick={onClose}><X aria-hidden="true" />{closeLabel}</Button>
    </div>
  </AccountDialog>;
}

function PathList({ label, paths }: { label: string; paths: string[] }) {
  return <ul className="detail-files" aria-label={label}>{paths.map(path => <li key={path}><code>{path}</code></li>)}</ul>;
}

/** Player files that are in the way. Continuing is offered only when moving them aside can work. */
export function ConflictsDialog({ conflicts, operation, canContinue, resolvable, busy, onContinue, onClose }: {
  conflicts: PackConflict[]; operation: string; canContinue: boolean; resolvable: boolean; busy: boolean; onContinue: () => void; onClose: () => void;
}) {
  return <DetailsDialog title={<><TriangleAlert aria-hidden="true" />{conflictHeading(conflicts.length, operation)}</>}
    closeLabel={canContinue ? 'Cancelar' : 'Cerrar'} onClose={onClose}
    actions={canContinue && <Button disabled={busy} onClick={onContinue}><ShieldCheck aria-hidden="true" />Continuar y guardar mis archivos</Button>}>
    <ul className="update-notes">
      <li><ShieldCheck aria-hidden="true" />No cambiamos nada: tu instalación sigue como estaba.</li>
      {canContinue
        ? <li><FolderOpen aria-hidden="true" /><span>Si continúas, movemos tus archivos a la carpeta <code>pack-backups</code> de esta instancia (no los borramos) e instalamos los del modpack.</span></li>
        : <li className="is-warning"><TriangleAlert aria-hidden="true" />{resolvable
          ? 'Muévelos o elimínalos y vuelve a intentarlo. Nunca los reemplazamos por nuestra cuenta.'
          : 'Mueve esa carpeta o enlace fuera de la instancia y vuelve a intentarlo.'}</li>}
    </ul>
    <h3 className="detail-heading">Archivos afectados</h3>
    <ul className="detail-files" aria-label="Archivos afectados">{conflicts.map(conflict => <li key={conflict.path}><code>{conflict.path}</code>: {conflictReasonText(conflict.reason)}</li>)}</ul>
  </DetailsDialog>;
}

/** Files the player must download by hand, each with the buttons that lead to it. */
export function ManualFilesDialog({ files, busy, onOpenSource, onOpenFolder, onClose }: {
  files: PackFile[]; busy: boolean; onOpenSource: (file: PackFile) => void; onOpenFolder: (file: PackFile) => void; onClose: () => void;
}) {
  return <DetailsDialog title={<><TriangleAlert aria-hidden="true" />{manualHeading(files.length)}</>} onClose={onClose}>
    <ul className="update-notes">
      <li><ShieldCheck aria-hidden="true" />No cambiamos nada. Descarga estos archivos, guárdalos en la carpeta indicada y vuelve a intentarlo.</li>
    </ul>
    <ul className="detail-files" aria-label="Archivos que faltan">{files.map(file => <li key={file.path}>
      <strong>{file.name}</strong> (<code>{file.fileName}</code>)
      <div>{reasonText(file.reason)} {folderHint(file)}</div>
      <div className="account-buttons">
        <Button variant="secondary" size="sm" disabled={busy} onClick={() => onOpenSource(file)}><ExternalLink aria-hidden="true" />Abrir CurseForge</Button>
        <Button variant="secondary" size="sm" disabled={busy} onClick={() => onOpenFolder(file)}><FolderOpen aria-hidden="true" />Abrir carpeta</Button>
      </div>
    </li>)}</ul>
  </DetailsDialog>;
}

/** Leftover native folders and why they stayed. */
export function NativeCleanupSection({ report }: { report: NativeCleanupReport }) {
  return <>
    <p className="detail-intro">{nativeCleanupSummary(report)}</p>
    <p className="detail-intro">Cierra los programas que usen esas carpetas y vuelve a intentar «Limpiar nativos antiguos».</p>
    {report.details.length > 0 && <ul className="detail-files" aria-label="Carpetas nativas conservadas">
      {report.details.map(detail => <li key={detail.directory}><code>{detail.directory}</code>: {nativeRetentionReason(detail.reason)}</li>)}
    </ul>}
  </>;
}

/** Everything a finished operation kept or left behind, one section per kind. */
export function CompletionDialog({ title, details, busy, error, onOpenBackups, onClose }: {
  title: ReactNode; details: CompletionDetails; busy: boolean; error: string | null; onOpenBackups: () => void; onClose: () => void;
}) {
  return <DetailsDialog title={title} onClose={onClose}>
    {details.skipped.length > 0 && <div><p className="detail-intro">{skippedSummary(details.skipped)}</p><PathList label="Archivos conservados" paths={details.skipped} /></div>}
    {details.released.length > 0 && <div><p className="detail-intro">{releasedSummary(details.released)}</p><PathList label="Archivos que ahora son tuyos" paths={details.released} /></div>}
    {details.backedUp.length > 0 && <div>
      <p className="detail-intro">{backedUpSummary(details.backedUp)}</p>
      <PathList label="Archivos guardados" paths={details.backedUp} />
      <div className="account-buttons"><Button variant="secondary" size="sm" disabled={busy} onClick={onOpenBackups}><FolderOpen aria-hidden="true" />Abrir copias de archivos</Button></div>
    </div>}
    {details.natives && <div><NativeCleanupSection report={details.natives} /></div>}
    {error && <p className="account-error" role="alert">{error}</p>}
  </DetailsDialog>;
}
