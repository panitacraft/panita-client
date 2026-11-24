import { Feather, Package, Sparkles, type LucideIcon } from 'lucide-react';
import type { Variant } from '../variant-state';

/** A feather for the light one, a box for the whole instance, sparkles for the extras. */
export const variantIcons: Record<Variant, LucideIcon> = { lite: Feather, standard: Package, deluxe: Sparkles };
