import { filterAndSortMedia } from "../../lib/mediaList";
import { localizeUserMessage } from "../../lib/localizeMessage";
import type { SliceCreator, StoreSet, UiSlice } from "../types";

let toastTimer: ReturnType<typeof setTimeout> | null = null;

/** Hide a toast that is still pending its auto-dismiss (e.g. a long "scanning…" toast). */
export function dismissPendingToast(set: StoreSet) {
  if (toastTimer) {
    clearTimeout(toastTimer);
    toastTimer = null;
    set({ toastMessage: null });
  }
}

export const createUiSlice: SliceCreator<UiSlice> = (set, get) => ({
  searchQuery: "",
  sortOption: "nameAscending",
  statusFilter: "all",
  listViewMode: "poster",
  toastMessage: null,

  showToast: (message, durationMs = 2200) => {
    if (toastTimer) clearTimeout(toastTimer);
    set({ toastMessage: localizeUserMessage(message) });
    toastTimer = setTimeout(() => {
      set({ toastMessage: null });
      toastTimer = null;
    }, durationMs);
  },

  visibleMediaItems: () => {
    const { mediaItems, searchQuery, statusFilter, sortOption } = get();
    return filterAndSortMedia(mediaItems, searchQuery, statusFilter, sortOption);
  },

  setSearchQuery: (q) => set({ searchQuery: q }),
  setSortOption: (o) => set({ sortOption: o }),
  setStatusFilter: (f) => set({ statusFilter: f }),
  setListViewMode: (mode) => set({ listViewMode: mode }),
});
