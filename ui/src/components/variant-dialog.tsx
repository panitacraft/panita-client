import { useId, useState } from 'react';
import { Download, Feather, Package, RefreshCw, Sparkles, X, type LucideIcon } from 'lucide-react';
import { AccountDialog } from './account-dialog';
import { Button } from './ui/button';
import {
  defaultVariant, optionSizeText, selectableOptions, variantDescriptions, variantNames, variantTaglines,
  type Variant, type VariantOption,
} from '../variant-state';

/** Light for the essentials, a box for the whole instance, sparkles for the extras. */
const variantIcons: Record<Variant, LucideIcon> = { lite: Feather, standard: Package, deluxe: Sparkles };

/**
 * The choice of a variant, before installing or when changing an installation. Options
 * that add nothing over the one below are left out, so nobody picks a variant that is
 * the same as another. Standard is selected by default when installing.
 */
export function VariantDialog({ mode, options, installed = null, error = null, busy, onConfirm, onCancel }: {
  mode: 'install' | 'change';
  /** Null while the sizes are being fetched. */
  options: VariantOption[] | null;
  installed?: Variant | null;
  error?: string | null;
  busy: boolean;
  onConfirm: (variant: Variant) => void;
  onCancel: () => void;
}) {
  const prefix = useId();
  const choices = options ? selectableOptions(options, installed) : [];
  const [picked, setPicked] = useState<Variant | null>(null);
  const selected = picked && choices.some(option => option.id === picked) ? picked : defaultVariant(choices, installed);
  const unchanged = mode === 'change' && selected === installed;
  return <AccountDialog title={mode === 'install' ? 'Elige cómo instalar' : 'Cambiar variante'} className="variant-dialog" onCancel={onCancel}>
    <p>{mode === 'install'
      ? 'Esta instancia tiene varias variantes. Cada una incluye todo lo de la anterior. Puedes cambiarla después sin perder tus mundos.'
      : 'Descargaremos lo que falte y quitaremos solo los archivos que instalamos nosotros. Tus mundos, mods añadidos y ajustes editados se conservan.'}</p>
    {error && <p className="account-error" role="alert">{error}</p>}
    {!options && !error && <p role="status">Consultando las variantes…</p>}
    {options && <fieldset className="variant-options">
      <legend className="sr-only">Variante</legend>
      {choices.map(option => {
        const Icon = variantIcons[option.id];
        return <label key={option.id} className="variant-option">
          <input className="sr-only" type="radio" name="variant" aria-labelledby={`${prefix}-${option.id}-name`} aria-describedby={`${prefix}-${option.id}-text`} value={option.id} checked={selected === option.id} onChange={() => setPicked(option.id)} />
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
            <small className="variant-meta">{optionSizeText(option)}</small>
          </span>
        </label>;
      })}
    </fieldset>}
    <div className="account-buttons">
      <Button disabled={busy || !options || choices.length === 0 || unchanged} onClick={() => onConfirm(selected)}>
        {mode === 'install' ? <Download aria-hidden="true" /> : <RefreshCw aria-hidden="true" />}{mode === 'install' ? 'Instalar' : 'Cambiar variante'}
      </Button>
      <Button variant="secondary" disabled={busy} onClick={onCancel}><X aria-hidden="true" />Cancelar</Button>
    </div>
  </AccountDialog>;
}
