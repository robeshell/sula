import { invoke } from "@tauri-apps/api/core";

import { localizeUserMessage } from "../../lib/localizeMessage";
import { tt } from "../shared";
import type {
  FilesSlice,
  ResidualCandidate,
  SliceCreator,
  TaskSnapshot,
} from "../types";
import { dismissPendingToast } from "./ui";

export const createFilesSlice: SliceCreator<FilesSlice> = (set, get) => ({
  scanResidualsForSelected: async () => {
    const ids = get().selectedMediaIds;
    if (ids.length === 0) return [];
    const scraped = ids.filter((id) => {
      const item = get().mediaItems.find((m) => m.id === id);
      return item?.status === "scraped";
    });
    if (scraped.length === 0) {
      get().showToast(tt("toast.residualsOnlyScraped"));
      return [];
    }
    get().showToast(tt("toast.residualsScanning"), 60_000);
    try {
      const candidates = await invoke<ResidualCandidate[]>("scan_media_residuals", {
        itemIds: scraped,
      });
      if (candidates.length === 0) {
        get().showToast(tt("toast.residualsNone"));
      } else {
        dismissPendingToast(set);
      }
      return candidates;
    } catch (err) {
      const message = localizeUserMessage(String(err));
      set({ error: message });
      get().showToast(message);
      return [];
    }
  },

  cleanupResiduals: async (paths) => {
    if (paths.length === 0) return;
    try {
      const task = await invoke<TaskSnapshot>("cleanup_media_residuals", {
        paths,
      });
      get().upsertTask(task);
      get().showToast(tt("toast.cleanupStartedN", { n: paths.length }));
    } catch (err) {
      const message = localizeUserMessage(String(err));
      set({ error: message });
      get().showToast(message);
    }
  },

  refreshSelectedItemsFromDisk: async () => {
    const ids = get().selectedMediaIds;
    if (ids.length === 0) return;
    try {
      const task = await invoke<TaskSnapshot>("refresh_media_items", {
        itemIds: ids,
      });
      get().upsertTask(task);
      get().showToast(
        ids.length === 1
          ? tt("toast.refreshDiskStarted")
          : tt("toast.refreshDiskStartedN", { n: ids.length }),
      );
    } catch (err) {
      const message = localizeUserMessage(String(err));
      set({ error: message });
      get().showToast(message);
    }
  },

  revealSelectedItem: async () => {
    const id = get().selectedMediaId ?? get().selectedMediaIds[0];
    if (!id) return;
    const item = get().mediaItems.find((m) => m.id === id);
    if (!item) return;
    const target =
      (item.filePath && item.filePath.trim()) ||
      (item.folderPath && item.folderPath.trim()) ||
      "";
    if (!target) {
      get().showToast(tt("toast.noPath"));
      return;
    }
    try {
      await invoke("reveal_in_file_manager", { path: target });
    } catch (err) {
      get().showToast(localizeUserMessage(String(err)));
    }
  },

  deleteSelectedItems: async (alsoTrash) => {
    const ids = get().selectedMediaIds;
    if (ids.length === 0) {
      // The selection can vanish under an open confirmation (a background
      // reload removed the items); say so instead of silently doing nothing.
      get().showToast(tt("toast.deleteSelectionGone"), 3200);
      return;
    }
    try {
      const n = await invoke<number>("delete_media_items", {
        itemIds: ids,
        alsoTrash,
      });
      set({
        selectedMediaId: null,
        selectedMediaIds: [],
        detail: null,
        detailLoading: false,
        posterUrl: null,
      });
      const libraryId = get().selectedLibraryId;
      if (libraryId) void get().reloadLibraryItems(libraryId);
      get().showToast(
        alsoTrash
          ? tt("toast.deletedWithTrash", { n })
          : tt("toast.deletedRecords", { n }),
      );
    } catch (err) {
      const message = localizeUserMessage(String(err));
      set({ error: message });
      get().showToast(message);
    }
  },
});
