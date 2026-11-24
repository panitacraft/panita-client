import { useId } from 'react';
import { variantIcons } from './variant-icons';
import { optionSizeText, variantDescriptions, variantNames, variantTaglines, type Variant, type VariantCard } from '../variant-state';

/**
 * The variant cards: one row that scrolls sideways when the window is narrow. They are
 * radio buttons, named by their variant and described by their text. While the sizes load,
 * the cards are already in place and cannot be chosen, so nothing moves when they arrive.
 */
export function VariantPicker({ cards, loading, selected, installed = null, recommended = null, onPick }: {
  cards: VariantCard[];
  loading: boolean;
  selected: Variant;
  installed?: Variant | null;
  /** The variant that suits this machine, marked on its card. */
  recommended?: Variant | null;
  onPick: (variant: Variant) => void;
}) {
  const prefix = useId();
  if (cards.length === 0) return null;
  return <fieldset className="variant-options" aria-busy={loading}>
    <legend className="sr-only">Rendimiento</legend>
    {cards.map(card => {
      const Icon = variantIcons[card.id];
      return <label key={card.id} className="variant-option">
        <input className="sr-only" type="radio" name="variant" disabled={loading} aria-labelledby={`${prefix}-${card.id}-name`} aria-describedby={`${prefix}-${card.id}-text`} value={card.id} checked={selected === card.id} onChange={() => onPick(card.id)} />
        <span className="variant-head">
          <span className="variant-icon" aria-hidden="true"><Icon /></span>
          <span className="variant-chips">
            {card.id === installed && <span className="variant-chip is-installed">Instalada</span>}
            {card.id === recommended && <span className="variant-chip">Recomendada</span>}
          </span>
        </span>
        <span className="variant-text" id={`${prefix}-${card.id}-text`}>
          <strong id={`${prefix}-${card.id}-name`}>{variantNames[card.id]}</strong>
          <span className="variant-tagline">{variantTaglines[card.id]}</span>
          <small>{variantDescriptions[card.id]}</small>
          {card.pending
            ? <small className="variant-meta variant-pending" aria-hidden="true"><span className="variant-spinner" />Calculando tamaño…</small>
            : <small className="variant-meta variant-ready">{optionSizeText(card)}</small>}
        </span>
      </label>;
    })}
  </fieldset>;
}
