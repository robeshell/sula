const STORAGE_KEY = "sula.detailPanelWidth";

/** Detail panel width bounds (px). */
export const DETAIL_WIDTH_MIN = 280;
export const DETAIL_WIDTH_MAX = 560;
export const DETAIL_WIDTH_DEFAULT = 352; // ~22rem

export function clampDetailWidth(px: number): number {
  return Math.min(DETAIL_WIDTH_MAX, Math.max(DETAIL_WIDTH_MIN, Math.round(px)));
}

export function loadDetailWidth(): number {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return DETAIL_WIDTH_DEFAULT;
    const n = Number(raw);
    if (!Number.isFinite(n)) return DETAIL_WIDTH_DEFAULT;
    return clampDetailWidth(n);
  } catch {
    return DETAIL_WIDTH_DEFAULT;
  }
}

export function saveDetailWidth(px: number) {
  try {
    localStorage.setItem(STORAGE_KEY, String(clampDetailWidth(px)));
  } catch {
    /* ignore quota / private mode */
  }
}
