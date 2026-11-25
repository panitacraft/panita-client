import { variantIcons } from './variant-icons';
import { variantNames, type Recommendation } from '../variant-state';

/**
 * The sentence that explains the recommended performance profile, with the profile's name
 * colored and its icon beside it. The plain sentence stays the text a screen reader gets.
 */
export function VariantReason({ recommendation }: { recommendation: Recommendation }) {
  const reason = recommendation.reason ?? '';
  const name = variantNames[recommendation.variant];
  const at = reason.indexOf(name);
  const Icon = variantIcons[recommendation.variant];
  if (at < 0) return <p className="variant-reason">{reason}</p>;
  return <p className="variant-reason">{reason.slice(0, at)}<strong className="variant-reason-name"><Icon aria-hidden="true" />{name}</strong>{reason.slice(at + name.length)}</p>;
}
