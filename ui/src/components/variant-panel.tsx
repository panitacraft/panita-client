import { useEffect, useState } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { RefreshCw } from 'lucide-react';
import { Button } from './ui/button';
import { VariantPicker } from './variant-picker';
import { messageForError } from '../errors';
import type { CatalogInstance } from '../preview-catalog';
import { usePlayControl } from '../play';
import { playActive } from '../play-state';
import { instanceVariants, previewOptions, resolveVariant, variantCards, variantChange, type Variant, type VariantOption } from '../variant-state';

/**
 * The "Variante" tab of an instance's settings. Changing it is an update of the same
 * published version: it downloads what is missing and removes only what the launcher
 * installed. Worlds are never touched.
 */
export function VariantPanel({ instance, stale, onStarted }: { instance: CatalogInstance; stale: boolean; onStarted: () => void }) {
  const play = usePlayControl();
  const change = variantChange(instance, stale);
  const variants = instanceVariants(instance);
  const [options, setOptions] = useState<VariantOption[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [picked, setPicked] = useState<Variant | null>(null);
  const [attempt, setAttempt] = useState(0);
  const entry = play?.snapshot?.instances.find(item => item.instanceId === instance.id);
  const working = !!entry && playActive(entry.phase);
  const installed = change?.installed ?? null;
  const canLoad = !!change;

  useEffect(() => {
    if (!canLoad) return;
    let current = true;
    setOptions(null); setError(null);
    if (!isTauri()) { setOptions(previewOptions(variants)); return; }
    void invoke<VariantOption[]>('instance_variants', { instanceId: instance.id })
      .then(next => { if (current) setOptions(next.length > 0 ? next : previewOptions(variants).map(option => ({ ...option, fileCount: 0, size: 0 }))); })
      .catch(cause => { if (current) setError(messageForError(cause)); });
    return () => { current = false; };
  }, [instance.id, canLoad, attempt]);

  if (!change) {
    return <section className="variant-panel" aria-labelledby="variant-heading">
      <h2 id="variant-heading" className="sr-only">Variante</h2>
      <p>{installed === null && !instance.installedSource
        ? 'Instala la instancia para poder cambiar su variante. Al instalarla podrás elegir una.'
        : 'Ahora mismo no se puede cambiar la variante. Actualiza la instancia o comprueba tu conexión e inténtalo de nuevo.'}</p>
    </section>;
  }
  const loading = !options && !error;
  const cards = variantCards(variants, options, error, installed);
  const selected = resolveVariant(cards, picked, installed);
  const unchanged = selected === installed;

  async function apply() {
    if (!play || !change || unchanged) return;
    // The same published version: only the variant changes. Worlds are never touched.
    await play.act('begin_update', { instanceId: instance.id, requestId: crypto.randomUUID(), update: { from_version: change.pin, to_version: change.pin, backup_worlds: false, from_variant: change.installed, to_variant: selected } });
    onStarted();
  }

  return <section className="variant-panel" aria-labelledby="variant-heading">
    <h2 id="variant-heading" className="sr-only">Variante</h2>
    <p>Descargaremos lo que falte y quitaremos solo los archivos que instalamos nosotros. Tus mundos, mods añadidos y ajustes editados se conservan.</p>
    {error && <div role="alert"><p className="account-error">No pudimos consultar las variantes. {error}</p><button type="button" onClick={() => setAttempt(previous => previous + 1)}>Reintentar</button></div>}
    {loading && <p className="sr-only" role="status">Consultando las variantes…</p>}
    <VariantPicker cards={cards} loading={loading} selected={selected} installed={installed} onPick={setPicked} />
    <div className="account-buttons">
      <Button disabled={loading || !!error || working || unchanged || !isTauri()} onClick={() => void apply()}><RefreshCw aria-hidden="true" />Cambiar variante</Button>
    </div>
    {working && <p role="status">Hay una operación en curso en esta instancia. Espera a que termine.</p>}
  </section>;
}
