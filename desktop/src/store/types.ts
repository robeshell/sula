import type { StateCreator } from "zustand";

import type { MediaSortOption, MediaStatusFilter } from "../lib/mediaList";

export type AppConfig = {
  scrapeConcurrency: number;
  metadataLanguage: string;
  nfoFormat: string;
  scanExcludedFolders: string[];
  renameAutoAfterScrape: boolean;
  renameCreateSeasonFolders: boolean;
  renameMovieFolderTemplate: string;
  renameMovieFileTemplate: string;
  renameTvShowFolderTemplate: string;
  renameSeasonFolderTemplate: string;
  renameEpisodeFileTemplate: string;
  appearance: string;
  accent: string;
  trayEnabled: boolean;
  keepRunningOnClose: boolean;
  uiLocale: string;
  /** Saved keys are still being read from the system keychain. */
  apiKeysLoading?: boolean;
  apiKeys: {
    tmdb: string;
    tvdb: string;
    omdb: string;
    bangumi: string;
  };
};

export type AppStatus = {
  appName: string;
  version: string;
  dataDir: string;
  databasePath: string;
  libraryCount: number;
  config: AppConfig;
  crates: {
    mediaCore: string;
    scraperKit: string;
    renamer: string;
  };
};

export type MediaType = "movie" | "tvShow" | "anime";

export type Library = {
  id: string;
  name: string;
  rootPath: string;
  mediaType: MediaType;
  addedAt: string;
};

export type MediaItem = {
  id: string;
  mediaType: MediaType;
  title: string;
  originalTitle?: string | null;
  year?: number | null;
  folderPath: string;
  filePath: string;
  status: string;
  libraryId: string;
  addedAt: string;
};

export type MediaMetaSummary = {
  mediaItemId: string;
  posterPath?: string | null;
  fanartPath?: string | null;
  overview?: string | null;
  rating?: number | null;
  genres: string[];
  scrapedAt?: string;
};

export type ShowListStats = {
  mediaItemId: string;
  seasonCount: number;
  episodeCount: number;
  localEpisodeCount: number;
};

export type CastMember = {
  name: string;
  role?: string | null;
  type?: string | null;
  thumbUrl?: string | null;
  order?: number | null;
};

export type MediaMetadata = {
  mediaItemId: string;
  overview?: string | null;
  tagline?: string | null;
  genres: string[];
  rating?: number | null;
  ratingVotes?: number | null;
  contentRating?: string | null;
  director?: string | null;
  runtime?: number | null;
  showStatus?: string | null;
  posterPath?: string | null;
  fanartPath?: string | null;
  sourceId: string;
  credits?: CastMember[];
};

export type TvSeason = {
  id: string;
  mediaItemId: string;
  seasonNumber: number;
  title?: string | null;
  overview?: string | null;
  posterPath?: string | null;
  airDate?: string | null;
  episodeCount?: number | null;
};

export type TvEpisode = {
  id: string;
  seasonId: string;
  episodeNumber: number;
  title?: string | null;
  overview?: string | null;
  airDate?: string | null;
  stillPath?: string | null;
  filePath: string;
  runtime?: number | null;
  rating?: number | null;
  director?: string | null;
};

export type MediaDetail = {
  item: MediaItem;
  metadata?: MediaMetadata | null;
  seasons: TvSeason[];
  episodes: TvEpisode[];
};

export type TaskSnapshot = {
  id: string;
  title: string;
  kind: string;
  status: string;
  progress?: {
    completed: number;
    total: number;
    current: string;
    stageKey?: string;
  } | null;
  errorMessage?: string | null;
  result?: { success: number; unmatched: number; failed: number } | null;
  targetId?: string | null;
  createdAt: string;
  updatedAt: string;
};

export type ResidualCandidate = {
  path: string;
  itemId: string;
  itemTitle: string;
  reason: string;
  size: number;
};

// ---- Store slices -------------------------------------------------------

