import i18n from "../i18n";
import type { MediaItem } from "./types";

export function tt(key: string, opts?: Record<string, unknown>): string {
  return i18n.t(key, opts);
}

export function itemTitles(items: MediaItem[], ids: string[]): string[] {
  const byId = new Map(items.map((item) => [item.id, item]));
  return ids.map((id) => {
    const item = byId.get(id);
    return item ? (item.year ? `${item.title} (${item.year})` : item.title) : id;
  });
}

export function folderName(path: string): string {
  return path.split(/[/\\]/).filter(Boolean).pop() ?? path;
}

/**
 * Monotonic request tokens shared across slices: a response is applied only if
 * its token is still the latest. Library switches also bump `detail` so an
 * in-flight detail fetch for the old library is dropped.
 */
export const requestSeq = { library: 0, detail: 0 };
