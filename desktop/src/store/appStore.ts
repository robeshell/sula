import { create } from "zustand";

import { createFilesSlice } from "./slices/files";
import { createLibrarySlice } from "./slices/library";
import { createOrganizeSlice } from "./slices/organize";
import { createScrapeSlice } from "./slices/scrape";
import { createSelectionSlice } from "./slices/selection";
import { createTasksSlice } from "./slices/tasks";
import { createUiSlice } from "./slices/ui";
import type { AppStore } from "./types";

export type {
  AppConfig,
  AppStatus,
  CastMember,
  Library,
  MediaDetail,
  MediaItem,
  MediaMetadata,
  MediaMetaSummary,
  MediaType,
  ResidualCandidate,
  ShowListStats,
  TaskSnapshot,
  TvEpisode,
  TvSeason,
} from "./types";

/**
 * Single app store composed from slices. Each slice's actions see the whole
 * store through `set`/`get`, so cross-slice reads and writes stay direct.
 */
export const useAppStore = create<AppStore>()((...a) => ({
  ...createLibrarySlice(...a),
  ...createSelectionSlice(...a),
  ...createUiSlice(...a),
  ...createTasksSlice(...a),
  ...createScrapeSlice(...a),
  ...createOrganizeSlice(...a),
  ...createFilesSlice(...a),
}));
