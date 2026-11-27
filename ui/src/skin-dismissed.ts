// Which unsaved skins the player asked not to be reminded about. It is a preference of this window,
// so it lives in local storage like the motion preference does, and losing it only brings a reminder
// back. The key names a skin and its arm model; a different unsaved skin asks again.
const STORAGE_KEY = 'panita.skins.dismissed';
const LIMIT = 50;

export function loadDismissed(): Set<string> {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '[]');
    return new Set(Array.isArray(parsed) ? parsed.filter((item): item is string => typeof item === 'string').slice(-LIMIT) : []);
  } catch {
    return new Set();
  }
}

/** The set with `key` added, oldest entries dropped past the limit, and remembered. */
export function dismissUnsaved(current: ReadonlySet<string>, key: string): Set<string> {
  const next = new Set([...current].filter(item => item !== key));
  next.add(key);
  const kept = new Set([...next].slice(-LIMIT));
  try { localStorage.setItem(STORAGE_KEY, JSON.stringify([...kept])); } catch { /* the reminder then returns after a restart */ }
  return kept;
}
