import { invoke } from "@tauri-apps/api/core";

import { POSTER_THUMB, resolvePosterSrc } from "../../lib/posterLoadQueue";
import { requestSeq } from "../shared";
import type {
  MediaDetail,
  SelectionSlice,
  SliceCreator,
  StoreGet,
  StoreSet,
} from "../types";

export const createSelectionSlice: SliceCreator<SelectionSlice> = (set, get) => ({
  selectedMediaId: null,
  selectedMediaIds: [],
  detail: null,
  detailLoading: false,
  posterUrl: null,

  selectMedia: async (id) => {
    const request = ++requestSeq.detail;
    set({
      selectedMediaId: id,
      selectedMediaIds: id ? [id] : [],
      detail: null,
      detailLoading: Boolean(id),
      posterUrl: null,
    });
    if (!id) return;

    const itemHint = get().mediaItems.find((m) => m.id === id);
    const posterHint = get().metadataById[id]?.posterPath;
    const posterPromise =
      itemHint && posterHint
        ? resolvePosterSrc({
            folderPath: itemHint.folderPath,
            posterPath: posterHint,
            width: POSTER_THUMB.width,
            height: POSTER_THUMB.height,
          })
            .then((url) => {
              if (request !== requestSeq.detail || get().selectedMediaId !== id) return;
              set({ posterUrl: url });
            })
            .catch(() => {
              /* detail path below surfaces errors if needed */
            })
        : Promise.resolve();

    try {
      const [detail] = await Promise.all([
        invoke<MediaDetail>("get_media_detail", { id }),
        posterPromise,
      ]);
      if (request !== requestSeq.detail || get().selectedMediaId !== id) return;
      set({ detail, detailLoading: false });
      // If list metadata had no posterPath, resolve from detail.
      if (!posterHint) {
        const posterPath = detail.metadata?.posterPath;
        if (posterPath) {
          const url = await resolvePosterSrc({
            folderPath: detail.item.folderPath,
            posterPath,
            width: POSTER_THUMB.width,
            height: POSTER_THUMB.height,
          });
          if (request !== requestSeq.detail || get().selectedMediaId !== id) return;
          set({ posterUrl: url });
        }
      }
    } catch (err) {
      if (request !== requestSeq.detail || get().selectedMediaId !== id) return;
      const message = String(err);
      set({ error: message, detailLoading: false });
      get().showToast(message);
    }
  },

  toggleMediaSelection: async (id, additive) => {
    if (!additive) {
      await get().selectMedia(id);
      return;
    }
    // Additive multi-select: update selection only — never open/fetch detail.
    const prev = get().selectedMediaIds;
    const current = get().selectedMediaId;
    const seed = prev.length === 0 && current ? [current] : prev;
    const next = seed.includes(id)
      ? seed.filter((x) => x !== id)
      : [...seed, id];
    set({
      selectedMediaIds: next,
      selectedMediaId: null,
      detail: null,
      detailLoading: false,
      posterUrl: null,
    });
  },

  clearMediaSelection: () => {
    set({
      selectedMediaId: null,
      selectedMediaIds: [],
      detail: null,
      detailLoading: false,
      posterUrl: null,
    });
  },
});

/** Re-fetch the open detail after a background reload without a loading flash. */
export async function refreshOpenDetail(get: StoreGet, set: StoreSet, id: string) {
  const request = ++requestSeq.detail;
  const current = () => request === requestSeq.detail && get().selectedMediaId === id;
  try {
    const detail = await invoke<MediaDetail>("get_media_detail", { id });
    if (!current()) return;
    set({ detail, detailLoading: false });
    const posterPath = get().metadataById[id]?.posterPath ?? detail.metadata?.posterPath;
    if (!posterPath) return;
    const url = await resolvePosterSrc({
      folderPath: detail.item.folderPath,
      posterPath,
      width: POSTER_THUMB.width,
      height: POSTER_THUMB.height,
    });
    if (current() && url) set({ posterUrl: url });
  } catch {
    // Keep showing the previous detail; the next explicit selection surfaces errors.
    if (current()) set({ detailLoading: false });
  }
}
