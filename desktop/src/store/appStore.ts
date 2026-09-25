import { confirmAction } from "../lib/confirmation";
import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

import i18n from "../i18n";
import {
  filterAndSortMedia,
  type MediaSortOption,
  type MediaStatusFilter,
} from "../lib/mediaList";
import { localizeUserMessage } from "../lib/localizeMessage";
import { notifyTaskDone } from "../lib/notify";
import {
  POSTER_THUMB,
  resolvePosterSrc,
  invalidatePosterCache,
  invalidatePosterFolders,
} from "../lib/posterLoadQueue";

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
  apiKeys: {
    tmdb: string;
    tvdb: string;
    omdb: string;
    bangumi: string;
  };
};

function tt(key: string, opts?: Record<string, unknown>): string {
  return i18n.t(key, opts);
}

function itemTitles(items: MediaItem[], ids: string[]): string[] {
  const byId = new Map(items.map((item) => [item.id, item]));
  return ids.map((id) => {
    const item = byId.get(id);
    return item ? (item.year ? `${item.title} (${item.year})` : item.title) : id;
  });
}

function folderName(path: string): string {
  return path.split(/[/\\]/).filter(Boolean).pop() ?? path;
}

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
  get: () => AppStore,
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

function scrapeUnmatchedCount(summary: string): number {
  const m =
    summary.match(/(?:^|\s)unmatched=(\d+)/) ||
    summary.match(/未自动匹配\s+(\d+)/) ||
    summary.match(/未自動マッチ\s+(\d+)/) ||
    summary.match(/\bunmatched\s+(\d+)/i);
  return m ? Number(m[1]) : 0;
}

function localizeScrapeSummary(summary: string): string {
  const m = summary.match(/^success=(\d+) unmatched=(\d+) failed=(\d+)$/);
  if (m) {
    return tt("toast.scrapeSummary", {
      success: m[1],
      unmatched: m[2],
      failed: m[3],
    });
  }
  return summary;
}

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

type AppStore = {
  status: AppStatus | null;
  libraries: Library[];
  selectedLibraryId: string | null;
  mediaItems: MediaItem[];
  metadataById: Record<string, MediaMetaSummary>;
  showStatsById: Record<string, ShowListStats>;
  selectedMediaId: string | null;
  selectedMediaIds: string[];
  detail: MediaDetail | null;
  detailLoading: boolean;
  posterUrl: string | null;
  searchQuery: string;
  sortOption: MediaSortOption;
  statusFilter: MediaStatusFilter;
  listViewMode: "list" | "poster";
  tasks: TaskSnapshot[];
  loading: boolean;
  error: string | null;
  toastMessage: string | null;
  showToast: (message: string, durationMs?: number) => void;
  refreshStatus: () => Promise<void>;
  refreshLibraries: () => Promise<void>;
  /** User-initiated library switch: clears the list, selection and detail. */
  selectLibrary: (id: string | null) => Promise<void>;
  /**
   * Background refresh of the current library (task finished, library-updated).
   * Keeps the list, selection and detail; coalesces bursts into one reload.
   * `posters: "all"` re-resolves every poster (e.g. after a rescrape).
   */
  reloadLibraryItems: (
    libraryId: string,
    opts?: { posters?: "changed" | "all" },
  ) => Promise<void>;
  selectMedia: (id: string | null) => Promise<void>;
  toggleMediaSelection: (id: string, additive: boolean) => Promise<void>;
  clearMediaSelection: () => void;
  setSearchQuery: (q: string) => void;
  setSortOption: (o: MediaSortOption) => void;
  setStatusFilter: (f: MediaStatusFilter) => void;
  setListViewMode: (mode: "list" | "poster") => void;
  addLibrary: (mediaType: MediaType) => Promise<void>;
  deleteSelectedLibrary: () => Promise<void>;
  refreshSelectedLibrary: () => Promise<void>;
  scrapeSelectedLibrary: () => Promise<void>;
  /** Auto-scrape selected items (multi-select / context menu). Single-select UI opens manual match instead. */
  scrapeSelectedItems: () => Promise<void>;
  /** Rescrape selected scraped items (overwrite metadata). */
  rescrapeSelectedItems: () => Promise<void>;
  scrapeSelectedItem: () => Promise<void>;
  /** Scrape one season's metadata from TMDB. */
  scrapeSeason: (mediaItemId: string, seasonNumber: number) => Promise<void>;
  ensureScrapeReady: () => Promise<boolean>;
  renameSelectedItem: () => Promise<void>;
  renameSelectedItems: () => Promise<void>;
  organizeSelectedItems: () => Promise<void>;
  /** Merge selected TV/anime duplicates into the canonical show. */
  consolidateSelectedShows: () => Promise<void>;
  /** Scan the whole TV/anime library for duplicate shows and merge them. */
  consolidateSelectedLibraryShows: () => Promise<void>;
  /** Dry-run residual scan for scraped selection; returns candidates (may be empty). */
  scanResidualsForSelected: () => Promise<ResidualCandidate[]>;
  cleanupResiduals: (paths: string[]) => Promise<void>;
  /** SCAN-15: refresh selected items from disk (missing primary → delete). */
  refreshSelectedItemsFromDisk: () => Promise<void>;
  /** MAINT-07: reveal item in OS file manager. */
  revealSelectedItem: () => Promise<void>;
  deleteSelectedItems: (alsoTrash: boolean) => Promise<void>;
  refreshTasks: () => Promise<void>;
  upsertTask: (task: TaskSnapshot) => void;
  visibleMediaItems: () => MediaItem[];
};

