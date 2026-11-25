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
 * The scale under the slider. The slider runs from 512 MiB in steps of 512 MiB, so each step is
 * one tick (every few steps once there are more than 64, to keep them apart), and the middle
 * label sits at the fraction of the track where that value really is. It is left out when it
 * would crowd an end label.
 */
export function sliderScale(max: number): { steps: number; every: number; midAt: number | null } {
  const span = max - SLIDER_MIN_MIB;
  const steps = Math.max(1, Math.round(span / SLIDER_STEP_MIB));
  const mid = span > 0 ? (max / 2 - SLIDER_MIN_MIB) / span : 0;
  return { steps, every: Math.max(1, Math.ceil(steps / 64)), midAt: mid >= 0.2 && mid <= 0.8 ? mid : null };
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
  limited: ' Tu equipo tiene poca RAM, así que será menos de lo ideal.',
  floor: ' Tu equipo tiene muy poca RAM; se usará el mínimo para que el juego abra.',
  unknown: ' No pudimos leer toda la RAM; se usará un valor prudente.',
};

/** The automatic value for an instance of this kind, or null when it cannot be known yet. */
export function autoRamMib(preview: RamPreview | null, profile: RamProfile | null): number | null {
  return preview && profile ? preview[profile].mib : null;
}

/**
 * The short explanation behind Automática, shown as its tooltip. An instance adds a
 * remark only when its value had to be lowered; the global settings, where the kind of
 * game is not known, just say how the value is chosen.
 */
export function autoRamText(preview: RamPreview | null, profile: RamProfile | null): string {
  if (!preview) return 'No pudimos leer la RAM de tu equipo; se usará un valor prudente.';
  if (!profile) return 'Se calcula al jugar según la RAM de tu equipo y el tipo de instancia.';
  return 'Se calcula al jugar según la RAM de tu equipo.' + basisNotes[preview[profile].basis];
}
