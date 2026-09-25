import { invoke } from "@tauri-apps/api/core";

import { confirmAction } from "../../lib/confirmation";
import { localizeUserMessage } from "../../lib/localizeMessage";
import { folderName, itemTitles, tt } from "../shared";
import type {
  OrganizeSlice,
  SliceCreator,
  StoreGet,
  TaskSnapshot,
} from "../types";

type ShowMergePlan = {
  sourceId: string;
  sourceTitle: string;
  sourceFolder: string;
  targetId: string;
  targetTitle: string;
  targetFolder: string;
};

/** Preview duplicate-show merges, ask once with the concrete list, then run
 * exactly the confirmed pairs (the backend re-checks each one). */
async function confirmAndMergeShows(
  get: StoreGet,
  scope: { libraryId: string } | { itemIds: string[] },
) {
  const plan = await invoke<ShowMergePlan[]>("plan_show_merges", scope);
  if (plan.length === 0) {
    get().showToast(tt("toast.mergedNone"));
    return;
  }
  const confirmed = await confirmAction({
    title: tt("confirm.mergeTitle"),
    description: tt("confirm.mergeDescription", { n: plan.length }),
    details: plan.map((p) => `${folderName(p.sourceFolder)} → ${folderName(p.targetFolder)}`),
    confirmLabel: tt("confirm.mergeAction"),
  });
  if (!confirmed) return;
  const merged = await invoke<number>("merge_planned_shows", {
    pairs: plan.map((p) => ({ sourceId: p.sourceId, targetId: p.targetId })),
  });
  if (merged === plan.length) {
    get().showToast(tt("toast.mergedShows", { n: merged }), 3200);
  } else {
    get().showToast(tt("toast.mergedPartial", { n: merged, skipped: plan.length - merged }), 4200);
  }
}

export const createOrganizeSlice: SliceCreator<OrganizeSlice> = (set, get) => ({
  renameSelectedItem: async () => {
    await get().renameSelectedItems();
  },

  renameSelectedItems: async () => {
    const ids = get().selectedMediaIds;
    if (ids.length === 0) return;
    const scraped = ids.filter((id) => {
      const item = get().mediaItems.find((m) => m.id === id);
      return item?.status === "scraped";
    });
    if (scraped.length === 0) {
      get().showToast(tt("toast.renameOnlyScraped"));
      return;
    }
    const confirmed = await confirmAction({
      title: tt("confirm.renameTitle"),
      description: tt("confirm.renameDescription", { n: scraped.length }),
      details: itemTitles(get().mediaItems, scraped),
      confirmLabel: tt("confirm.renameAction"),
    });
    if (!confirmed) return;
    try {
      const task = await invoke<TaskSnapshot>("apply_rename_templates", {
        itemIds: scraped,
      });
      get().upsertTask(task);
      get().showToast(tt("toast.renameStartedN", { n: scraped.length }));
    } catch (err) {
      const message = localizeUserMessage(String(err));
      set({ error: message });
      get().showToast(message);
    }
  },

  organizeSelectedItems: async () => {
    const ids = get().selectedMediaIds;
    if (ids.length === 0) return;
    const targets = ids.filter((id) => {
      const item = get().mediaItems.find((m) => m.id === id);
      return (
        item?.status === "scraped" &&
        (item.mediaType === "tvShow" || item.mediaType === "anime")
      );
    });
    if (targets.length === 0) {
      get().showToast(tt("toast.organizeOnlyTvAnime"));
      return;
    }
    const confirmed = await confirmAction({
      title: tt("confirm.organizeTitle"),
      description: tt("confirm.organizeDescription", { n: targets.length }),
      details: itemTitles(get().mediaItems, targets),
      confirmLabel: tt("confirm.organizeAction"),
    });
    if (!confirmed) return;
    try {
      const task = await invoke<TaskSnapshot>("organize_season_folders", {
        itemIds: targets,
      });
      get().upsertTask(task);
      get().showToast(tt("toast.organizeStartedN", { n: targets.length }));
    } catch (err) {
      const message = localizeUserMessage(String(err));
      set({ error: message });
      get().showToast(message);
    }
  },

  consolidateSelectedShows: async () => {
    const ids = get().selectedMediaIds;
    if (ids.length === 0) return;
    const targets = ids.filter((id) => {
      const item = get().mediaItems.find((m) => m.id === id);
      return item?.mediaType === "tvShow" || item?.mediaType === "anime";
    });
    if (targets.length === 0) {
      get().showToast(tt("toast.mergeOnlyTvAnime"));
      return;
    }
    try {
      await confirmAndMergeShows(get, { itemIds: targets });
    } catch (err) {
      const message = localizeUserMessage(String(err));
      set({ error: message });
      get().showToast(message);
    }
  },

  consolidateSelectedLibraryShows: async () => {
    const libraryId = get().selectedLibraryId;
    if (!libraryId) return;
    const lib = get().libraries.find((l) => l.id === libraryId);
    if (!lib || (lib.mediaType !== "tvShow" && lib.mediaType !== "anime")) {
      get().showToast(tt("toast.mergeOnlyTvAnime"));
      return;
    }
    try {
      await confirmAndMergeShows(get, { libraryId });
    } catch (err) {
      const message = localizeUserMessage(String(err));
      set({ error: message });
      get().showToast(message);
    }
  },
});