export type ResidualCandidate = {
  path: string;
  itemId: string;
  itemTitle: string;
  reason: string;
  size: number;
};

let toastTimer: ReturnType<typeof setTimeout> | null = null;

let libraryRequest = 0;
let detailRequest = 0;

type MediaPage = {
  items: MediaItem[];
  metadata: MediaMetaSummary[];
  showStats: ShowListStats[];
  nextOffset?: number | null;
};

/** Trailing debounce that merges a task's completion with its library-updated event. */
const RELOAD_DEBOUNCE_MS = 150;
type ReloadRequest = {
  libraryId: string;
  posters: "changed" | "all";
  waiters: Array<() => void>;
};
let pendingReload: ReloadRequest | null = null;
let reloadTimer: ReturnType<typeof setTimeout> | null = null;
let reloadRunning: Promise<void> | null = null;

function scheduleReload(
  run: (req: ReloadRequest) => Promise<void>,
  libraryId: string,
  posters: "changed" | "all",
): Promise<void> {
  if (pendingReload && pendingReload.libraryId !== libraryId) {
    pendingReload.waiters.forEach((resolve) => resolve());
    pendingReload = null;
  }
  const req: ReloadRequest = pendingReload ?? { libraryId, posters, waiters: [] };
  if (posters === "all") req.posters = "all";
  pendingReload = req;
  const done = new Promise<void>((resolve) => req.waiters.push(resolve));
  if (reloadTimer) clearTimeout(reloadTimer);
  reloadTimer = setTimeout(() => {
    reloadTimer = null;
    void flushReload(run);
  }, RELOAD_DEBOUNCE_MS);
  return done;
}

async function flushReload(run: (req: ReloadRequest) => Promise<void>) {
  // A request made mid-flight may postdate the pages already fetched: run again.
  while (reloadRunning) await reloadRunning;
  const req = pendingReload;
  if (!req || reloadTimer) return; // a newer timer owns this request
  pendingReload = null;
  reloadRunning = run(req)
    .catch(() => {})
    .finally(() => {
      reloadRunning = null;
      req.waiters.forEach((resolve) => resolve());
    });
  await reloadRunning;
}

function metaFingerprint(meta: MediaMetaSummary | undefined): string {
  return meta ? JSON.stringify(meta) : "";
}

