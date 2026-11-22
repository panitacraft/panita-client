import { useId, useState } from 'react';
import { Download, RefreshCw, X } from 'lucide-react';
import { AccountDialog } from './account-dialog';
import { Button } from './ui/button';
import { variantIcons } from './variant-icons';
import {
  defaultVariant, optionSizeText, selectableOptions, variantDescriptions, variantNames, variantTaglines,
  type Variant, type VariantOption,
} from '../variant-state';

/**
 * The choice of a variant, before installing or when changing an installation. Options
 * that add nothing over the one below are left out, so nobody picks a variant that is
 * the same as another. Standard is selected by default when installing.
 *
 * The cards appear at once, from what the catalog already says is offered; only each
 * size waits for the backend. So the dialog never changes height when the answer comes.
 */
export function VariantDialog({ mode, variants, options, installed = null, error = null, busy, onConfirm, onCancel }: {
  mode: 'install' | 'change';
  /** The variants the instance offers, known before the sizes are. */
  variants: Variant[];
  /** Null while the sizes are being fetched. */
  options: VariantOption[] | null;
  installed?: Variant | null;
  error?: string | null;
  busy: boolean;
  onConfirm: (variant: Variant) => void;
  onCancel: () => void;
}) {
  const prefix = useId();
  const loading = !options && !error;
  const cards: (VariantOption & { pending?: boolean })[] = options
    ? selectableOptions(options, installed)
    : error ? [] : variants.map(id => ({ id, fileCount: 0, size: 0, sameAsPrevious: false, pending: true }));
  const [picked, setPicked] = useState<Variant | null>(null);
  const selected = picked && cards.some(option => option.id === picked) ? picked : defaultVariant(cards, installed);
  const unchanged = mode === 'change' && selected === installed;
  return <AccountDialog title={mode === 'install' ? 'Elige cómo instalar' : 'Cambiar variante'} className="variant-dialog" onCancel={onCancel}>
    <p>{mode === 'install'
      ? 'Esta instancia tiene varias variantes. Cada una incluye todo lo de la anterior. Puedes cambiarla después sin perder tus mundos.'
      : 'Descargaremos lo que falte y quitaremos solo los archivos que instalamos nosotros. Tus mundos, mods añadidos y ajustes editados se conservan.'}</p>
    {error && <p className="account-error" role="alert">{error}</p>}
    {loading && <p className="sr-only" role="status">Consultando las variantes…</p>}
    {cards.length > 0 && <fieldset className="variant-options" aria-busy={loading}>
      <legend className="sr-only">Variante</legend>
      {cards.map(option => {
        const Icon = variantIcons[option.id];
        return <label key={option.id} className="variant-option">
          <input className="sr-only" type="radio" name="variant" disabled={loading} aria-labelledby={`${prefix}-${option.id}-name`} aria-describedby={`${prefix}-${option.id}-text`} value={option.id} checked={selected === option.id} onChange={() => setPicked(option.id)} />
          <span className="variant-head">
            <span className="variant-icon" aria-hidden="true"><Icon /></span>
            <span className="variant-chips">
              {option.id === installed && <span className="variant-chip is-installed">Instalada</span>}
              {option.id === 'standard' && mode === 'install' && <span className="variant-chip">Recomendada</span>}
            </span>
          </span>
          <span className="variant-text" id={`${prefix}-${option.id}-text`}>
            <strong id={`${prefix}-${option.id}-name`}>{variantNames[option.id]}</strong>
            <span className="variant-tagline">{variantTaglines[option.id]}</span>
            <small>{variantDescriptions[option.id]}</small>
            {option.pending
              ? <small className="variant-meta variant-pending" aria-hidden="true"><span className="variant-spinner" />Calculando tamaño…</small>
              : <small className="variant-meta variant-ready">{optionSizeText(option)}</small>}
          </span>
        </label>;
      })}
    </fieldset>}
    <div className="account-buttons">
      <Button disabled={busy || loading || cards.length === 0 || unchanged} onClick={() => onConfirm(selected)}>
        {mode === 'install' ? <Download aria-hidden="true" /> : <RefreshCw aria-hidden="true" />}{mode === 'install' ? 'Instalar' : 'Cambiar variante'}
      </Button>
      <Button variant="secondary" disabled={busy} onClick={onCancel}><X aria-hidden="true" />Cancelar</Button>
    </div>
  </AccountDialog>;
}