export type LibrarySlice = {
  status: AppStatus | null;
  libraries: Library[];
  selectedLibraryId: string | null;
  mediaItems: MediaItem[];
  metadataById: Record<string, MediaMetaSummary>;
  showStatsById: Record<string, ShowListStats>;
  loading: boolean;
  error: string | null;
  refreshStatus: () => Promise<void>;
  refreshLibraries: () => Promise<void>;
  /** User-initiated library switch: clears the list, selection and detail. */
  selectLibrary: (id: string | null) => Promise<void>;
  /**
   * Background refresh of the current library (task finished, library-updated).
   * Keeps the list, selection and detail; coalesces bursts into one reload.
   * `posters: "all"` re-resolves every poster; rescrapes need not use it because
   * metadata fingerprints include `scrapedAt`.
   */
  reloadLibraryItems: (
    libraryId: string,
    opts?: { posters?: "changed" | "all" },
  ) => Promise<void>;
  addLibrary: (mediaType: MediaType) => Promise<void>;
  deleteSelectedLibrary: () => Promise<void>;
  refreshSelectedLibrary: () => Promise<void>;
};

export type SelectionSlice = {
  selectedMediaId: string | null;
  selectedMediaIds: string[];
  detail: MediaDetail | null;
  detailLoading: boolean;
  posterUrl: string | null;
  selectMedia: (id: string | null) => Promise<void>;
  toggleMediaSelection: (id: string, additive: boolean) => Promise<void>;
  clearMediaSelection: () => void;
};

export type UiSlice = {
  searchQuery: string;
  sortOption: MediaSortOption;
  statusFilter: MediaStatusFilter;
  listViewMode: "list" | "poster";
  toastMessage: string | null;
  showToast: (message: string, durationMs?: number) => void;
  setSearchQuery: (q: string) => void;
  setSortOption: (o: MediaSortOption) => void;
  setStatusFilter: (f: MediaStatusFilter) => void;
  setListViewMode: (mode: "list" | "poster") => void;
  visibleMediaItems: () => MediaItem[];
};

export type TasksSlice = {
  tasks: TaskSnapshot[];
  refreshTasks: () => Promise<void>;
  upsertTask: (task: TaskSnapshot) => void;
};

export type ScrapeSlice = {
  scrapeSelectedLibrary: () => Promise<void>;
  /** Auto-scrape selected items (multi-select / context menu). Single-select UI opens manual match instead. */
  scrapeSelectedItems: () => Promise<void>;
  /** Rescrape selected scraped items (overwrite metadata). */
  rescrapeSelectedItems: () => Promise<void>;
  scrapeSelectedItem: () => Promise<void>;
  /** Scrape one season's metadata from TMDB. */
  scrapeSeason: (mediaItemId: string, seasonNumber: number) => Promise<void>;
  ensureScrapeReady: () => Promise<boolean>;
};

export type OrganizeSlice = {
  renameSelectedItem: () => Promise<void>;
  renameSelectedItems: () => Promise<void>;
  organizeSelectedItems: () => Promise<void>;
  /** Merge selected TV/anime duplicates into the canonical show. */
  consolidateSelectedShows: () => Promise<void>;
  /** Scan the whole TV/anime library for duplicate shows and merge them. */
  consolidateSelectedLibraryShows: () => Promise<void>;
};

export type FilesSlice = {
  /** Dry-run residual scan for scraped selection; returns candidates (may be empty). */
  scanResidualsForSelected: () => Promise<ResidualCandidate[]>;
  cleanupResiduals: (paths: string[]) => Promise<void>;
  /** SCAN-15: refresh selected items from disk (missing primary → delete). */
  refreshSelectedItemsFromDisk: () => Promise<void>;
  /** MAINT-07: reveal item in OS file manager. */
  revealSelectedItem: () => Promise<void>;
  deleteSelectedItems: (alsoTrash: boolean) => Promise<void>;
};

export type AppStore = LibrarySlice &
  SelectionSlice &
  UiSlice &
  TasksSlice &
  ScrapeSlice &
  OrganizeSlice &
  FilesSlice;

export type StoreGet = () => AppStore;
export type StoreSet = (partial: Partial<AppStore>) => void;

/** Each slice sees the whole store through `set`/`get`. */
export type SliceCreator<T> = StateCreator<AppStore, [], [], T>;