export const useAppStore = create<AppStore>((set, get) => ({
  status: null,
  libraries: [],
  selectedLibraryId: null,
  mediaItems: [],
  metadataById: {},
  showStatsById: {},
  selectedMediaId: null,
  selectedMediaIds: [],
  detail: null,
  detailLoading: false,
  posterUrl: null,
  searchQuery: "",
  sortOption: "nameAscending",
  statusFilter: "all",
  listViewMode: "poster",
  tasks: [],
  loading: false,
  error: null,
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
    const request = ++libraryRequest;
    ++detailRequest;
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
        const page: { items: MediaItem[]; metadata: MediaMetaSummary[]; showStats: ShowListStats[]; nextOffset?: number | null } = await invoke("list_media_page", { libraryId: id, offset, limit: 256 });
        if (request !== libraryRequest || get().selectedLibraryId !== id) return;
        for (const item of page.items) items.set(item.id, item);
        for (const meta of page.metadata) metadataById[meta.mediaItemId] = meta;
        for (const stats of page.showStats ?? []) showStatsById[stats.mediaItemId] = stats;
        set({ mediaItems: [...items.values()], metadataById: {...metadataById}, showStatsById: {...showStatsById} });
        const next: number | null = page.nextOffset ?? null;
        if (next !== null && next <= offset) throw new Error("invalid media page cursor");
        offset = next;
      }
    } catch (err) {
      if (request !== libraryRequest || get().selectedLibraryId !== id) return;
      const message = String(err);
      set({ error: message });
      get().showToast(message);
    }
  },

  reloadLibraryItems: (libraryId, opts) =>
    scheduleReload(
      async ({ libraryId: id, posters }) => {
        if (get().selectedLibraryId !== id) return;
        const request = ++libraryRequest;
        const stale = () => request !== libraryRequest || get().selectedLibraryId !== id;
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

  selectMedia: async (id) => {
    const request = ++detailRequest;
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
              if (request !== detailRequest || get().selectedMediaId !== id) return;
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
      if (request !== detailRequest || get().selectedMediaId !== id) return;
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
          if (request !== detailRequest || get().selectedMediaId !== id) return;
          set({ posterUrl: url });
        }
      }
    } catch (err) {
      if (request !== detailRequest || get().selectedMediaId !== id) return;
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

  setSearchQuery: (q) => set({ searchQuery: q }),
  setSortOption: (o) => set({ sortOption: o }),
  setStatusFilter: (f) => set({ statusFilter: f }),
  setListViewMode: (mode) => set({ listViewMode: mode }),

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
      } else if (toastTimer) {
        clearTimeout(toastTimer);
        toastTimer = null;
        set({ toastMessage: null });
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

  refreshTasks: async () => {
    try {
      const tasks = await invoke<TaskSnapshot[]>("list_tasks");
      set({ tasks });
    } catch (err) {
      const message = localizeUserMessage(String(err));
      set({ error: message });
      get().showToast(message);
    }
  },

  upsertTask: (task) => {
    const prev = get().tasks.find((t) => t.id === task.id);
    if (
      prev &&
      prev.status === task.status &&
      prev.errorMessage === task.errorMessage &&
      JSON.stringify(prev.result) === JSON.stringify(task.result) &&
      prev.progress?.completed === task.progress?.completed &&
      prev.progress?.total === task.progress?.total &&
      prev.progress?.current === task.progress?.current &&
      prev.progress?.stageKey === task.progress?.stageKey
    ) {
      return;
    }
    set((state) => {
      const rest = state.tasks.filter((t) => t.id !== task.id);
      return { tasks: [task, ...rest] };
    });
    if (prev?.status === task.status) return;
    if (task.status === "completed") {
      if (task.kind === "refresh") {
        const detail = task.progress?.current?.trim();
        if (task.progress?.stageKey === "refreshItems") {
          const title = detail
            ? tt("toast.itemsRefreshDoneDetail", { detail })
            : tt("toast.itemsRefreshDone");
          get().showToast(title, 2800);
          void notifyTaskDone(tt("toast.itemsRefreshDone"), detail);
          const libraryId = get().selectedLibraryId;
          if (libraryId) void get().reloadLibraryItems(libraryId);
        } else {
          const added =
            task.progress?.stageKey === "saveResults"
              ? Number(task.progress?.completed ?? 0)
              : 0;
          const title =
            added > 0
              ? tt("toast.refreshAdded", { n: added })
              : detail
                ? tt("toast.refreshDoneDetail", { detail })
                : tt("toast.refreshDone");
          get().showToast(title, 3200);
          void notifyTaskDone(tt("toast.refreshDone"), detail || title);
          const targetId = task.targetId;
          if (targetId && get().selectedLibraryId === targetId) {
            if (added > 0) {
              set({
                sortOption: "unscrapedFirst",
                statusFilter: "all",
                searchQuery: "",
              });
            }
            void get().reloadLibraryItems(targetId);
          }
        }
      } else if (
        task.kind === "batchScrape" ||
        task.kind === "scrape" ||
        task.kind === "rescrape" ||
        task.kind === "manualMatch"
      ) {
        const raw = task.progress?.current?.trim() ?? "";
        const summary = raw ? localizeScrapeSummary(raw) : "";
        const label =
          task.kind === "rescrape"
            ? tt("toast.rescrapeDone")
            : task.kind === "manualMatch"
              ? tt("toast.manualMatchDone")
              : tt("toast.scrapeDone");
        if (summary && task.kind !== "manualMatch") {
          const unmatched = scrapeUnmatchedCount(raw || summary);
          const hint = unmatched > 0 ? tt("toast.scrapeUnmatchedHint") : "";
          get().showToast(`${label} · ${summary}${hint}`);
          void notifyTaskDone(label, summary);
        } else {
          get().showToast(label);
          void notifyTaskDone(label);
        }
        const libraryId = get().selectedLibraryId;
        if (libraryId) void get().reloadLibraryItems(libraryId, { posters: "all" });
      } else if (task.kind === "rename") {
        const detail = task.progress?.current?.trim();
        get().showToast(
          detail ? tt("toast.renameDoneDetail", { detail }) : tt("toast.renameDone"),
        );
        void notifyTaskDone(tt("toast.renameDone"), detail);
        const libraryId = get().selectedLibraryId;
        if (libraryId) void get().reloadLibraryItems(libraryId);
      } else if (task.kind === "organize") {
        const detail = task.progress?.current?.trim();
        get().showToast(
          detail
            ? tt("toast.organizeDoneDetail", { detail })
            : tt("toast.organizeDone"),
        );
        void notifyTaskDone(tt("toast.organizeDone"), detail);
        const libraryId = get().selectedLibraryId;
        if (libraryId) void get().reloadLibraryItems(libraryId);
      } else if (task.kind === "cleanup") {
        const detail = task.progress?.current?.trim();
        get().showToast(
          detail ? tt("toast.cleanupDoneDetail", { detail }) : tt("toast.cleanupDone"),
        );
        void notifyTaskDone(tt("toast.cleanupDone"), detail);
      }
    } else if (task.status === "failed") {
      const err = localizeUserMessage(task.errorMessage || tt("toast.taskFailed"));
      get().showToast(err);
      void notifyTaskDone(tt("toast.taskFailed"), err);
    }
  },
}));

/** Re-fetch the open detail after a background reload without a loading flash. */
async function refreshOpenDetail(
  get: () => AppStore,
  set: (partial: Partial<AppStore>) => void,
  id: string,
) {
  const request = ++detailRequest;
  const current = () => request === detailRequest && get().selectedMediaId === id;
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

async function ensureScrapeKeys(
  get: () => AppStore,
): Promise<boolean> {
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
