import type { CatalogInstance, PackSource } from './preview-catalog';
// Installation variants of a custom instance. They are nested: Lite is contained in
// Standard, which is contained in Deluxe. Standard is always offered. The names are
// fixed by the API; the short descriptions are ours.
export type Variant = 'lite' | 'standard' | 'deluxe';

/** One choice before installing, as the backend reports it: counts are cumulative. */
export type VariantOption = { id: Variant; fileCount: number; size: number; sameAsPrevious: boolean };

export const variantOrder: readonly Variant[] = ['lite', 'standard', 'deluxe'];

export const variantNames: Record<Variant, string> = { lite: 'Ligero', standard: 'Estándar', deluxe: 'Mejorado' };

export const variantTaglines: Record<Variant, string> = { lite: 'Ligero y rápido', standard: 'Equilibrado', deluxe: 'Máxima experiencia' };

export const variantDescriptions: Record<Variant, string> = {
  lite: 'Lo mínimo para entrar al servidor, con resource packs y configuración. Sin mods cosméticos ni shaders: pesa poco y se instala rápido.',
  standard: 'La experiencia completa de la instancia: mods necesarios y opcionales, resource packs y configuración. Es la opción recomendada.',
  deluxe: 'La más completa: añade mods extra del lado del cliente, resource packs adicionales y shader packs.',
};

export function isVariant(value: unknown): value is Variant {
  return typeof value === 'string' && (variantOrder as readonly string[]).includes(value);
}

/** The variants an answer offers, in order. A missing or empty list is Standard alone. */
export function offeredVariants(value: readonly string[] | null | undefined): Variant[] {
  const offered = variantOrder.filter(variant => value?.includes(variant));
  return offered.length > 0 ? offered : ['standard'];
}

/**
 * The choices to show. A variant that installs exactly what the one below it does
 * adds nothing, so it is left out, unless it is the one already installed.
 */
export function selectableOptions(options: readonly VariantOption[], installed: Variant | null = null): VariantOption[] {
  return options.filter(option => !option.sameAsPrevious || option.id === installed);
}

/** The installed variant is chosen by default when changing; Standard when installing. */
export function defaultVariant(options: readonly VariantOption[], installed: Variant | null = null): Variant {
  if (installed && options.some(option => option.id === installed)) return installed;
  return options.some(option => option.id === 'standard') ? 'standard' : options[0]?.id ?? 'standard';
}

const sizes = new Intl.NumberFormat('es', { maximumFractionDigits: 1 });
function bytes(size: number): string {
  if (!Number.isFinite(size) || size <= 0) return '0 B';
  if (size < 1024 * 1024) return `${sizes.format(size / 1024)} KB`;
  if (size < 1024 * 1024 * 1024) return `${sizes.format(size / (1024 * 1024))} MB`;
  return `${sizes.format(size / (1024 * 1024 * 1024))} GB`;
}

export function optionSizeText(option: Pick<VariantOption, 'fileCount' | 'size'>): string {
  return `${bytes(option.size)} · ${option.fileCount === 1 ? '1 archivo' : `${option.fileCount} archivos`}`;
}

export function withdrawnVariantText(variant: Variant): string {
  return `La opción de rendimiento ${variantNames[variant]} ya no se ofrece en esta instancia. Pasamos tu instalación a ${variantNames.standard}.`;
}

// Rules over a catalog entry. They read the entry only; the backend decides again.

/** The pinned version of a custom pack source: its current or recorded version id. */
function pin(source: PackSource | null | undefined): string | null {
  return source?.type === 'custom' ? source.current_version_id ?? null : null;
}

function isCustom(instance: CatalogInstance): boolean {
  return instance.source?.type === 'custom';
}

/** Only a custom pack offers variants; vanilla and Modrinth are always Standard. */
export function instanceVariants(instance: CatalogInstance): Variant[] {
  return isCustom(instance) ? offeredVariants(instance.variants) : ['standard'];
}

/** The player has a choice only when the instance offers more than Standard. */
export function hasVariantChoice(instance: CatalogInstance): boolean {
  return instanceVariants(instance).length > 1;
}

/** The variant an installed pack was made from; a pack recorded before variants is Standard. */
export function installedVariant(instance: CatalogInstance): Variant | null {
  if (!isCustom(instance) || pin(instance.installedSource) === null) return null;
  return isVariant(instance.installedVariant) ? instance.installedVariant : 'standard';
}

/** The installed variant, when the instance stopped offering it and the answer is current. */
export function withdrawnVariant(instance: CatalogInstance, stale: boolean): Variant | null {
  const installed = installedVariant(instance);
  if (stale || instance.availability !== 'enabled' || !installed) return null;
  return instanceVariants(instance).includes(installed) ? null : installed;
}

/** True when a same-pin change can run now: the installed pin is the current one and nothing is in the way. */
function samePinReady(instance: CatalogInstance, stale: boolean): boolean {
  const from = pin(instance.installedSource);
  return !instance.removalPending && !stale && instance.availability === 'enabled' && instance.compatible && instance.supported
    && isCustom(instance) && from !== null && from === pin(instance.source)
    && ['installed', 'needs_repair'].includes(instance.localState);
}

export type VariantChange = { pin: string; installed: Variant };

/** What changing the variant of an installed pack starts from; null when it is not possible. */
export function variantChange(instance: CatalogInstance, stale: boolean): VariantChange | null {
  const installed = installedVariant(instance);
  const current = pin(instance.installedSource);
  if (!installed || !current || !hasVariantChoice(instance) || !samePinReady(instance, stale)) return null;
  return { pin: current, installed };
}

/**
 * The move to Standard that needs no question: the installed variant is withdrawn and
 * the pin did not change. A pin change goes through the confirmed update, which also
 * moves a withdrawn variant to Standard.
 */
export function automaticVariantSwitch(instance: CatalogInstance, stale: boolean): (VariantChange & { to: Variant }) | null {
  const withdrawn = withdrawnVariant(instance, stale);
  const current = pin(instance.installedSource);
  if (!withdrawn || !current || !samePinReady(instance, stale)) return null;
  return { pin: current, installed: withdrawn, to: 'standard' };
}

/** A choice as shown: with its sizes, or still waiting for them. */
export type VariantCard = VariantOption & { pending?: boolean };

/**
 * The cards to show. They come from what the catalog says is offered, so they exist before
 * the sizes do; an error shows none. Once the sizes arrive, a variant that adds nothing
 * over the one below is left out, unless it is the installed one.
 */
export function variantCards(variants: readonly Variant[], options: readonly VariantOption[] | null, error: string | null, installed: Variant | null = null): VariantCard[] {
  if (options) return selectableOptions(options, installed);
  if (error) return [];
  return variants.map(id => ({ id, fileCount: 0, size: 0, sameAsPrevious: false, pending: true }));
}

/** The variant picked, or the default when nothing was picked or the pick is no longer shown. */
export function resolveVariant(cards: readonly VariantCard[], picked: Variant | null, installed: Variant | null = null): Variant {
  return picked && cards.some(card => card.id === picked) ? picked : defaultVariant(cards, installed);
}

/** Stand-in sizes for the browser preview, where there is no backend to ask. */
export function previewOptions(variants: readonly Variant[]): VariantOption[] {
  return variants.map((id, index) => ({ id, fileCount: 12 * (index + 1), size: 48_000_000 * (index + 1), sameAsPrevious: false }));
}
