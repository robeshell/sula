import { invoke } from "@tauri-apps/api/core";

import { localizeUserMessage } from "../../lib/localizeMessage";
import { tt } from "../shared";
import type {
  AppConfig,
  ScrapeSlice,
  SliceCreator,
  StoreGet,
  TaskSnapshot,
} from "../types";

async function ensureScrapeKeys(get: StoreGet): Promise<boolean> {
  try {
    const config = await invoke<AppConfig>("get_config");
    const hasKey =
      Boolean(config.apiKeys.tmdb.trim()) ||
      Boolean(config.apiKeys.bangumi.trim()) ||
      Boolean(config.apiKeys.omdb.trim()) ||
      Boolean(config.apiKeys.tvdb.trim());
    if (!hasKey) {
      get().showToast(tt("toast.needApiKeys"));
      return false;
    }
    return true;
  } catch (err) {
    get().showToast(localizeUserMessage(String(err)));
    return false;
  }
}

export const createScrapeSlice: SliceCreator<ScrapeSlice> = (set, get) => ({
  scrapeSelectedLibrary: async () => {
    const id = get().selectedLibraryId;
    if (!id) return;
    if (!(await ensureScrapeKeys(get))) return;
    try {
      const task = await invoke<TaskSnapshot>("scrape_library", {
        libraryId: id,
      });
      get().upsertTask(task);
      get().showToast(tt("toast.batchScrapeStarted"));
    } catch (err) {
      const message = localizeUserMessage(String(err));
      set({ error: message });
      get().showToast(message);
    }
  },

  scrapeSelectedItems: async () => {
    const ids = get().selectedMediaIds;
    if (ids.length === 0) return;
    if (!(await ensureScrapeKeys(get))) return;
    try {
      const task = await invoke<TaskSnapshot>("scrape_items", {
        itemIds: ids,
      });
      get().upsertTask(task);
      get().showToast(
        ids.length === 1
          ? tt("toast.scrapeStarted")
          : tt("toast.scrapeStartedN", { n: ids.length }),
      );
    } catch (err) {
      const message = localizeUserMessage(String(err));
      set({ error: message });
      get().showToast(message);
    }
  },

  rescrapeSelectedItems: async () => {
    const ids = get().selectedMediaIds;
    if (ids.length === 0) return;
    const scraped = ids.filter((id) => {
      const item = get().mediaItems.find((m) => m.id === id);
      return item?.status === "scraped";
    });
    if (scraped.length === 0) {
      get().showToast(tt("toast.rescrapeOnlyScraped"));
      return;
    }
    if (!(await ensureScrapeKeys(get))) return;
    try {
      const task = await invoke<TaskSnapshot>("rescrape_items", {
        itemIds: scraped,
      });
      get().upsertTask(task);
      get().showToast(
        scraped.length === 1
          ? tt("toast.rescrapeStarted")
          : tt("toast.rescrapeStartedN", { n: scraped.length }),
      );
    } catch (err) {
      const message = localizeUserMessage(String(err));
      set({ error: message });
      get().showToast(message);
    }
  },

  scrapeSelectedItem: async () => {
    await get().scrapeSelectedItems();
  },

  scrapeSeason: async (mediaItemId, seasonNumber) => {
    if (!(await ensureScrapeKeys(get))) return;
    get().showToast(tt("toast.scrapeSeasonStarted", { n: seasonNumber }), 60_000);
    try {
      const task = await invoke<TaskSnapshot>("scrape_season", { mediaItemId, seasonNumber });
      get().upsertTask(task);
      get().showToast(tt("toast.scrapeSeasonStarted", { n: seasonNumber }), 2800);
    } catch (err) {
      const message = localizeUserMessage(String(err));
      set({ error: message });
      get().showToast(message);
    }
  },

  ensureScrapeReady: async () => ensureScrapeKeys(get),
});
