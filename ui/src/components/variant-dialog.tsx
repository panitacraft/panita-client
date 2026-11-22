import { useState } from 'react';
import { Download, RefreshCw, X } from 'lucide-react';
import { AccountDialog } from './account-dialog';
import { Button } from './ui/button';
import {
  defaultVariant, optionSizeText, selectableOptions, variantDescriptions, variantNames,
  type Variant, type VariantOption,
} from '../variant-state';

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
  const choices = options ? selectableOptions(options, installed) : [];
  const [picked, setPicked] = useState<Variant | null>(null);
  const selected = picked && choices.some(option => option.id === picked) ? picked : defaultVariant(choices, installed);
  const unchanged = mode === 'change' && selected === installed;
  return <AccountDialog title={mode === 'install' ? 'Elige cómo instalar' : 'Cambiar variante'} className="update-dialog variant-dialog" onCancel={onCancel}>
    <p>{mode === 'install'
      ? 'Esta instancia tiene varias variantes. Cada una incluye todo lo de la anterior. Puedes cambiarla después sin perder tus mundos.'
      : 'Descargaremos lo que falte y quitaremos solo los archivos que instalamos nosotros. Tus mundos, mods añadidos y ajustes editados se conservan.'}</p>
    {error && <p className="account-error" role="alert">{error}</p>}
    {!options && !error && <p role="status">Consultando las variantes…</p>}
    {options && <fieldset className="variant-options">
      <legend className="sr-only">Variante</legend>
      {choices.map(option => <label key={option.id} className="backup-choice variant-option">
        <input type="radio" name="variant" value={option.id} checked={selected === option.id} onChange={() => setPicked(option.id)} />
        <span>
          <strong>{variantNames[option.id]}</strong>
          {option.id === 'standard' && mode === 'install' ? ' · recomendada' : ''}
          {option.id === installed ? ' · instalada' : ''}
          <small>{variantDescriptions[option.id]}</small>
          <small>{optionSizeText(option)}</small>
        </span>
      </label>)}
    </fieldset>}
    <div className="account-buttons">
      <Button disabled={busy || !options || choices.length === 0 || unchanged} onClick={() => onConfirm(selected)}>
        {mode === 'install' ? <Download aria-hidden="true" /> : <RefreshCw aria-hidden="true" />}{mode === 'install' ? 'Instalar' : 'Cambiar variante'}
      </Button>
      <Button variant="secondary" disabled={busy} onClick={onCancel}><X aria-hidden="true" />Cancelar</Button>
    </div>
  </AccountDialog>;
}
