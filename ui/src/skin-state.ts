// What the backend reports about the selected Microsoft account's skin and capes. Textures
// are named by key (a pixel hash for skins, the service's identifier for capes); the
// frontend never handles a file path or a credential.
export type SkinModel = 'classic' | 'slim';

export type CurrentLook = {
  /** Whether the account has a skin of its own; without one the game shows a default. */
  custom: boolean;
  /** Key of the cached skin image, null while it could not be fetched. */
  texture: string | null;
  model: SkinModel;
  cape: string | null;
};
export type CapeView = { id: string; alias: string; cached: boolean; active: boolean };
export type LookView = { id: string; name: string; model: SkinModel; texture: string; cape: string | null; createdMs: number };
export type SkinsView = {
  profileId: string;
  profileName: string;
  current: CurrentLook | null;
  capes: CapeView[];
  /** Newest first. */
  library: LookView[];
  matchingLook: string | null;
  syncedMs: number | null;
  incomplete: boolean;
};
export type Saved = { id: string; view: SkinsView };
export type ApplyRequest = { lookId: string | null; capeChange?: { id: string | null } };

/** Keep in step with the backend: a saved look's name is 1 to 32 characters. */
export const MAX_LOOK_NAME = 32;
/** Skin files are a few kilobytes; the backend refuses anything over a megabyte anyway. */
export const MAX_SKIN_FILE_BYTES = 512 * 1024;

export const modelLabels: Record<SkinModel, string> = { classic: 'Clásico (Steve)', slim: 'Delgado (Alex)' };
export const modelShortLabels: Record<SkinModel, string> = { classic: 'Clásico', slim: 'Delgado' };

/**
 * What the player is previewing but has not applied: a saved look, replacing the skin and model,
 * and/or a cape choice. `cape: null` keeps whatever cape the look or the account implies.
 */
export type Selection = { lookId: string | null; cape: { id: string | null } | null };
export const noSelection: Selection = { lookId: null, cape: null };

/** What the 3D viewer should show for a selection. */
export type Preview = { texture: string | null; model: SkinModel; cape: string | null; custom: boolean };

const ownsCape = (view: SkinsView, id: string) => view.capes.some(cape => cape.id === id);

/**
 * The cape a saved look puts on the account: its own when the account owns it, none when the
 * look has none, and the cape already worn when the look names one the account lacks. The
 * backend applies the same rule, so the preview is what applying produces.
 */
export function lookCape(view: SkinsView, look: LookView): string | null {
  if (look.cape === null) return null;
  return ownsCape(view, look.cape) ? look.cape : view.current?.cape ?? null;
}

export function previewOf(view: SkinsView, selection: Selection): Preview {
  const look = selection.lookId === null ? undefined : view.library.find(item => item.id === selection.lookId);
  const current = view.current;
  const skin = look
    ? { texture: look.texture, model: look.model, custom: true }
    : { texture: current?.texture ?? null, model: current?.model ?? 'classic' as SkinModel, custom: current?.custom ?? false };
  const cape = selection.cape ? selection.cape.id : look ? lookCape(view, look) : current?.cape ?? null;
  return { ...skin, cape };
}

/** Whether the preview differs from what the account wears. */
export function isDirty(view: SkinsView, selection: Selection): boolean {
  const current = view.current;
  if (!current) return false;
  const preview = previewOf(view, selection);
  return preview.texture !== current.texture || preview.model !== current.model || preview.cape !== current.cape;
}

/** The request that turns the current look into the previewed one, or null when nothing differs. */
export function applyRequestOf(view: SkinsView, selection: Selection): ApplyRequest | null {
  const current = view.current;
  if (!current || !isDirty(view, selection)) return null;
  const preview = previewOf(view, selection);
  const skinChanged = preview.texture !== current.texture || preview.model !== current.model;
  const look = skinChanged ? selection.lookId : null;
  const request: ApplyRequest = { lookId: look };
  if (preview.cape !== current.cape) request.capeChange = { id: preview.cape };
  return look === null && request.capeChange === undefined ? null : request;
}

/** Choosing a saved look previews it whole, including its own cape. Choosing it again clears it. */
export function pickLook(selection: Selection, lookId: string): Selection {
  return selection.lookId === lookId ? noSelection : { lookId, cape: null };
}

/** Choosing the cape the preview already shows without an explicit choice keeps it implicit. */
export function pickCape(view: SkinsView, selection: Selection, id: string | null): Selection {
  const implied = previewOf(view, { ...selection, cape: null }).cape;
  return { ...selection, cape: id === implied ? null : { id } };
}

/** The saved look is the one the account wears, and the account wears a skin worth saving. */
export function canSaveCurrent(view: SkinsView): boolean {
  const current = view.current;
  return !!current && current.custom && current.texture !== null && view.matchingLook === null;
}

/** Drop a selection that names a look that no longer exists. */
export function validSelection(view: SkinsView, selection: Selection): Selection {
  if (selection.lookId !== null && !view.library.some(look => look.id === selection.lookId)) return noSelection;
  if (selection.cape?.id && !ownsCape(view, selection.cape.id)) return { ...selection, cape: null };
  return selection;
}

/** A name for a look that is not taken yet, within the length limit. */
export function suggestedLookName(view: SkinsView): string {
  const names = new Set(view.library.map(look => look.name.toLowerCase()));
  const base = `Skin de ${view.profileName}`.slice(0, MAX_LOOK_NAME);
  if (!names.has(base.toLowerCase())) return base;
  for (let number = 2; number < 1000; number += 1) {
    const suffix = ` ${number}`;
    const candidate = `${base.slice(0, MAX_LOOK_NAME - suffix.length)}${suffix}`;
    if (!names.has(candidate.toLowerCase())) return candidate;
  }
  return base;
}

/** The player-facing reason a chosen file cannot be a skin, or null when it may be. */
export function skinFileProblem(file: { name: string; type: string; size: number }): string | null {
  if (!/\.png$/i.test(file.name) && file.type !== 'image/png') return 'Elige un archivo PNG.';
  if (file.size === 0) return 'El archivo está vacío.';
  if (file.size > MAX_SKIN_FILE_BYTES) return 'El archivo es demasiado grande para ser una skin.';
  return null;
}

export function lookNameProblem(name: string): string | null {
  const trimmed = name.trim();
  if (!trimmed) return 'Escribe un nombre.';
  if ([...trimmed].length > MAX_LOOK_NAME) return `Usa como máximo ${MAX_LOOK_NAME} caracteres.`;
  return null;
}

export function syncedText(ms: number | null, now = Date.now()): string {
  if (ms === null) return 'Aún no sincronizado';
  const minutes = Math.floor(Math.max(0, now - ms) / 60_000);
  if (minutes < 1) return 'Sincronizado hace un momento';
  if (minutes < 60) return `Sincronizado hace ${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `Sincronizado hace ${hours} h`;
  return new Intl.DateTimeFormat('es', { dateStyle: 'medium' }).format(new Date(ms));
}

const fold = (text: string) => text.normalize('NFD').replace(/[\u0300-\u036f]/g, '').toLowerCase();

/** Whether a name matches what the player typed, ignoring case and accents. An empty search matches all. */
export function matchesSearch(name: string, query: string): boolean {
  const wanted = fold(query.trim());
  return wanted === '' || fold(name).includes(wanted);
}

export const filterLooks = (library: readonly LookView[], query: string) => library.filter(look => matchesSearch(look.name, query));
export const filterCapes = (capes: readonly CapeView[], query: string) => capes.filter(cape => matchesSearch(cape.alias, query));
