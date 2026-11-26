import { invoke, isTauri } from '@tauri-apps/api/core';
import type { ApplyRequest, Saved, SkinModel, SkinsView } from './skin-state';

/** Skin management exists only in the desktop app, and only for Microsoft accounts. */
export const skinsAvailable = () => isTauri();

export const loadSkins = () => invoke<SkinsView>('skins_snapshot');
export const syncSkins = () => invoke<SkinsView>('skins_sync');
export const applySkins = (request: ApplyRequest) => invoke<SkinsView>('skins_apply', { request });
export const deleteLook = (id: string) => invoke<SkinsView>('skins_delete_look', { id });
export const saveCurrentLook = (name: string) => invoke<Saved>('skins_save_current', { name });

export type LookDraft = { name: string; model: SkinModel; cape: string | null };
/** The skin file travels as base64; the backend validates it as an image before using it. */
export const addLook = (draft: LookDraft & { png: string }) => invoke<Saved>('skins_add_look', draft);
export const editLook = (id: string, draft: LookDraft & { png: string | null }) => invoke<SkinsView>('skins_edit_look', { id, ...draft });

const textures = new Map<string, Promise<string>>();
const TEXTURE_LIMIT = 96;

/**
 * A cached skin or cape as a data URL, which the content security policy allows without any
 * change. Skin keys are content hashes, so an entry never goes stale; a failed load is never kept.
 */
export function textureUrl(kind: 'skin' | 'cape', key: string): Promise<string> {
  const id = `${kind}:${key}`;
  let pending = textures.get(id);
  if (pending) {
    textures.delete(id);
  } else {
    pending = invoke<ArrayBuffer>('skin_texture', { kind, key }).then(bytes => new Promise<string>((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => typeof reader.result === 'string' ? resolve(reader.result) : reject(new Error('unreadable image'));
      reader.onerror = () => reject(reader.error ?? new Error('unreadable image'));
      reader.readAsDataURL(new Blob([bytes], { type: 'image/png' }));
    }));
    const current = pending;
    current.catch(() => { if (textures.get(id) === current) textures.delete(id); });
  }
  textures.set(id, pending);
  while (textures.size > TEXTURE_LIMIT) {
    const oldest = textures.keys().next();
    if (oldest.done) break;
    textures.delete(oldest.value);
  }
  return pending;
}

/** A file chosen by the player, as a data URL for previewing and base64 for sending. */
export function readSkinFile(file: Blob): Promise<{ dataUrl: string; base64: string }> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const dataUrl = typeof reader.result === 'string' ? reader.result : '';
      const comma = dataUrl.indexOf(',');
      if (comma < 0) reject(new Error('unreadable file'));
      else resolve({ dataUrl, base64: dataUrl.slice(comma + 1) });
    };
    reader.onerror = () => reject(reader.error ?? new Error('unreadable file'));
    reader.readAsDataURL(file);
  });
}
