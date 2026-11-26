import type { ReactNode } from 'react';
import { Archive, Check, CircleCheck, ExternalLink, FileText, FolderOpen, Shield, ShieldCheck, TriangleAlert, Wrench, X } from 'lucide-react';
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

/** One kind of thing a finished operation kept or left behind: an icon and title, how many, a short note, and the rows. */
function DetailCard({ icon, title, count, note, children, foot }: { icon: ReactNode; title: string; count: number; note: string; children: ReactNode; foot?: ReactNode }) {
  return <div className="detail-card" role="group" aria-label={title}>
    <div className="detail-card-head">{icon}<h3>{title}</h3><span className="detail-card-count" aria-label={`${count} en total`}>{count}</span></div>
    <p className="detail-card-note">{note}</p>
    <ul className="detail-card-rows">{children}</ul>
    {foot && <div className="detail-card-foot">{foot}</div>}
  </div>;
}

function PathRows({ paths }: { paths: string[] }) {
  return <>{paths.map(path => <li key={path}><FileText aria-hidden="true" /><code>{path}</code></li>)}</>;
}

/**
 * What a finished operation kept or left behind, as the owner's template shows it: a header
 * that says the instance is ready (with the version it now has), then one card per kind.
 */
export function CompletionDialog({ heading, name, outcome, version, details, busy, error, onOpenBackups, onClose }: {
  heading: string; name: string | null; outcome: 'ready' | 'removed'; version: string | null; details: CompletionDetails;
  busy: boolean; error: string | null; onOpenBackups: () => void; onClose: () => void;
}) {
  const who = name ? <strong>{name}</strong> : 'La instancia';
  return <AccountDialog className="update-dialog update-modal" titleClassName="dialog-title" onCancel={onClose}
    title={<><CircleCheck aria-hidden="true" />{heading}</>}
    aside={outcome === 'ready' && version ? <span className="version-chip is-new">{version}</span> : undefined}>
    <p className="done-sub">{outcome === 'ready' ? <>{who} ya está listo para jugar.</> : <>{who} se desinstaló. Tus datos personales se conservan.</>}</p>
    {details.skipped.length > 0 && <DetailCard icon={<Shield aria-hidden="true" />} title="Archivos conservados" count={details.skipped.length} note={skippedSummary(details.skipped)}>
      <PathRows paths={details.skipped} />
    </DetailCard>}
    {details.released.length > 0 && <DetailCard icon={<ShieldCheck aria-hidden="true" />} title="Ahora son tuyos" count={details.released.length} note={releasedSummary(details.released)}>
      <PathRows paths={details.released} />
    </DetailCard>}
    {details.backedUp.length > 0 && <DetailCard icon={<Archive aria-hidden="true" />} title="Archivos guardados" count={details.backedUp.length} note={backedUpSummary(details.backedUp)}
      foot={<Button variant="secondary" size="sm" disabled={busy} onClick={onOpenBackups}><FolderOpen aria-hidden="true" />Abrir copias de archivos</Button>}>
      <PathRows paths={details.backedUp} />
    </DetailCard>}
    {details.natives && <DetailCard icon={<Wrench aria-hidden="true" />} title="Carpetas nativas antiguas" count={Math.max(details.natives.details.length, details.natives.retainedGenerations)}
      note={`${nativeCleanupSummary(details.natives)} Cierra los programas que usen esas carpetas y vuelve a intentar «Limpiar nativos antiguos».`}>
      {details.natives.details.map(detail => <li key={detail.directory}><FolderOpen aria-hidden="true" /><span><code>{detail.directory}</code><br />{nativeRetentionReason(detail.reason)}</span></li>)}
    </DetailCard>}
    {error && <p className="account-error" role="alert">{error}</p>}
    <div className="update-foot is-start"><Button onClick={onClose}><Check aria-hidden="true" />Aceptar</Button></div>
  </AccountDialog>;
}
