import { Download, X, Wrench, RefreshCw, Trash2, ShieldCheck } from 'lucide-react';
import type { PlayEntry } from '../play-state';
import { formatBytes, packProgressFraction } from '../pack-state';
import { Button } from './ui/button';

export function InstanceProgress({ entry, busy, onCancel, title }: { entry: PlayEntry; busy: boolean; onCancel: () => void; title?: string }) {
  const pack = entry.packProgress;
  const measured = !!pack && pack.bytesTotal > 0;
  const artifact = entry.artifactProgress;
  const fraction = measured ? packProgressFraction(pack) : 0;
  const label = entry.phase === 'cancelling' ? 'Cancelando…'
    : entry.operation === 'play' ? 'Preparando Minecraft…'
    : entry.operation === 'uninstall' ? 'Quitando la instancia…'
    : entry.operation === 'repair' ? 'Comprobando archivos…'
    : entry.operation === 'update' ? 'Actualizando la instancia…' : 'Instalando la instancia…';
  const Icon = entry.operation === 'repair' ? Wrench : entry.operation === 'update' ? RefreshCw : entry.operation === 'uninstall' ? Trash2 : entry.operation === 'play' ? ShieldCheck : Download;
  return <div className="operation-card" role="status">
    <div className="operation-heading"><Icon aria-hidden="true" /><span className="operation-title">{title && <small>{title}</small>}<strong>{label}</strong></span>
      <Button variant="ghost" size="icon" aria-label="Cancelar operación" disabled={busy || entry.phase === 'cancelling'} onClick={onCancel}><X aria-hidden="true" /></Button>
    </div>
    {measured ? <>
      <div className="operation-metrics"><span>{formatBytes(pack.bytesDone)} / {formatBytes(pack.bytesTotal)}</span><strong>{Math.floor(fraction * 100)} %</strong></div>
      <progress aria-label="Descarga del modpack" max={1} value={fraction} />
      <p>{pack.filesDone} / {pack.filesTotal} archivos{fraction === 1 ? ' · Verificando la instalación…' : ''}</p>
    </> : <>
      {artifact && artifact.total > 0 && <>
        <div className="operation-metrics"><span>Archivo de Minecraft / Java</span><span>{formatBytes(artifact.bytes)} / {formatBytes(artifact.total)}</span></div>
        <progress aria-label="Descarga del archivo actual" max={artifact.total} value={Math.min(artifact.total, artifact.bytes)} />
      </>}
      {!(artifact && artifact.total > 0) && <div className="indeterminate-bar" role="progressbar" aria-label="Procesando" />}
      <p>{entry.operation === 'play' ? 'Comprobando archivos y cuenta.' : `${entry.receivedFiles} archivos recibidos · ${entry.reusedFiles} reutilizados`}</p>
    </>}
  </div>;
}
