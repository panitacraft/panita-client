import type { Screenshot } from './screenshot-state';

/**
 * A small least-recently-used cache of full-size images for the viewer, so stepping back
 * and forth is instant while memory stays bounded. A failed load is never remembered.
 */
export function createImageCache(load: (screenshot: Screenshot) => Promise<string>, limit = 6) {
  const entries = new Map<string, Promise<string>>();
  const key = (screenshot: Screenshot) => `${screenshot.name}|${screenshot.modifiedMs}`;
  return {
    get(screenshot: Screenshot): Promise<string> {
      const id = key(screenshot);
      let pending = entries.get(id);
      if (pending) {
        entries.delete(id);
      } else {
        pending = load(screenshot);
        pending.catch(() => { if (entries.get(id) === pending) entries.delete(id); });
      }
      entries.set(id, pending);
      while (entries.size > limit) {
        const oldest = entries.keys().next();
        if (oldest.done) break;
        entries.delete(oldest.value);
      }
      return pending;
    },
    drop(screenshot: Screenshot) { entries.delete(key(screenshot)); },
  };
}
