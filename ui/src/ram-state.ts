// Only type imports: the node test runner loads this module without resolving sibling files.
import type { CatalogInstance } from './preview-catalog';

/** What kind of game an automatic allocation is sized for; the same names the core uses. */
export type RamProfile = 'vanilla' | 'lite' | 'standard' | 'deluxe';
export type RamBasis = 'target' | 'limited' | 'floor' | 'unknown';
export type RamResolution = { mib: number; basis: RamBasis };
/** What an automatic allocation would be on this machine, per profile, as the core resolves it. */
export type RamPreview = {
  totalMib: number | null;
  vanilla: RamResolution; lite: RamResolution; standard: RamResolution; deluxe: RamResolution;
};

const GIB = 1024;
const SLIDER_MIN_MIB = 512;
const SLIDER_STEP_MIB = 512;

export const formatGiB = (mib: number) => Number((mib / GIB).toFixed(2));

/**
 * The profile of an instance, from what the catalog shows: vanilla has no pack, a Modrinth
 * pack counts as Standard, and a custom pack follows its installed variant (Standard until
 * installed). The core decides again from the installation when the game starts.
 */
export function ramProfileOf(instance: CatalogInstance | null | undefined): RamProfile | null {
  if (!instance) return null;
  switch (instance.source?.type) {
    case 'custom': {
      // The installed variant, as variant-state reads it: only while something is installed from a pinned pack.
      const installed = instance.localState !== 'not_installed' && instance.installedSource?.current_version_id ? instance.installedVariant : null;
      return installed === 'lite' || installed === 'deluxe' ? installed : 'standard';
    }
    case 'modrinth': return 'standard';
    default: return 'vanilla';
  }
}

/**
 * The top of the memory slider: the machine's own RAM, rounded to whole gigabytes (a
 * 16 GB machine reports a little less), so a player cannot drag past what is installed.
 * Without a reading of the machine it keeps the former 32 GiB range, widened to the
 * current value so that value can still be shown.
 */
export function sliderMaxMib(machineMib: number | null | undefined, current: number): number {
  if (typeof machineMib === 'number' && Number.isFinite(machineMib) && machineMib > 0) {
    return Math.max(SLIDER_MIN_MIB * 2, Math.round(machineMib / GIB) * GIB);
  }
  return Math.ceil(Math.min(262144, Math.max(32768, Number.isFinite(current) ? current : 0)) / SLIDER_STEP_MIB) * SLIDER_STEP_MIB;
}

/**
 * Where the slider's colors change, as fractions of its length. Purely visual: green while
 * the system keeps its room, yellow up to the point where it starts to suffer, red beyond.
 * The launch itself never relies on these numbers.
 */
export function sliderZones(machineMib: number | null | undefined, max: number): { green: number; yellow: number } {
  if (typeof machineMib !== 'number' || !(machineMib > 0) || !(max > 0)) return { green: 0.6, yellow: 0.85 };
  const yellow = Math.min(0.85, Math.max(0.3, (machineMib - Math.max(2 * GIB, machineMib / 4)) / max));
  return { green: Math.min(0.6, yellow - 0.15), yellow };
}

/** A saved value larger than the machine's RAM, worth a warning in manual mode. */
export function exceedsMachine(ram: number, machineMib: number | null | undefined): boolean {
  return typeof machineMib === 'number' && machineMib > 0 && Number.isFinite(ram) && ram > Math.round(machineMib / GIB) * GIB;
}

const basisNotes: Record<RamBasis, string> = {
  target: '',
  limited: ' Es menos de lo ideal porque tu equipo tiene poca RAM.',
  floor: ' Tu equipo tiene muy poca RAM; usaremos el mínimo para que el juego pueda abrir.',
  unknown: ' No pudimos leer toda la RAM de tu equipo, así que usaremos un valor prudente.',
};

/** The performance profile names of variant-state, plus Vanilla. */
const profileNames: Record<RamProfile, string> = { vanilla: 'Vanilla', lite: 'Ligero', standard: 'Estándar', deluxe: 'Mejorado' };
const profileOrder: readonly RamProfile[] = ['vanilla', 'lite', 'standard', 'deluxe'];

/**
 * The line shown under Automática. For an instance it names that instance's value; for
 * the global settings, where the kind of game is not known, it lists every kind.
 */
export function autoRamText(preview: RamPreview | null, profile: RamProfile | null): string {
  if (!preview) return 'Se calculará al jugar según tu equipo. No pudimos leer tu RAM ahora, así que se usará un valor prudente.';
  if (profile) {
    const value = preview[profile];
    return `Se calculará al jugar según tu equipo. Con la RAM de tu equipo serían ${formatGiB(value.mib)} GiB.${basisNotes[value.basis]}`;
  }
  const list = profileOrder.map(id => `${profileNames[id]} ${formatGiB(preview[id].mib)} GiB`).join(', ');
  const worst = profileOrder.map(id => preview[id].basis).find(basis => basis === 'floor' || basis === 'unknown' || basis === 'limited') ?? 'target';
  return `Se calculará al jugar según tu equipo y el tipo de instancia. Con la RAM de tu equipo: ${list}.${basisNotes[worst]}`;
}
