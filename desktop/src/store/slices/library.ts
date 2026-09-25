import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

import { confirmAction } from "../../lib/confirmation";
import { localizeUserMessage } from "../../lib/localizeMessage";
import {
  invalidatePosterCache,
  invalidatePosterFolders,
} from "../../lib/posterLoadQueue";
import { scheduleReload } from "../reloadQueue";
import { requestSeq, tt } from "../shared";
import type {
  AppStatus,
  Library,
  LibrarySlice,
  MediaItem,
  MediaMetaSummary,
  ShowListStats,
  SliceCreator,
  TaskSnapshot,
} from "../types";
import { refreshOpenDetail } from "./selection";

type MediaPage = {
  items: MediaItem[];
  metadata: MediaMetaSummary[];
  showStats: ShowListStats[];
  nextOffset?: number | null;
};

function metaFingerprint(meta: MediaMetaSummary | undefined): string {
  return meta ? JSON.stringify(meta) : "";
}

export const createLibrarySlice: SliceCreator<LibrarySlice> = (set, get) => ({
  status: null,
  libraries: [],
  selectedLibraryId: null,
  mediaItems: [],
  metadataById: {},
  showStatsById: {},
  loading: false,
  error: null,

  refreshStatus: async () => {
    set({ loading: true, error: null });
    try {
      const status = await invoke<AppStatus>("app_status");
      set({ status, loading: false });
    } catch (err) {
      const message = String(err);
      set({ loading: false, error: message });
      get().showToast(message);
    }
  },

  refreshLibraries: async () => {
    try {
      const libraries = await invoke<Library[]>("list_libraries");
      const selected = get().selectedLibraryId;
      const nextSelected =
        selected && libraries.some((l) => l.id === selected)
          ? selected
          : libraries[0]?.id ?? null;
      set({ libraries });
      // Item changes arrive via reloadLibraryItems (task completion /
      // library-updated); only a changed selection needs a fresh load here.
      if (nextSelected && nextSelected !== selected) {
        await get().selectLibrary(nextSelected);
      } else if (!nextSelected) {
        set({
          selectedLibraryId: null,
          mediaItems: [],
          metadataById: {},
          showStatsById: {},
          selectedMediaId: null,
          selectedMediaIds: [],
          detail: null,
          detailLoading: false,
          posterUrl: null,
        });
      }
    } catch (err) {
      const message = String(err);
      set({ error: message });
      get().showToast(message);
    }
  },

  selectLibrary: async (id) => {
    const request = ++requestSeq.library;
    ++requestSeq.detail;
    set({
      selectedLibraryId: id,
      mediaItems: [],
      metadataById: {},
      showStatsById: {},
      selectedMediaId: null,
      selectedMediaIds: [],
      detail: null,
      detailLoading: false,
      posterUrl: null,
    });
    if (!id) {
      set({ mediaItems: [], metadataById: {}, showStatsById: {} });
      return;
    }
    try {
      let offset: number | null = 0;
      const items = new Map<string, MediaItem>();
      const metadataById: Record<string, MediaMetaSummary> = {};
      const showStatsById: Record<string, ShowListStats> = {};
      while (offset !== null) {
        const page: MediaPage = await invoke("list_media_page", { libraryId: id, offset, limit: 256 });
        if (request !== requestSeq.library || get().selectedLibraryId !== id) return;
        for (const item of page.items) items.set(item.id, item);
        for (const meta of page.metadata) metadataById[meta.mediaItemId] = meta;
        for (const stats of page.showStats ?? []) showStatsById[stats.mediaItemId] = stats;
        set({ mediaItems: [...items.values()], metadataById: {...metadataById}, showStatsById: {...showStatsById} });
        const next: number | null = page.nextOffset ?? null;
        if (next !== null && next <= offset) throw new Error("invalid media page cursor");
        offset = next;
      }
    } catch (err) {
      if (request !== requestSeq.library || get().selectedLibraryId !== id) return;
      const message = String(err);
      set({ error: message });
      get().showToast(message);
    }
  },

  reloadLibraryItems: (libraryId, opts) =>
    scheduleReload(
      async ({ libraryId: id, posters }) => {
        if (get().selectedLibraryId !== id) return;
        const request = ++requestSeq.library;
        const stale = () => request !== requestSeq.library || get().selectedLibraryId !== id;
        const items = new Map<string, MediaItem>();
        const metadataById: Record<string, MediaMetaSummary> = {};
        const showStatsById: Record<string, ShowListStats> = {};
        try {
          let offset: number | null = 0;
          while (offset !== null) {
            const page: MediaPage = await invoke("list_media_page", { libraryId: id, offset, limit: 256 });
            if (stale()) return;
            for (const item of page.items) items.set(item.id, item);
            for (const meta of page.metadata) metadataById[meta.mediaItemId] = meta;
            for (const stats of page.showStats ?? []) showStatsById[stats.mediaItemId] = stats;
            const next: number | null = page.nextOffset ?? null;
            if (next !== null && next <= offset) throw new Error("invalid media page cursor");
            offset = next;
          }
        } catch (err) {
          if (stale()) return;
          const message = String(err);
          set({ error: message });
          get().showToast(message);
          return;
        }

        const prev = get();
        if (posters === "all") {
          invalidatePosterCache();
        } else {
          const changed = new Set<string>();
          const oldById = new Map(prev.mediaItems.map((item) => [item.id, item]));
          for (const item of items.values()) {
            const old = oldById.get(item.id);
            if (!old) continue;
            if (
              old.folderPath !== item.folderPath ||
              old.status !== item.status ||
              metaFingerprint(prev.metadataById[item.id]) !== metaFingerprint(metadataById[item.id])
            ) {
              changed.add(old.folderPath);
              changed.add(item.folderPath);
            }
          }
          invalidatePosterFolders(changed);
        }

        const selectedIds = prev.selectedMediaIds.filter((x) => items.has(x));
        const keepDetail = prev.selectedMediaId !== null && items.has(prev.selectedMediaId);
        set({
          mediaItems: [...items.values()],
          metadataById,
          showStatsById,
          selectedMediaIds: selectedIds,
          ...(keepDetail
            ? {}
            : prev.selectedMediaId !== null
              ? { selectedMediaId: null, detail: null, detailLoading: false, posterUrl: null }
              : {}),
        });
        // Not awaited: a slow detail fetch must not hold up the next list reload.
        if (keepDetail) void refreshOpenDetail(get, set, prev.selectedMediaId!);
      },
      libraryId,
      opts?.posters ?? "changed",
    ),

  addLibrary: async (mediaType) => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: tt("toast.pickLibraryRoot"),
      });
      const rootPath = Array.isArray(selected) ? selected[0] : selected;
      if (!rootPath) {
        return;
      }
      const name =
        rootPath.split(/[/\\]/).filter(Boolean).pop() ??
        tt("toast.libraryDefaultName");
      await invoke<Library>("add_library", {
        name,
        rootPath,
        mediaType,
      });
      // Load tasks first so the empty list shows "scanning" instead of a Refresh CTA.
      await get().refreshTasks();
      await get().refreshLibraries();
      await get().refreshStatus();
      get().showToast(tt("toast.libraryAdded", { name }));
    } catch (err) {
      const message = localizeUserMessage(String(err));
      set({ error: message });
      get().showToast(message);
    }
  },

  deleteSelectedLibrary: async () => {
    const id = get().selectedLibraryId;
    if (!id) return;
    const lib = get().libraries.find((l) => l.id === id);
    if (!lib) return;
    if (!(await confirmAction({title: tt("action.deleteLibrary"), description: tt("toast.libraryDeleteConfirm", { name: lib.name })}))) {
      return;
    }
    try {
      await invoke("delete_library", { id });
      set({
        selectedLibraryId: null,
        mediaItems: [],
        selectedMediaId: null,
        detail: null,
        detailLoading: false,
      });
      await get().refreshLibraries();
      await get().refreshStatus();
      get().showToast(tt("toast.libraryDeleted", { name: lib.name }));
    } catch (err) {
      const message = localizeUserMessage(String(err));
      set({ error: message });
      get().showToast(message);
    }
  },

  refreshSelectedLibrary: async () => {
    const id = get().selectedLibraryId;
    if (!id) return;
    const busy = get().tasks.some(
      (t) =>
        t.kind === "refresh" &&
        t.targetId === id &&
        (t.status === "pending" || t.status === "running"),
    );
    if (busy) {
      get().showToast(tt("toast.libraryScanning"));
      return;
    }
    try {
      const task = await invoke<TaskSnapshot>("refresh_library", {
        libraryId: id,
      });
      get().upsertTask(task);
      get().showToast(tt("toast.refreshStarted"));
    } catch (err) {
      const message = localizeUserMessage(String(err));
      set({ error: message });
      get().showToast(message);
    }
  },
});
