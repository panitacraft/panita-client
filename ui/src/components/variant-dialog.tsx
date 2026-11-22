import { useState } from 'react';
import { Download, X } from 'lucide-react';
import { AccountDialog } from './account-dialog';
import { Button } from './ui/button';
import { VariantPicker } from './variant-picker';
import { resolveVariant, variantCards, type Variant, type VariantOption } from '../variant-state';

/**
 * The choice of a variant before installing. Standard is selected by default. The cards
 * appear at once, from what the catalog already says is offered; only each size waits for
 * the backend, so the dialog never changes height when the answer comes.
 */
export function VariantDialog({ variants, options, error = null, busy, onConfirm, onCancel }: {
  /** The variants the instance offers, known before the sizes are. */
  variants: Variant[];
  /** Null while the sizes are being fetched. */
  options: VariantOption[] | null;
  error?: string | null;
  busy: boolean;
  onConfirm: (variant: Variant) => void;
  onCancel: () => void;
}) {
  const loading = !options && !error;
  const cards = variantCards(variants, options, error);
  const [picked, setPicked] = useState<Variant | null>(null);
  const selected = resolveVariant(cards, picked);
  return <AccountDialog title="Elige cómo instalar" className="variant-dialog" onCancel={onCancel}>
    <p>Esta instancia tiene varias variantes. Cada una incluye todo lo de la anterior. Puedes cambiarla después desde los ajustes de la instancia, sin perder tus mundos.</p>
    {error && <p className="account-error" role="alert">{error}</p>}
    {loading && <p className="sr-only" role="status">Consultando las variantes…</p>}
    <VariantPicker cards={cards} loading={loading} selected={selected} recommended onPick={setPicked} />
    <div className="account-buttons">
      <Button disabled={busy || loading || cards.length === 0} onClick={() => onConfirm(selected)}><Download aria-hidden="true" />Instalar</Button>
      <Button variant="secondary" disabled={busy} onClick={onCancel}><X aria-hidden="true" />Cancelar</Button>
    </div>
  </AccountDialog>;
}
