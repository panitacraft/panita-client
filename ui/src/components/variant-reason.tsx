import { Info } from 'lucide-react';
import { variantNames, type Recommendation } from '../variant-state';

/**
 * The sentence that explains the recommended performance profile, led by an info icon,
 * with the profile's name colored. The plain sentence stays the text a screen reader gets.
 */
export function VariantReason({ recommendation }: { recommendation: Recommendation }) {
  const reason = recommendation.reason ?? '';
  const name = variantNames[recommendation.variant];
  const at = reason.indexOf(name);
  return <p className="variant-reason"><Info aria-hidden="true" /><span>{at < 0 ? reason : <>{reason.slice(0, at)}<strong className="variant-reason-name">{name}</strong>{reason.slice(at + name.length)}</>}</span></p>;
}
